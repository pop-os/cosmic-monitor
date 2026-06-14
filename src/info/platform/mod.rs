use std::{collections::HashMap, sync::Arc, time::Duration};
use sysinfo::{Components, Disk, Pid, Process, System};

use super::{AppEntry, DiskItem, GpuId, GpuItem};

#[cfg(target_os = "linux")]
mod linux;

#[cfg(feature = "nvml")]
mod nvml;

pub trait Platform: Send + Sync {
    fn refresh(&mut self, _refresh_processes: bool, _components: &Components) {}

    fn disk_item(&self, disk: &Disk, refresh: Duration, _components: &Components) -> DiskItem {
        DiskItem::new(disk, refresh)
    }

    fn gpus(&self) -> Vec<GpuItem> {
        Vec::new()
    }

    fn process_app(&self, _process: &Process, _sys: &System) -> Option<Arc<AppEntry>> {
        None
    }

    fn process_gpu_usage(&self, _pid: Pid) -> HashMap<GpuId, (f32, u64)> {
        HashMap::new()
    }
}

pub struct FallbackPlatform;

impl Platform for FallbackPlatform {}

#[allow(unreachable_code)]
pub fn default_platform() -> Box<dyn Platform> {
    #[cfg(target_os = "linux")]
    return Box::new(linux::LinuxPlatform::new());

    #[cfg(feature = "nvml")]
    return Box::new(nvml::NvmlPlatform::new());

    #[allow(unreachable_code)]
    Box::new(FallbackPlatform)
}
