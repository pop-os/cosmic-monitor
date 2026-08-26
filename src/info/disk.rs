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
    pub utilization: f32,
    pub temp: Option<f32>,
}

pub fn disk_utilization(busy: Duration, elapsed: Duration) -> f32 {
    if elapsed.is_zero() {
        return 0.0;
    }

    (100.0 * busy.as_secs_f32() / elapsed.as_secs_f32()).min(100.0)
}

pub fn total_disk_utilization(disks: &[DiskItem]) -> f32 {
    disks
        .iter()
        .map(|disk| disk.utilization)
        .fold(0.0, f32::max)
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
            utilization: 0.0,
            temp: None,
        }
    }
}
