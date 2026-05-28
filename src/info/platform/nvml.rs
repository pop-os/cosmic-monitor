use nvml_wrapper::{Nvml, error::NvmlError, struct_wrappers::device::ProcessUtilizationSample};
use std::collections::HashMap;
use sysinfo::Pid;

use super::Platform;

pub struct NvmlPlatform {
    nvml: Option<Nvml>,
    processes: HashMap<Pid, HashMap<u32, ProcessUtilizationSample>>,
}

impl NvmlPlatform {
    pub fn new() -> Self {
        Self {
            //TODO: only use NVML if GPU is awake
            //TODO: log error?
            nvml: Nvml::init().ok(),
            processes: HashMap::new(),
        }
    }

    fn refresh_processes_inner(&mut self) -> Result<(), NvmlError> {
        let Some(nvml) = &self.nvml else {
            return Ok(());
        };
        self.processes.clear();
        for index in 0..nvml.device_count()? {
            let device = nvml.device_by_index(index)?;
            //TODO: last_seen_timestamp
            for sample in device.process_utilization_stats(None)? {
                let pid = Pid::from_u32(sample.pid);
                self.processes
                    .entry(pid)
                    .or_insert_with(|| HashMap::new())
                    .insert(index, sample);
            }
        }
        Ok(())
    }
}

impl Platform for NvmlPlatform {
    fn refresh_processes(&mut self) {
        //TODO: log error?
        let _ = self.refresh_processes_inner();
    }

    fn process_gpu_usage(&self, pid: Pid) -> Option<f32> {
        let samples = self.processes.get(&pid)?;
        //TODO: use more sample information, show each GPU independently
        Some(
            samples
                .iter()
                .fold(0.0, |total, (_index, sample)| total + sample.sm_util as f32),
        )
    }
}
