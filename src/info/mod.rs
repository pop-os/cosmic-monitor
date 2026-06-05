use cosmic::iced::{
    futures::{SinkExt, Stream},
    stream,
};
use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
    thread,
    time::{Duration, Instant},
};
use sysinfo::{
    Components, CpuRefreshKind, Disks, InterfaceOperationalState, MemoryRefreshKind, Networks,
    ProcessRefreshKind, RefreshKind, System, UpdateKind, Users,
};
use tokio::sync::mpsc;

use crate::Message;

mod app;
pub use self::app::*;

mod cpu;
pub use self::cpu::*;

mod disk;
pub use self::disk::*;

mod gpu;
pub use self::gpu::*;

mod memory;
pub use self::memory::*;

mod network;
pub use self::network::*;

mod platform;
pub use self::platform::*;

mod process;
pub use self::process::*;

#[derive(Clone, Debug)]
pub struct GraphItem {
    pub time: Instant,
    pub cpus: Vec<CpuItem>,
    pub disks: Vec<DiskItem>,
    pub gpus: Vec<GpuItem>,
    pub memory: MemoryItem,
    pub networks: Vec<NetworkItem>,
}

impl GraphItem {
    pub fn new(
        time: Instant,
        sys: &System,
        components: &Components,
        disks: &Disks,
        networks: &Networks,
        platform: &Box<dyn Platform>,
        refresh: Duration,
    ) -> Self {
        let cpus = sys.cpus();
        let mut cpu_items = Vec::with_capacity(cpus.len());
        for cpu in cpus {
            cpu_items.push(CpuItem::new(cpu, components));
        }

        let disk_list = disks.list();
        let mut disk_items = Vec::with_capacity(disk_list.len());
        for disk in disk_list {
            disk_items.push(DiskItem::new(disk, platform, refresh));
        }

        let network_list = networks.list();
        let mut network_items = Vec::with_capacity(network_list.len());
        for (name, data) in network_list.iter() {
            network_items.push(NetworkItem::new(name, data, refresh));
        }
        network_items.sort_by(|a, b| {
            let weight = |state| match state {
                InterfaceOperationalState::Up => 0,
                InterfaceOperationalState::Dormant => 1,
                InterfaceOperationalState::Unknown => 2,
                _ => 3,
            };
            weight(a.state).cmp(&weight(b.state))
        });

        Self {
            time,
            cpus: cpu_items,
            disks: disk_items,
            gpus: platform.gpus(),
            memory: MemoryItem::new(&sys),
            networks: network_items,
        }
    }

    pub fn max_cpu_temp(&self) -> Option<f32> {
        let mut max = None;
        for cpu in self.cpus.iter() {
            if let Some(temp) = cpu.temp {
                max = Some(max.map_or(temp, |x| temp.max(x)));
            }
        }
        max
    }

    pub fn max_cpu_frequency(&self) -> u64 {
        let mut max = 0;
        for cpu in self.cpus.iter() {
            max = cpu.frequency.max(max);
        }
        max
    }

    pub fn total_cpu_usage(&self) -> f32 {
        let mut total = 0.0;
        for cpu in self.cpus.iter() {
            total += cpu.usage;
        }
        total / (self.cpus.len() as f32)
    }

    pub fn total_disk_io(&self) -> (f64, f64) {
        let mut total = (0.0, 0.0);
        for disk in self.disks.iter() {
            total.0 += disk.read;
            total.1 += disk.write;
        }
        total
    }

    pub fn total_network_io(&self) -> (f64, f64) {
        let mut total = (0.0, 0.0);
        for network in self.networks.iter() {
            total.0 += network.rx;
            total.1 += network.tx;
        }
        total
    }
}

pub fn worker() -> impl Stream<Item = Message> {
    stream::channel(16, async |mut output| {
        let (tx, mut rx) = mpsc::channel(1);

        //TODO: configurable refresh times
        let processes_refresh = Duration::from_millis(3000);
        let graph_refresh = sysinfo::MINIMUM_CPU_UPDATE_INTERVAL;

        let platform_lock = Arc::new(RwLock::new(default_platform()));

        // Gather graph information
        {
            let platform_lock = platform_lock.clone();
            let tx = tx.clone();
            thread::spawn(move || {
                // Ignore first samples so disk and network speeds are accurate
                let mut ignore = 4;
                let mut sys = System::new();
                let mut components = Components::new();
                let mut disks = Disks::new();
                let mut networks = Networks::new();
                loop {
                    let time = Instant::now();
                    sys.refresh_specifics(
                        RefreshKind::nothing()
                            .with_cpu(CpuRefreshKind::nothing().with_cpu_usage().with_frequency())
                            .with_memory(MemoryRefreshKind::nothing().with_ram().with_swap()),
                    );
                    components.refresh(true);
                    disks.refresh(true);
                    networks.refresh(true);
                    {
                        let mut platform = platform_lock.write().unwrap();
                        platform.refresh(&components, false);
                    }

                    if ignore > 0 {
                        ignore -= 1;
                    } else {
                        let message = {
                            let platform = platform_lock.read().unwrap();
                            Message::Graph(GraphItem::new(
                                time,
                                &sys,
                                &components,
                                &disks,
                                &networks,
                                &platform,
                                graph_refresh,
                            ))
                        };

                        match tx.blocking_send(message) {
                            Ok(()) => {}
                            Err(_) => break,
                        }
                    }
                    thread::sleep(graph_refresh);
                }
            });
        }

        // Gather snapshot information
        thread::spawn(move || {
            //TODO: refresh users periodically?
            let users = Users::new_with_refreshed_list();
            let mut sys = System::new();
            let mut components = Components::new();
            let mut disks = Disks::new();
            let mut networks = Networks::new();
            loop {
                let time = Instant::now();
                sys.refresh_specifics(
                    RefreshKind::nothing()
                        .with_cpu(CpuRefreshKind::nothing().with_cpu_usage().with_frequency())
                        .with_memory(MemoryRefreshKind::nothing().with_ram().with_swap())
                        .with_processes(
                            ProcessRefreshKind::nothing()
                                .with_cmd(UpdateKind::OnlyIfNotSet)
                                .with_cpu()
                                .with_disk_usage()
                                .with_exe(UpdateKind::OnlyIfNotSet)
                                .with_memory()
                                .with_user(UpdateKind::OnlyIfNotSet),
                        ),
                );
                components.refresh(true);
                disks.refresh(true);
                networks.refresh(true);
                {
                    let mut platform = platform_lock.write().unwrap();
                    platform.refresh(&components, true);
                }

                let message = {
                    let platform = platform_lock.read().unwrap();
                    let graph_item = GraphItem::new(
                        time,
                        &sys,
                        &components,
                        &disks,
                        &networks,
                        &platform,
                        processes_refresh,
                    );

                    let processes = sys.processes();
                    let mut apps = HashMap::new();
                    let mut process_items = Vec::with_capacity(processes.len());
                    for (_pid, process) in processes.iter() {
                        // Do not show threads
                        if process.thread_kind().is_some() {
                            continue;
                        }
                        let item = ProcessItem::new(
                            process,
                            &sys,
                            &graph_item.gpus,
                            &platform,
                            &users,
                            processes_refresh,
                        );
                        if let Some(app) = &item.app {
                            let app_item = apps
                                .entry((app.id.clone(), item.username.clone()))
                                .or_insert_with(|| {
                                    let mut app_item = ProcessItem::default();
                                    app_item.app = Some(app.clone());
                                    app_item.name = app.name.clone().unwrap_or_default();
                                    app_item.username = item.username.clone();
                                    app_item
                                });
                            app_item.cpu_usage += item.cpu_usage;
                            app_item.disk_read += item.disk_read;
                            app_item.disk_write += item.disk_write;
                            app_item.disk_total += item.disk_total;
                            for (gpu_id, info) in item.gpu_usages.iter() {
                                app_item
                                    .gpu_usages
                                    .entry(*gpu_id)
                                    .or_insert(ProcessGpuInfo::default())
                                    .add(info);
                            }
                            app_item.gpu_total.add(&item.gpu_total);
                            app_item.memory += item.memory;
                        }
                        process_items.push(item);
                    }
                    let mut app_items = Vec::with_capacity(apps.len());
                    for ((_id, _user), mut app) in apps {
                        app.generate_strings();
                        app_items.push(app);
                    }
                    Message::Snapshot(graph_item, app_items, process_items)
                };

                match tx.blocking_send(message) {
                    Ok(()) => {}
                    Err(_) => break,
                }

                thread::sleep(processes_refresh);
            }
        });

        while let Some(msg) = rx.recv().await {
            output.send(msg).await.unwrap();
        }
    })
}
