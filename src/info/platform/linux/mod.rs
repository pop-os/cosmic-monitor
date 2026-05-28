use libc::c_uint;
use std::{
    collections::HashMap,
    fs,
    path::PathBuf,
    time::{Duration, Instant},
};
use sysinfo::Pid;

use super::Platform;

use fdinfo::FdInfo;
mod fdinfo;

struct LinuxProcess {
    fdinfos: HashMap<(c_uint, c_uint), FdInfo>,
    gpu_usage: Option<f32>,
    pid: Pid,
    proc_path: PathBuf,
    time: Instant,
    version: u64,
}

impl LinuxProcess {
    fn new(pid: Pid, proc_path: PathBuf) -> Self {
        Self {
            fdinfos: HashMap::new(),
            gpu_usage: None,
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

        self.gpu_usage = nvml.process_gpu_usage(self.pid);
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
                            self.gpu_usage = Some(self.gpu_usage.map_or(*usage, |x| x + *usage));
                        }
                    }
                }
            }
        }

        self.fdinfos = fdinfos;
        self.time = time;
        self.version = version;
    }
}

pub struct LinuxPlatform {
    nvml: Box<dyn Platform>,
    processes: HashMap<Pid, LinuxProcess>,
    version: u64,
}

impl LinuxPlatform {
    pub fn new() -> Self {
        Self {
            #[cfg(feature = "nvml")]
            nvml: Box::new(super::nvml::NvmlPlatform::new()),
            #[cfg(not(feature = "nvml"))]
            nvml: Box::new(super::FallbackPlatform),
            processes: HashMap::new(),
            version: 0,
        }
    }
}

impl Platform for LinuxPlatform {
    fn refresh_processes(&mut self) {
        self.nvml.refresh_processes();

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
        self.processes.retain(|_k, v| v.version == self.version)
    }

    fn process_gpu_usage(&self, pid: Pid) -> Option<f32> {
        self.processes.get(&pid)?.gpu_usage
    }
}
