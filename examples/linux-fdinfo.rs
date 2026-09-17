use std::{
    collections::BTreeMap,
    fs,
    os::linux::fs::MetadataExt,
    path::Path,
    time::{Duration, Instant},
};

pub struct DrmFdInfo {
    client_id: libc::c_uint,
    engines: Vec<(String, u64)>,
    totals: Vec<(String, u64)>,
}

impl DrmFdInfo {
    pub fn new(data: &str) -> Option<Self> {
        let mut client_id = None;
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
                    engines.push((key.to_string(), nanos));
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
            engines,
            totals,
        })
    }
}

fn main() {
    let mut fdinfos = BTreeMap::new();

    let mut fdinfos_from_proc_path = |pid: libc::pid_t, proc_path: &Path| {
        let proc_fd_path = proc_path.join("fd");
        let proc_fdinfo_path = proc_path.join("fdinfo");
        let Ok(entries) = fs::read_dir(&proc_fd_path) else {
            return;
        };
        for entry_res in entries {
            let Ok(entry) = entry_res else { continue };
            let path = entry.path();
            let Ok(metadata) = fs::metadata(&path) else {
                return;
            };
            // DRI devices are character devices with major dev number 226
            // https://www.kernel.org/doc/Documentation/admin-guide/devices.txt
            if metadata.st_mode() & libc::S_IFMT == libc::S_IFCHR
                && libc::major(metadata.st_rdev()) == 226
            {
                let name = entry.file_name();
                if let Ok(data) = fs::read_to_string(proc_fdinfo_path.join(&name))
                    && let Some(fdinfo) = DrmFdInfo::new(&data) {
                        let minor = libc::minor(metadata.st_rdev());
                        // Only one (minor device number, drm client id) pair is inserted to avoid duplicates
                        fdinfos
                            .entry(pid)
                            .or_insert_with(BTreeMap::new)
                            .entry((minor, fdinfo.client_id))
                            .or_insert(fdinfo);
                    }
            }
        }
    };

    let instant = Instant::now();
    let mut count = 0;
    for pid_str in std::env::args().skip(1) {
        let Ok(pid) = pid_str.parse::<libc::pid_t>() else {
            continue;
        };
        let proc_path = Path::new("/proc").join(pid_str);
        fdinfos_from_proc_path(pid, &proc_path);
        count += 1;
    }
    if count == 0
        && let Ok(entries) = fs::read_dir("/proc") {
            for entry_res in entries {
                let Ok(entry) = entry_res else { continue };
                let file_name = entry.file_name();
                let Some(pid_str) = file_name.to_str() else {
                    continue;
                };
                let Ok(pid) = pid_str.parse::<libc::pid_t>() else {
                    continue;
                };
                let proc_path = entry.path();
                fdinfos_from_proc_path(pid, &proc_path);
                count += 1;
            }
        }
    let elapsed = instant.elapsed();

    let instant = Instant::now();
    for (pid, clients) in fdinfos.iter() {
        println!("PID {}", pid);
        for ((minor, client_id), fdinfo) in clients.iter() {
            println!("  DEV {} CLIENT {}", minor, client_id);
            for (name, nanos) in fdinfo.engines.iter() {
                println!("    engine-{}: {:?}", name, Duration::from_nanos(*nanos));
            }
            for (name, bytes) in fdinfo.totals.iter() {
                println!(
                    "    total-{}: {}",
                    name,
                    humansize::format_size(*bytes, humansize::BINARY)
                );
            }
        }
    }

    eprintln!(
        "Collected {} processes in {:?}, printed in {:?}",
        count,
        elapsed,
        instant.elapsed()
    );
}
