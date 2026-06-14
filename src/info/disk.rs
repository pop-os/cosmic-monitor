use std::time::Duration;

use sysinfo::Disk;

#[derive(Clone, Debug)]
pub struct DiskItem {
    pub mount_path: String,
    pub name: String,
    pub used: u64,
    pub total: u64,
    pub read: f64,
    pub write: f64,
    pub temp: Option<f32>,
}

impl DiskItem {
    pub fn new(disk: &Disk, refresh: Duration) -> Self {
        let usage = disk.usage();
        Self {
            mount_path: disk.mount_point().to_string_lossy().into(),
            name: disk.name().to_string_lossy().into(),
            used: disk.total_space() - disk.available_space(),
            total: disk.total_space(),
            read: (usage.read_bytes as f64) / refresh.as_secs_f64(),
            write: (usage.written_bytes as f64) / refresh.as_secs_f64(),
            temp: None,
        }
    }
}
