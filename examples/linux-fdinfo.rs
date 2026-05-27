use std::{fs, path::Path, time::Instant};

fn main() {
    let pid = std::env::args().nth(1).unwrap();
    let instant = Instant::now();
    let proc_path = Path::new("/proc").join(&pid);
    let proc_fd_path = proc_path.join("fd");
    let proc_fdinfo_path = proc_path.join("fdinfo");
    // Do we have to collect more than one per process?
    let mut fdinfos = Vec::new();
    for entry_res in fs::read_dir(&proc_fd_path).unwrap() {
        let Ok(entry) = entry_res else { continue };
        let path = entry.path();
        let Ok(link) = fs::read_link(&path) else {
            continue;
        };
        if link.starts_with("/dev/dri") {
            let name = entry.file_name();
            if let Ok(data) = fs::read_to_string(proc_fdinfo_path.join(&name)) {
                fdinfos.push((name, link, data));
            }
        }
    }
    let elapsed = instant.elapsed();
    for (name, link, data) in fdinfos {
        println!("{}: {}", name.display(), link.display());
        for line in data.lines() {
            let Some((key, value)) = line.split_once(":") else {
                continue;
            };
            // https://docs.kernel.org/gpu/drm-usage-stats.html
            if let Some(key) = key.strip_prefix("drm-") {
                let value = value.trim_start();
                if let Some(key) = key.strip_prefix("total-") {
                    let Some((value, suffix)) = value.split_once(' ') else {
                        continue;
                    };
                    println!("  total {}: {} {}", key, value, suffix);
                }
                if let Some(key) = key.strip_prefix("engine-") {
                    if key.starts_with("capacity-") {
                        continue;
                    }
                    let Some((value, suffix)) = value.split_once(' ') else {
                        continue;
                    };
                    println!("  engine {}: {} {}", key, value, suffix);
                }
            }
        }
    }
    eprintln!("Collected in {:?}", elapsed);
}
