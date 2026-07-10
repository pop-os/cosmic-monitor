use nvml_wrapper::{
    Nvml, enum_wrappers::device::TemperatureSensor, enums::device::UsedGpuMemory, error::NvmlError,
};
use std::{collections::HashMap, fs, time::Instant};
use sysinfo::{Components, Pid};

use crate::info::{GpuId, GpuItem, GpuState, Platform};

pub struct NvmlPlatform {
    gpu_items: Vec<GpuItem>,
    last_seen_timestamps: HashMap<GpuId, u64>,
    nvml: Option<Nvml>,
    processes: HashMap<Pid, HashMap<GpuId, (f32, u64)>>,
}

impl NvmlPlatform {
    pub fn new() -> Self {
        Self {
            gpu_items: Vec::new(),
            last_seen_timestamps: HashMap::new(),
            //TODO: log error?
            nvml: Nvml::init().ok(),
            processes: HashMap::new(),
        }
    }

    fn refresh_inner(&mut self, refresh_processes: bool) -> Result<(), NvmlError> {
        let Some(nvml) = &self.nvml else {
            self.gpu_items.clear();
            self.processes.clear();
            return Ok(());
        };

        // Device count can be read without waking GPUs
        let device_count = nvml.device_count()?;
        self.gpu_items.truncate(device_count as usize);
        for index in 0..device_count {
            // Reading the name and PCI info will not wake the GPU
            let device = nvml.device_by_index(index)?;
            let name = device.name()?;
            let pci_info = device.pci_info()?;
            //TODO: would this ever be non-zero?
            let pci_func = 0;
            let gpu_id = GpuId::Pci {
                domain: pci_info.domain,
                bus: pci_info.bus,
                device: pci_info.device,
                func: pci_func,
            };

            if self.gpu_items.len() <= index as usize {
                self.gpu_items.push(GpuItem {
                    boot_vga: false,
                    id: gpu_id,
                    name: String::new(),
                    state: GpuState::Active,
                    power: None,
                    temp: None,
                    usage: None,
                    vram_used: None,
                    vram_total: None,
                });
            }

            let gpu = &mut self.gpu_items[index as usize];
            gpu.name = name;
            gpu.id = gpu_id;

            // Check GPU runtime status, only supported on Linux
            #[cfg(target_os = "linux")]
            {
                let runtime_status_path = format!(
                    "/sys/bus/pci/devices/{:04x}:{:02x}:{:02x}.{:01x}/power/runtime_status",
                    pci_info.domain, pci_info.bus, pci_info.device, pci_func,
                );
                match fs::read_to_string(&runtime_status_path) {
                    Ok(data) => {
                        if matches!(data.trim(), "suspended" | "suspending") {
                            gpu.state = GpuState::Suspended;
                        } else if matches!(gpu.state, GpuState::Suspended) {
                            gpu.state = GpuState::Active;
                        }
                    }
                    Err(err) => {
                        log::debug!("failed to read {}: {}", runtime_status_path, err);
                    }
                }
            }

            match gpu.state {
                GpuState::Active => {}
                GpuState::Idle(instant) => {
                    //TODO: determine best idle timeout
                    if instant.elapsed().as_secs() >= 5 && refresh_processes {
                        gpu.state = GpuState::Active;
                    } else {
                        continue;
                    }
                }
                GpuState::Suspended => {
                    // Clear GPU values when suspended
                    gpu.power = None;
                    gpu.temp = None;
                    gpu.usage = Some(0.0);
                    gpu.vram_used = Some(0);
                    continue;
                }
            }

            // Reading the values below will wake the GPU
            let power = (device.power_usage()? as f32) / 1000.0;
            gpu.power = Some(power as f32);

            let temp = device.temperature(TemperatureSensor::Gpu)?;
            gpu.temp = Some(temp as f32);

            let util = device.utilization_rates()?;
            gpu.usage = Some(util.gpu as f32);

            let memory_info = device.memory_info()?;
            gpu.vram_used = Some(memory_info.used);
            gpu.vram_total = Some(memory_info.total);

            if refresh_processes {
                self.processes.retain(|_pid, gpu_usages| {
                    gpu_usages.remove(&gpu_id);
                    !gpu_usages.is_empty()
                });

                let mut last_seen_timestamp = self.last_seen_timestamps.get(&gpu_id).copied();
                match device.process_utilization_stats(last_seen_timestamp) {
                    Ok(samples) => {
                        if samples.is_empty() {
                            gpu.state = GpuState::Idle(Instant::now());
                        }

                        for sample in samples {
                            let pid = Pid::from_u32(sample.pid);
                            //TODO: use more sample information?
                            self.processes
                                .entry(pid)
                                .or_insert_with(|| HashMap::new())
                                .entry(gpu_id)
                                .or_insert((0.0, 0))
                                .0 += sample.sm_util as f32;
                            last_seen_timestamp = Some(
                                last_seen_timestamp
                                    .map_or(sample.timestamp, |x| x.max(sample.timestamp)),
                            );
                        }
                    }
                    Err(_err) => {
                        gpu.state = GpuState::Idle(Instant::now());
                    }
                }
                if let Some(timestamp) = last_seen_timestamp {
                    self.last_seen_timestamps.insert(gpu_id, timestamp);
                }

                for processes_res in &[
                    device.running_graphics_processes(),
                    device.running_compute_processes(),
                ] {
                    let Ok(processes) = processes_res else {
                        continue;
                    };
                    for process in processes {
                        if let UsedGpuMemory::Used(vram) = process.used_gpu_memory {
                            let pid = Pid::from_u32(process.pid);
                            let entry = self
                                .processes
                                .entry(pid)
                                .or_insert_with(|| HashMap::new())
                                .entry(gpu_id)
                                .or_insert((0.0, 0));
                            entry.1 = entry.1.max(vram);
                        }
                    }
                }
            }
        }

        Ok(())
    }
}

impl Platform for NvmlPlatform {
    fn refresh(&mut self, processes: bool, _components: &Components) {
        //TODO: log error?
        match self.refresh_inner(processes) {
            Ok(()) => {}
            Err(err) => {
                log::warn!("failed to refresh NVML: {}", err);
            }
        }
    }

    fn gpus(&self) -> Vec<GpuItem> {
        self.gpu_items.clone()
    }

    fn process_gpu_usage(&self, pid: Pid) -> HashMap<GpuId, (f32, u64)> {
        if let Some(usages) = self.processes.get(&pid) {
            //TODO: use more sample information?
            usages.clone()
        } else {
            HashMap::new()
        }
    }
}
