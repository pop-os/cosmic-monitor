use std::{
    borrow::Cow, cmp::Ordering, collections::HashMap, fmt, path::Path, sync::Arc, time::Duration,
};

use cosmic::{
    iced::{Alignment, Length},
    widget::{
        self, Icon,
        table::{ItemCategory, ItemInterface},
    },
};
use humansize::{BINARY, DECIMAL, format_size};
use regex::Regex;
use sysinfo::{Pid, Process, System, Users};

use super::{GpuId, GpuItem, Platform};
use crate::{fl, info::AppEntry};

fn best_name(p: &Process) -> String {
    // Name is truncated on Linux, try to fill in using cmdline or exe
    let name = p.name().to_string_lossy().to_string();
    if let Some(cmd) = p
        .cmd()
        .get(0)
        .map(Path::new)
        .and_then(|x| x.file_name())
        .and_then(|x| x.to_str())
    {
        if cmd.starts_with(&name) {
            return cmd.to_string();
        }
    }
    if let Some(exe_name) = p.exe().and_then(|x| x.file_name()).and_then(|x| x.to_str()) {
        if exe_name.starts_with(&name) {
            return exe_name.to_string();
        } else {
            return format!("{} ({})", name, exe_name);
        }
    }
    name
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Hash)]
pub enum ProcessCategory {
    App,
    Name,
    User,
    PID,
    #[default]
    CPU,
    Memory,
    GpuUsage(GpuId, Option<usize>),
    GpuUsageTotal,
    GpuVram(GpuId, Option<usize>),
    GpuVramTotal,
    DiskRead,
    DiskWrite,
    DiskTotal,
    Priority,
}

impl ProcessCategory {
    pub fn for_applications(sort_category: Self) -> Vec<Self> {
        vec![
            Self::App,
            Self::Name,
            Self::User,
            Self::CPU,
            Self::Memory,
            if let Self::GpuUsage(..) = sort_category {
                sort_category
            } else {
                Self::GpuUsageTotal
            },
            if let Self::GpuVram(..) = sort_category {
                sort_category
            } else {
                Self::GpuVramTotal
            },
            // Having both disk read and write takes up too much space
            Self::DiskTotal,
        ]
    }

    pub fn for_processes(sort_category: Self) -> Vec<Self> {
        vec![
            Self::App,
            Self::Name,
            Self::User,
            Self::PID,
            Self::CPU,
            Self::Memory,
            if let Self::GpuUsage(..) = sort_category {
                sort_category
            } else {
                Self::GpuUsageTotal
            },
            if let Self::GpuVram(..) = sort_category {
                sort_category
            } else {
                Self::GpuVramTotal
            },
            // Having both disk read and write takes up too much space
            Self::DiskTotal,
            Self::Priority,
        ]
    }

    pub fn for_top_processes(sort_category: Self) -> Vec<Self> {
        vec![
            Self::App,
            Self::Name,
            Self::CPU,
            Self::Memory,
            if let Self::GpuUsage(..) = sort_category {
                sort_category
            } else {
                Self::GpuUsageTotal
            },
            if let Self::GpuVram(..) = sort_category {
                sort_category
            } else {
                Self::GpuVramTotal
            },
            // Having both disk read and write takes up too much space
            Self::DiskTotal,
        ]
    }

    pub fn data_align(&self) -> Alignment {
        match self {
            Self::Name | Self::User | Self::Priority => Alignment::Start,
            Self::App
            | Self::PID
            | Self::CPU
            | Self::Memory
            | Self::GpuUsage(..)
            | Self::GpuUsageTotal
            | Self::GpuVram(..)
            | Self::GpuVramTotal
            | Self::DiskRead
            | Self::DiskWrite
            | Self::DiskTotal => Alignment::End,
        }
    }
}

impl fmt::Display for ProcessCategory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}",
            match self {
                Self::App => fl!("app"),
                Self::Name => fl!("name"),
                Self::User => fl!("user"),
                Self::PID => fl!("pid"),
                Self::CPU => fl!("cpu"),
                Self::Memory => fl!("memory"),
                Self::GpuUsage(_, Some(i)) => fl!("gpu-index", index = i),
                Self::GpuUsage(_, None) => fl!("gpu-index", index = "?"),
                Self::GpuUsageTotal => fl!("gpu"),
                Self::GpuVram(_, Some(i)) => fl!("gpu-vram-index", index = i),
                Self::GpuVram(_, None) => fl!("gpu-vram-index", index = "?"),
                Self::GpuVramTotal => fl!("gpu-vram"),
                Self::DiskRead => fl!("disk-read"),
                Self::DiskWrite => fl!("disk-write"),
                Self::DiskTotal => fl!("disk"),
                Self::Priority => fl!("priority"),
            }
        )
    }
}

impl ItemCategory for ProcessCategory {
    fn width(&self) -> Length {
        match self {
            Self::App => Length::Fixed(64.0),
            Self::Name => Length::Fill,
            Self::User | Self::PID | Self::Priority => Length::Fixed(96.0),
            Self::CPU | Self::GpuUsageTotal => Length::Fixed(64.0),
            Self::GpuUsage(..) => Length::Fixed(80.0),
            Self::Memory
            | Self::GpuVramTotal
            | Self::DiskRead
            | Self::DiskWrite
            | Self::DiskTotal => Length::Fixed(96.0),
            Self::GpuVram(..) => Length::Fixed(112.0),
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ProcessGpuInfo {
    pub index: Option<usize>,
    pub usage: Option<u32>,
    pub vram: Option<u64>,
}

impl ProcessGpuInfo {
    pub fn add(&mut self, other: &Self) {
        if let Some(usage) = other.usage {
            self.usage = Some(self.usage.map_or(usage, |x| x + usage));
        }
        if let Some(vram) = other.vram {
            self.vram = Some(self.vram.map_or(vram, |x| x + vram));
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ProcessItem {
    pub app: Option<Arc<AppEntry>>,
    pub cpu_usage: u32,
    pub disk_read: u64,
    pub disk_write: u64,
    pub disk_total: u64,
    pub gpu_usages: HashMap<GpuId, ProcessGpuInfo>,
    pub gpu_total: ProcessGpuInfo,
    pub memory: u64,
    pub name: String,
    pub parent: Option<Pid>,
    pub pid: Option<Pid>,
    pub priority: Option<i32>,
    pub username: String,
    pub strings: HashMap<ProcessCategory, String>,
}

impl ProcessItem {
    pub fn new(
        p: &Process,
        sys: &System,
        gpus: &[GpuItem],
        platform: &Box<dyn Platform>,
        users: &Users,
        refresh: Duration,
    ) -> Self {
        let app = platform.process_app(p, sys);

        let cpu_usage = ((p.cpu_usage() / (sys.cpus().len() as f32)) * 10.0) as u32;

        let disk_usage = p.disk_usage();
        let disk_read = disk_usage.read_bytes / refresh.as_secs();
        let disk_write = disk_usage.written_bytes / refresh.as_secs();
        let disk_total = disk_read + disk_write;

        let pid = p.pid();

        let mut gpu_total = ProcessGpuInfo {
            index: None,
            usage: None,
            vram: None,
        };
        let mut gpu_usages = HashMap::new();
        for (gpu_id, (usage_float, vram)) in platform.process_gpu_usage(pid) {
            let info = ProcessGpuInfo {
                index: gpus.iter().position(|gpu| gpu.id == gpu_id),
                usage: Some((usage_float * 10.0) as u32),
                vram: Some(vram),
            };
            gpu_total.add(&info);
            gpu_usages.insert(gpu_id, info);
        }

        let memory = p.memory();

        let name = best_name(&p);

        let mut priority = None;

        #[cfg(unix)]
        if let Some(pid) = rustix::process::Pid::from_raw(p.pid().as_u32() as _) {
            match rustix::process::getpriority_process(Some(pid)) {
                Ok(ok) => {
                    priority = Some(ok);
                }
                Err(err) => {
                    log::debug!("failed to get priority for {}: {}", p.pid(), err);
                }
            }
        }

        let username = match p.user_id() {
            Some(uid) => match users.get_user_by_id(uid) {
                Some(user) => user.name().to_string(),
                None => uid.to_string(),
            },
            None => String::new(),
        };

        let mut this = Self {
            app,
            cpu_usage,
            disk_read,
            disk_write,
            disk_total,
            gpu_usages,
            gpu_total,
            memory,
            name,
            parent: p.parent(),
            pid: Some(pid),
            priority,
            username,
            strings: HashMap::new(),
        };
        this.generate_strings();
        this
    }

    pub fn generate_strings(&mut self) {
        self.strings.insert(
            ProcessCategory::CPU,
            format!("{}.{}%", self.cpu_usage / 10, self.cpu_usage % 10),
        );
        self.strings.insert(
            ProcessCategory::DiskRead,
            format!("{}/s", format_size(self.disk_read, DECIMAL)),
        );
        self.strings.insert(
            ProcessCategory::DiskWrite,
            format!("{}/s", format_size(self.disk_write, DECIMAL)),
        );
        self.strings.insert(
            ProcessCategory::DiskTotal,
            format!("{}/s", format_size(self.disk_total, DECIMAL)),
        );
        for (gpu_id, info) in self.gpu_usages.iter() {
            if let Some(usage) = info.usage {
                self.strings.insert(
                    ProcessCategory::GpuUsage(*gpu_id, info.index),
                    format!("{}.{}%", usage / 10, usage % 10),
                );
            }
            if let Some(vram) = info.vram {
                self.strings.insert(
                    ProcessCategory::GpuVram(*gpu_id, info.index),
                    format!("{}", format_size(vram, BINARY)),
                );
            }
        }
        if let Some(usage) = self.gpu_total.usage {
            self.strings.insert(
                ProcessCategory::GpuUsageTotal,
                format!("{}.{}%", usage / 10, usage % 10),
            );
        }
        if let Some(vram) = self.gpu_total.vram {
            self.strings.insert(
                ProcessCategory::GpuVramTotal,
                format!("{}", format_size(vram, BINARY)),
            );
        }
        self.strings.insert(
            ProcessCategory::Memory,
            format!("{}", format_size(self.memory, BINARY)),
        );
        self.strings.insert(
            ProcessCategory::PID,
            self.pid.map(|x| x.to_string()).unwrap_or_default(),
        );
        //TODO: translate
        self.strings.insert(
            ProcessCategory::Priority,
            self.priority
                .map_or("N/A", |x| {
                    if x < -7 {
                        "Very high"
                    } else if x < -2 {
                        "High"
                    } else if x < 3 {
                        "Normal"
                    } else if x < 7 {
                        "Low"
                    } else {
                        "Very low"
                    }
                })
                .to_string(),
        );
    }

    pub fn matches(&self, regex: &Regex) -> bool {
        regex.is_match(&self.name)
            || regex.is_match(&self.username)
            || self
                .strings
                .get(&ProcessCategory::PID)
                .map_or(false, |x| regex.is_match(x))
    }

    // Like get_text but without allocation
    pub fn text(&self, category: ProcessCategory) -> &str {
        match category {
            // Only the icon is shown
            ProcessCategory::App => "",
            ProcessCategory::Name => &self.name,
            ProcessCategory::User => &self.username,
            _ => {
                //TODO: only generate strings when necessary?
                if let Some(string) = self.strings.get(&category) {
                    string.as_str()
                } else {
                    ""
                }
            }
        }
    }
}

impl ItemInterface<ProcessCategory> for ProcessItem {
    fn get_icon(&self, category: ProcessCategory) -> Option<Icon> {
        match category {
            ProcessCategory::App => {
                let icon = self.app.as_ref()?.icon.as_ref()?.as_str();
                Some(widget::icon::from_name(icon).size(24).icon())
            }
            _ => None,
        }
    }

    //TODO: Use Cow<'a, str> instead so borrows of strings work
    fn get_text(&self, category: ProcessCategory) -> Cow<'static, str> {
        Cow::Owned(self.text(category).into())
    }

    fn compare(&self, other: &Self, category: ProcessCategory) -> Ordering {
        match category {
            ProcessCategory::App => match (
                self.app.as_ref().and_then(|x| x.name.as_ref()),
                other.app.as_ref().and_then(|x| x.name.as_ref()),
            ) {
                (Some(name), Some(other_name)) => name.cmp(&other_name),
                // Sort some name above no name
                (Some(_), None) => Ordering::Less,
                (None, Some(_)) => Ordering::Greater,
                (None, None) => Ordering::Equal,
            },
            ProcessCategory::Name => self.name.cmp(&other.name),
            ProcessCategory::User => self.username.cmp(&other.username),
            ProcessCategory::PID => self.pid.cmp(&other.pid),
            // These are sorted with higher values at the top
            ProcessCategory::CPU => other.cpu_usage.cmp(&self.cpu_usage),
            ProcessCategory::Memory => other.memory.cmp(&self.memory),
            ProcessCategory::GpuUsage(gpu_id, _) => {
                let self_usage = self
                    .gpu_usages
                    .get(&gpu_id)
                    .map(|x| x.usage)
                    .unwrap_or_default();
                let other_usage = other
                    .gpu_usages
                    .get(&gpu_id)
                    .map(|x| x.usage)
                    .unwrap_or_default();
                other_usage.cmp(&self_usage)
            }
            ProcessCategory::GpuUsageTotal => other.gpu_total.usage.cmp(&self.gpu_total.usage),
            ProcessCategory::GpuVram(gpu_id, _) => {
                let self_usage = self
                    .gpu_usages
                    .get(&gpu_id)
                    .map(|x| x.vram)
                    .unwrap_or_default();
                let other_usage = other
                    .gpu_usages
                    .get(&gpu_id)
                    .map(|x| x.vram)
                    .unwrap_or_default();
                other_usage.cmp(&self_usage)
            }
            ProcessCategory::GpuVramTotal => other.gpu_total.vram.cmp(&self.gpu_total.vram),
            ProcessCategory::DiskRead => other.disk_read.cmp(&self.disk_read),
            ProcessCategory::DiskWrite => other.disk_write.cmp(&self.disk_write),
            ProcessCategory::DiskTotal => other.disk_total.cmp(&self.disk_total),
            ProcessCategory::Priority => self.priority.cmp(&other.priority),
        }
    }
}
