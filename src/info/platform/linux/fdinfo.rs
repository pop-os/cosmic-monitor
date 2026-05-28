use libc::c_uint;
use std::{collections::HashMap, fs, os::linux::fs::MetadataExt, path::Path};

pub struct FdInfo {
    pub client_id: c_uint,
    pub pdev: Option<String>,
    pub engines: Vec<(String, u64, f32)>,
    pub totals: Vec<(String, u64)>,
}

impl FdInfo {
    pub fn for_proc_path(proc_path: &Path) -> HashMap<(c_uint, c_uint), Self> {
        let mut fdinfos = HashMap::new();
        let proc_fd_path = proc_path.join("fd");
        let proc_fdinfo_path = proc_path.join("fdinfo");
        if let Ok(entries) = fs::read_dir(&proc_fd_path) {
            for entry_res in entries {
                let Ok(entry) = entry_res else { continue };
                let path = entry.path();
                let Ok(metadata) = fs::metadata(&path) else {
                    continue;
                };
                // DRI devices are character devices with major dev number 226
                // https://www.kernel.org/doc/Documentation/admin-guide/devices.txt
                if metadata.st_mode() & libc::S_IFMT == libc::S_IFCHR
                    && libc::major(metadata.st_rdev()) == 226
                {
                    let name = entry.file_name();
                    if let Ok(data) = fs::read_to_string(proc_fdinfo_path.join(&name)) {
                        if let Some(fdinfo) = Self::new(&data) {
                            let minor = libc::minor(metadata.st_rdev());
                            // Only one (minor device number, drm client id) pair is inserted to avoid duplicates
                            fdinfos.entry((minor, fdinfo.client_id)).or_insert(fdinfo);
                        }
                    }
                }
            }
        }
        fdinfos
    }

    pub fn new(data: &str) -> Option<Self> {
        let mut client_id = None;
        let mut pdev = None;
        let mut totals = Vec::new();
        let mut engines = Vec::new();
        for line in data.lines() {
            let Some((key, value)) = line.split_once(":") else {
                continue;
            };
            // https://docs.kernel.org/gpu/drm-usage-stats.html
            if let Some(key) = key.strip_prefix("drm-") {
                let value = value.trim_start();
                if key == "client-id" {
                    client_id = value.parse().ok();
                } else if key == "pdev" {
                    pdev = Some(value.to_string());
                } else if let Some(key) = key.strip_prefix("engine-") {
                    if key.starts_with("capacity-") {
                        continue;
                    }
                    let mut parts = value.splitn(2, ' ');
                    let Ok(nanos) = parts.next().unwrap_or_default().parse::<u64>() else {
                        continue;
                    };
                    match parts.next().unwrap_or_default() {
                        "ns" => {
                            // Nanoseconds
                        }
                        // Other suffixes not defined
                        _ => {
                            continue;
                        }
                    }
                    engines.push((key.to_string(), nanos, 0.0));
                } else if let Some(key) = key.strip_prefix("total-") {
                    let mut parts = value.splitn(2, ' ');
                    let Ok(mut bytes) = parts.next().unwrap_or_default().parse::<u64>() else {
                        continue;
                    };
                    match parts.next().unwrap_or_default() {
                        "KiB" => {
                            // Kilobytes
                            bytes *= 1024;
                        }
                        "MiB" => {
                            // Megabytes
                            bytes *= 1024 * 1024;
                        }
                        // Other suffixes not defined
                        _ => {
                            continue;
                        }
                    }
                    totals.push((key.to_string(), bytes))
                }
            }
        }

        Some(Self {
            client_id: client_id?,
            pdev,
            engines,
            totals,
        })
    }
}
