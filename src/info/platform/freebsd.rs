//! FreeBSD [`Platform`] implementation.

use std::{collections::HashMap, sync::Arc, time::Duration};

use sysinfo::{Components, Disk, Pid, Process, System};

use crate::info::{AppEntry, DiskItem, GpuId, GpuItem, GpuState, Platform};

pub struct FreeBSDPlatform {
    pub app_entries: Vec<Arc<AppEntry>>,
}

impl FreeBSDPlatform {
    pub fn new() -> Self {
        Self {
            app_entries: AppEntry::get_entries(),
        }
    }
}

impl Platform for FreeBSDPlatform {
    fn refresh(&mut self, _refresh_processes: bool, _components: &Components) {}

    fn disk_item(&self, disk: &Disk, refresh: Duration, _components: &Components) -> DiskItem {
        DiskItem::new(disk, refresh)
    }

    fn gpus(&self) -> Vec<GpuItem> {
        Vec::new()
    }

    fn process_app(&self, process: &Process, sys: &System) -> Option<Arc<AppEntry>> {
        AppEntry::process_app(&self.app_entries, process, sys)
    }

    fn process_gpu_usage(&self, _pid: Pid) -> HashMap<GpuId, (f32, u64)> {
        HashMap::new()
    }
}
