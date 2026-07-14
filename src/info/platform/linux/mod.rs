use freedesktop_desktop_entry::{
    DesktopEntry, Iter, default_paths, get_languages_from_env, group_entry_from_path,
};
use libc::c_uint;
use std::{
    cmp::Ordering,
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};
use sysinfo::{Components, Disk, Pid, Process, System};

use crate::info::{AppEntry, DiskItem, GpuId, GpuItem, GpuState, Platform};

use fdinfo::FdInfo;
mod fdinfo;

// From distinst util crate, license LGPL 3
fn resolve_slave(name: &str) -> Option<PathBuf> {
    let slaves_dir = PathBuf::from(["/sys/class/block/", name, "/slaves/"].concat());
    if !slaves_dir.exists() {
        return Some(PathBuf::from(["/dev/", name].concat()));
    }

    let mut slaves = Vec::new();

    for entry in slaves_dir.read_dir().ok()? {
        if let Ok(entry) = entry {
            if let Ok(name) = entry.file_name().into_string() {
                slaves.push(name);
            }
        }
    }

    if slaves.len() == 1 {
        return Some(PathBuf::from(["/dev/", &slaves[0]].concat()));
    }

    None
}

// From distinst util crate, license LGPL 3
fn resolve_to_physical(name: &str) -> Option<PathBuf> {
    let mut physical: Option<PathBuf> = None;

    loop {
        let physical_c = physical.clone();
        let name = physical_c.as_ref().map_or(name, |physical| {
            physical.file_name().unwrap().to_str().unwrap()
        });
        if let Some(slave) = resolve_slave(name) {
            if physical.as_ref().map_or(true, |rec| rec != &slave) {
                physical = Some(slave);
                continue;
            }
        }
        break;
    }

    physical
}

struct LinuxProcess {
    fdinfos: HashMap<(c_uint, c_uint), FdInfo>,
    gpu_usages: HashMap<GpuId, (f32, u64)>,
    pid: Pid,
    proc_path: PathBuf,
    time: Instant,
    version: u64,
}

impl LinuxProcess {
    fn new(pid: Pid, proc_path: PathBuf) -> Self {
        Self {
            fdinfos: HashMap::new(),
            gpu_usages: HashMap::new(),
            pid,
            proc_path,
            time: Instant::now(),
            version: 0,
        }
    }

    fn update(&mut self, version: u64, nvml: &Box<dyn Platform>) {
        let time = Instant::now();
        let mut fdinfos = FdInfo::for_proc_path(&self.proc_path);
        let duration = time.saturating_duration_since(self.time).as_secs_f32();

        // Add DRM fdinfo GPU usage to NVML GPU usage
        self.gpu_usages = nvml.process_gpu_usage(self.pid);
        for (id, fdinfo) in fdinfos.iter_mut() {
            if let Some(last_fdinfo) = self.fdinfos.get(id) {
                for (name, nanos, usage) in fdinfo.engines.iter_mut() {
                    for (last_name, last_nanos, _) in last_fdinfo.engines.iter() {
                        if last_name == name {
                            *usage = 100.0
                                * Duration::from_nanos(nanos.saturating_sub(*last_nanos))
                                    .as_secs_f32()
                                / duration;
                            //TODO: filter by engine name
                            self.gpu_usages.entry(fdinfo.gpu_id).or_insert((0.0, 0)).0 += *usage;
                        }
                    }
                }
            }
            for (name, vram) in fdinfo.residents.iter_mut() {
                //TODO: figure out what each GPU driver uses for this name
                match name.as_str() {
                    "vram" => {
                        self.gpu_usages.entry(fdinfo.gpu_id).or_insert((0.0, 0)).1 += *vram;
                    }
                    _ => {}
                }
            }
        }

        self.fdinfos = fdinfos;
        self.time = time;
        self.version = version;
    }
}

pub struct LinuxPlatform {
    amdgpu_ids: HashMap<(u16, u8), String>,
    app_entries: Vec<Arc<AppEntry>>,
    gpu_energies: HashMap<GpuId, (Instant, u64)>,
    gpu_items: Vec<GpuItem>,
    nvml: Box<dyn Platform>,
    processes: HashMap<Pid, LinuxProcess>,
    version: u64,
}

impl LinuxPlatform {
    pub fn new() -> Self {
        let mut amdgpu_ids = HashMap::new();
        if let Ok(data) = fs::read_to_string("/usr/share/libdrm/amdgpu.ids") {
            for line in data.lines() {
                if line.starts_with("#") {
                    continue;
                }
                let mut parts = line.splitn(3, ",\t");
                let Some(id_str) = parts.next() else { continue };
                let Some(rev_str) = parts.next() else {
                    continue;
                };
                let Some(name) = parts.next() else { continue };
                let Ok(id) = u16::from_str_radix(id_str, 16) else {
                    continue;
                };
                let Ok(rev) = u8::from_str_radix(rev_str, 16) else {
                    continue;
                };
                amdgpu_ids.insert((id, rev), name.to_string());
            }
        }

        //TODO: use this on all Unix-like systems
        //TODO: refresh on changes
        let locales = get_languages_from_env();
        let mut app_entries = Vec::new();
        for app in Iter::new(default_paths())
            .filter_map(|p| DesktopEntry::from_path(p, Some(&locales)).ok())
        {
            let Ok(args) = app.parse_exec() else { continue };
            let id = app.id().to_string();
            let mut icon = app.icon().map(|x| x.to_string());

            // Fixup for firefox user app icon
            if icon.is_none() && id.starts_with("userapp-Firefox-") {
                icon = Some("firefox".to_string());
            }

            app_entries.push(Arc::new(AppEntry {
                id,
                icon,
                name: app.full_name(&locales).map(|x| x.to_string()),
                no_display: app.no_display(),
                args,
            }));
        }

        // Sort no_display below
        app_entries.sort_by(|a, b| match (a.no_display, b.no_display) {
            (false, true) => Ordering::Less,
            (true, false) => Ordering::Greater,
            _ => a.name.cmp(&b.name),
        });

        Self {
            amdgpu_ids,
            app_entries,
            gpu_energies: HashMap::new(),
            gpu_items: Vec::new(),
            #[cfg(feature = "nvml")]
            nvml: Box::new(super::nvml::NvmlPlatform::new()),
            #[cfg(not(feature = "nvml"))]
            nvml: Box::new(super::FallbackPlatform),
            processes: HashMap::new(),
            version: 0,
        }
    }

    fn update_disk(&self, disk: &Disk, item: &mut DiskItem, components: &Components) -> Option<()> {
        let orig_dev_path = disk.name();
        let virt_dev_path = fs::canonicalize(&orig_dev_path).ok()?;
        let virt_dev_name = virt_dev_path.strip_prefix("/dev/").ok()?.to_string_lossy();
        let dev_path = resolve_to_physical(&virt_dev_name).unwrap_or(virt_dev_path);
        let dev_name = dev_path.strip_prefix("/dev/").ok()?;
        let sys_class_path = Path::new("/sys/class/block").join(&dev_name);
        let mut sys_path = fs::canonicalize(&sys_class_path).ok()?;
        // Partitions will be nested inside disk, which is inside device, which is inside subsystem
        // /sys/devices/.../nvme/nvme0/nvme0n1/nvme0n1p1
        for _depth in 0..3 {
            let model_path = sys_path.join("model");
            let Ok(model_data) = fs::read_to_string(&model_path) else {
                sys_path = sys_path.parent()?.to_path_buf();
                continue;
            };
            let model = model_data.trim();
            item.name = if orig_dev_path != dev_path {
                format!(
                    "{} ({} on {})",
                    model,
                    orig_dev_path.display(),
                    dev_path.display()
                )
            } else {
                format!("{} ({})", model, dev_path.display())
            };

            // Look for hwmon temperature
            for entry_res in fs::read_dir(&sys_path).ok()? {
                let Ok(entry) = entry_res else { continue };
                let file_name = entry.file_name();
                let Some(file_name) = file_name.to_str() else {
                    continue;
                };
                if file_name.starts_with("hwmon") {
                    for component in components {
                        let Some(id) = component.id() else { continue };
                        let Some((hwmon, _index)) = id.split_once('_') else {
                            continue;
                        };
                        if hwmon != file_name {
                            continue;
                        }
                        let Some(temp) = component.temperature() else {
                            continue;
                        };
                        item.temp = Some(item.temp.map_or(temp, |x| temp.max(x)));
                    }
                }
            }

            return Some(());
        }
        None
    }
}

impl Platform for LinuxPlatform {
    fn refresh(&mut self, refresh_processes: bool, components: &Components) {
        self.nvml.refresh(refresh_processes, components);

        // Refreshed first so total Intel GPU metrics can be calculated
        if refresh_processes {
            self.version += 1;
            if let Ok(entries) = fs::read_dir("/proc") {
                for entry_res in entries {
                    let Ok(entry) = entry_res else { continue };
                    let file_name = entry.file_name();
                    let Some(pid_str) = file_name.to_str() else {
                        continue;
                    };
                    let Ok(pid) = pid_str.parse::<Pid>() else {
                        continue;
                    };
                    self.processes
                        .entry(pid)
                        .or_insert_with(|| LinuxProcess::new(pid, entry.path()))
                        .update(self.version, &self.nvml)
                }
            }
            self.processes.retain(|_k, v| v.version == self.version);
        }

        self.gpu_items.clear();
        if let Ok(entries) = fs::read_dir("/sys/class/drm") {
            for entry_res in entries {
                let Ok(entry) = entry_res else { continue };
                let file_name = entry.file_name();
                let Some(name_str) = file_name.to_str() else {
                    continue;
                };
                let Some(id_str) = name_str.strip_prefix("card") else {
                    continue;
                };
                let Ok(id) = id_str.parse::<c_uint>() else {
                    continue;
                };
                let drm_path = entry.path();
                let device_path = drm_path.join("device");

                let mut id_opt = None;
                if let Ok(link_path) = fs::read_link(&device_path) {
                    if let Some(link_name) = link_path.file_name() {
                        if let Some(link_str) = link_name.to_str() {
                            id_opt = GpuId::parse_pci(link_str);
                        }
                    }
                }

                let name_from_pci_ids = || -> Result<String, Box<dyn std::error::Error>> {
                    let vendor_str = fs::read_to_string(device_path.join("vendor"))?;
                    let vendor_id =
                        u16::from_str_radix(vendor_str.trim().trim_start_matches("0x"), 16)?;
                    let device_str = fs::read_to_string(device_path.join("device"))?;
                    let device_id =
                        u16::from_str_radix(device_str.trim().trim_start_matches("0x"), 16)?;
                    if vendor_id == 0x1002 {
                        let rev_str = fs::read_to_string(device_path.join("revision"))?;
                        let rev = u8::from_str_radix(rev_str.trim().trim_start_matches("0x"), 16)?;
                        if let Some(name) = self.amdgpu_ids.get(&(device_id, rev)) {
                            return Ok(name.to_string());
                        }
                    }
                    if let Some(entry) = pci_ids::Device::from_vid_pid(vendor_id, device_id) {
                        Ok(format!(
                            "{} {}",
                            match vendor_id {
                                0x1002 | 0x1022 => "AMD",
                                0x10DE => "NVIDIA",
                                0x8086 => "Intel",
                                _ => entry.vendor().name(),
                            },
                            entry.name()
                        ))
                    } else {
                        Err(format!("no entry for {:04x}:{:04x}", vendor_id, device_id).into())
                    }
                };

                //TODO: only update name when GPUs change
                let name = match name_from_pci_ids() {
                    Ok(ok) => ok,
                    Err(err) => {
                        log::warn!("failed to get name from PCI IDs: {}", err);
                        format!("Unknown GPU {}", id)
                    }
                };

                let mut gpu_item = GpuItem {
                    boot_vga: false,
                    id: id_opt.unwrap_or(GpuId::Other(id)),
                    name,
                    state: GpuState::Active,
                    power: None,
                    temp: None,
                    usage: None,
                    vram_used: None,
                    vram_total: None,
                };

                if let Ok(data) = fs::read_to_string(device_path.join("boot_vga")) {
                    gpu_item.boot_vga = data.trim() == "1";
                };

                //TODO: log errors
                //TODO: gpu_busy_percent is only available on AMD
                if let Ok(data) = fs::read_to_string(device_path.join("gpu_busy_percent")) {
                    gpu_item.usage = data.trim().parse().ok();
                };
                //TODO: mem_info_vram_used is only available on AMD
                if let Ok(data) = fs::read_to_string(device_path.join("mem_info_vram_used")) {
                    gpu_item.vram_used = data.trim().parse().ok();
                };
                //TODO: mem_info_vram_total is only available on AMD
                if let Ok(data) = fs::read_to_string(device_path.join("mem_info_vram_total")) {
                    gpu_item.vram_total = data.trim().parse().ok();
                } else {
                    // Try to find largest prefetchable memory BAR and assume that is VRAM
                    if let Ok(data) = fs::read_to_string(device_path.join("resource")) {
                        for line in data.lines() {
                            let mut parts = line.split(" ");
                            let parse_hex = |string: &str| -> Option<u64> {
                                u64::from_str_radix(string.trim_start_matches("0x"), 16).ok()
                            };
                            let Some(start) = parts.next().and_then(parse_hex) else {
                                continue;
                            };
                            let Some(end) = parts.next().and_then(parse_hex) else {
                                continue;
                            };
                            let Some(flags) = parts.next().and_then(parse_hex) else {
                                continue;
                            };

                            const IORESOURCE_MEM: u64 = 0x00000200;
                            const IORESOURCE_PREFETCH: u64 = 0x00002000;
                            if (flags & (IORESOURCE_MEM | IORESOURCE_PREFETCH))
                                == (IORESOURCE_MEM | IORESOURCE_PREFETCH)
                            {
                                let len = (end + 1) - start;
                                gpu_item.vram_total =
                                    Some(gpu_item.vram_total.unwrap_or(0).max(len));
                            }
                        }
                    }
                };

                if let Ok(entries) = fs::read_dir(device_path.join("hwmon")) {
                    for entry_res in entries {
                        let Ok(entry) = entry_res else { continue };

                        // Check for power info
                        if let Ok(data) = fs::read_to_string(entry.path().join("energy1_input")) {
                            // Intel GPUs provide energy1_input
                            if let Ok(microjoules) = data.trim().parse::<u64>() {
                                let time = Instant::now();
                                if let Some((last_time, last_microjoules)) =
                                    self.gpu_energies.insert(gpu_item.id, (time, microjoules))
                                {
                                    if let Some(duration) = time.checked_duration_since(last_time) {
                                        let microwatts = (microjoules.wrapping_sub(last_microjoules)
                                            as f32)
                                            / duration.as_secs_f32();
                                        gpu_item.power = Some(microwatts / 1_000_000.0);
                                    }
                                }
                            }
                        } else if let Ok(data) =
                            fs::read_to_string(entry.path().join("power1_average"))
                        {
                            // AMD GPUs provide power1_average
                            if let Ok(microwatts) = data.trim().parse::<f32>() {
                                gpu_item.power = Some(microwatts / 1_000_000.0);
                            }
                        }

                        // Check for temperature from matching Component from sysinfo
                        let file_name = entry.file_name();
                        let Some(file_name) = file_name.to_str() else {
                            continue;
                        };
                        for component in components {
                            let Some(id) = component.id() else { continue };
                            let Some((hwmon, _index)) = id.split_once('_') else {
                                continue;
                            };
                            if hwmon != file_name {
                                continue;
                            }
                            if let Some(temp) = component.temperature() {
                                gpu_item.temp = Some(gpu_item.temp.map_or(temp, |x| temp.max(x)));
                            }
                        }
                    }
                }

                self.gpu_items.push(gpu_item)
            }
        }

        'nvml_gpus: for nvml_gpu in self.nvml.gpus() {
            for gpu in self.gpu_items.iter_mut() {
                if gpu.id == nvml_gpu.id {
                    // Copy fields that NVML will know better than DRM
                    gpu.name = nvml_gpu.name;
                    gpu.state = nvml_gpu.state;
                    gpu.power = nvml_gpu.power;
                    gpu.temp = nvml_gpu.temp;
                    gpu.usage = nvml_gpu.usage;
                    gpu.vram_used = nvml_gpu.vram_used;
                    gpu.vram_total = nvml_gpu.vram_total;
                    continue 'nvml_gpus;
                }
            }
            self.gpu_items.push(nvml_gpu);
        }

        // Fill in missing metrics using fdinfo totals
        for gpu_item in self.gpu_items.iter_mut() {
            let calc_usage = gpu_item.usage.is_none();
            let calc_vram = gpu_item.vram_used.is_none();
            if calc_usage || calc_vram {
                for (_pid, process) in self.processes.iter() {
                    if let Some(usage) = process.gpu_usages.get(&gpu_item.id) {
                        if calc_usage {
                            gpu_item.usage = Some(gpu_item.usage.map_or(usage.0, |x| x + usage.0));
                        }
                        if calc_vram {
                            gpu_item.vram_used =
                                Some(gpu_item.vram_used.map_or(usage.1, |x| x + usage.1));
                        }
                    }
                }
            }
        }
    }

    fn disk_item(&self, disk: &Disk, refresh: Duration, components: &Components) -> DiskItem {
        let mut item = DiskItem::new(disk, refresh);
        self.update_disk(disk, &mut item, components);
        item
    }

    fn gpus(&self) -> Vec<GpuItem> {
        self.gpu_items.clone()
    }

    fn process_app<'a>(&self, mut process: &'a Process, sys: &'a System) -> Option<Arc<AppEntry>> {
        // This loops to look for any parent processes that have an associated app, as well
        //TODO: maximum depth for parent app search?
        loop {
            let proc_args = process.cmd();

            // Handle flatpaks
            match group_entry_from_path(
                format!("/proc/{}/root/.flatpak-info", process.pid()),
                "Application",
                "name",
            ) {
                Ok(Some(name)) => {
                    for app in self.app_entries.iter() {
                        if app.id == name {
                            return Some(app.clone());
                        }
                    }
                }
                _ => {}
            }

            let proc_cmd = proc_args.get(0).and_then(|x| x.to_str())?;
            let proc_exe = process.exe().and_then(|x| x.to_str())?;
            for app in self.app_entries.iter() {
                let Some(cmd) = app.args.get(0) else { continue };
                if proc_cmd == cmd || proc_exe == cmd {
                    return Some(app.clone());
                }
            }
            let parent = process.parent()?;
            process = sys.process(parent)?;
        }
    }

    fn process_gpu_usage(&self, pid: Pid) -> HashMap<GpuId, (f32, u64)> {
        if let Some(process) = self.processes.get(&pid) {
            process.gpu_usages.clone()
        } else {
            HashMap::new()
        }
    }
}
