use nvml_wrapper::{
    Nvml, enum_wrappers::device::TemperatureSensor, enums::device::UsedGpuMemory, error::NvmlError,
};
use std::collections::HashMap;
use sysinfo::{Components, Pid};

use super::{GpuId, GpuItem, Platform};

pub struct NvmlPlatform {
    gpu_items: Vec<GpuItem>,
    last_seen_timestamp: Option<u64>,
    nvml: Option<Nvml>,
    processes: HashMap<Pid, HashMap<GpuId, (f32, u64)>>,
}

impl NvmlPlatform {
    pub fn new() -> Self {
        Self {
            gpu_items: Vec::new(),
            last_seen_timestamp: None,
            //TODO: only use NVML if GPU is awake
            //TODO: log error?
            nvml: Nvml::init().ok(),
            processes: HashMap::new(),
        }
    }

    fn refresh_inner(&mut self, refresh_processes: bool) -> Result<(), NvmlError> {
        let Some(nvml) = &self.nvml else {
            return Ok(());
        };

        self.gpu_items.clear();

        if refresh_processes {
            self.processes.clear();
        }

        for index in 0..nvml.device_count()? {
            let device = nvml.device_by_index(index)?;
            let name = device.name()?;
            let memory_info = device.memory_info()?;
            let pci_info = device.pci_info()?;
            let gpu_id = GpuId::Pci {
                domain: pci_info.domain,
                bus: pci_info.bus,
                device: pci_info.device,
                //TODO: would this ever be non-zero?
                func: 0,
            };
            let temp = device.temperature(TemperatureSensor::Gpu)?;
            let util = device.utilization_rates()?;

            self.gpu_items.push(GpuItem {
                boot_vga: false,
                id: gpu_id,
                name,
                temp: Some(temp as f32),
                usage: Some(util.gpu as f32),
                vram_used: Some(memory_info.used),
                vram_total: Some(memory_info.total),
            });

            if refresh_processes {
                for sample in device.process_utilization_stats(self.last_seen_timestamp)? {
                    let pid = Pid::from_u32(sample.pid);
                    //TODO: use more sample information?
                    self.processes
                        .entry(pid)
                        .or_insert_with(|| HashMap::new())
                        .entry(gpu_id)
                        .or_insert((0.0, 0))
                        .0 += sample.sm_util as f32;
                    self.last_seen_timestamp = Some(
                        self.last_seen_timestamp
                            .map_or(sample.timestamp, |x| x.max(sample.timestamp)),
                    );
                }
                for process in device.running_graphics_processes()? {
                    if let UsedGpuMemory::Used(vram) = process.used_gpu_memory {
                        let pid = Pid::from_u32(process.pid);
                        self.processes
                            .entry(pid)
                            .or_insert_with(|| HashMap::new())
                            .entry(gpu_id)
                            .or_insert((0.0, 0))
                            .1 += vram;
                    }
                }
                //TODO: device.running_compute_processes() without double counting
            }
        }

        Ok(())
    }
}

impl Platform for NvmlPlatform {
    fn refresh(&mut self, processes: bool, _components: &Components) {
        //TODO: log error?
        let _ = self.refresh_inner(processes);
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
