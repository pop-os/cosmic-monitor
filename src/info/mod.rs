use cosmic::iced::{
    futures::{SinkExt, Stream},
    stream,
};
use std::{
    thread,
    time::{Duration, Instant},
};
use sysinfo::{
    Components, CpuRefreshKind, Disks, MemoryRefreshKind, Networks, ProcessRefreshKind,
    RefreshKind, System, UpdateKind, Users,
};
use tokio::sync::mpsc;

use crate::Message;

mod cpu;
pub use self::cpu::*;

mod disk;
pub use self::disk::*;

mod memory;
pub use self::memory::*;

mod network;
pub use self::network::*;

mod process;
pub use self::process::*;

#[derive(Clone, Debug)]
pub struct GraphItem {
    pub time: Instant,
    pub cpus: Vec<CpuItem>,
    pub disks: Vec<DiskItem>,
    pub memory: MemoryItem,
    pub networks: Vec<NetworkItem>,
}

impl GraphItem {
    pub fn new(
        time: Instant,
        sys: &System,
        disks: &Disks,
        networks: &Networks,
        refresh: Duration,
    ) -> Self {
        let cpus = sys.cpus();
        let mut cpu_items = Vec::with_capacity(cpus.len());
        for cpu in cpus {
            cpu_items.push(CpuItem::new(cpu));
        }

        let disk_list = disks.list();
        let mut disk_items = Vec::with_capacity(disk_list.len());
        for disk in disk_list {
            disk_items.push(DiskItem::new(disk, refresh));
        }

        let network_list = networks.list();
        let mut network_items = Vec::with_capacity(network_list.len());
        for (name, data) in network_list.iter() {
            network_items.push(NetworkItem::new(name, data, refresh));
        }

        Self {
            time,
            cpus: cpu_items,
            disks: disk_items,
            memory: MemoryItem::new(&sys),
            networks: network_items,
        }
    }

    pub fn total_cpu_usage(&self) -> f32 {
        let mut total = 0.0;
        for cpu in self.cpus.iter() {
            total += cpu.cpu_usage;
        }
        total / (self.cpus.len() as f32)
    }
}

pub fn worker() -> impl Stream<Item = Message> {
    stream::channel(16, async |mut output| {
        let (tx, mut rx) = mpsc::channel(1);

        //TODO: configurable refresh times
        let processes_refresh = Duration::from_millis(3000);
        let graph_refresh = sysinfo::MINIMUM_CPU_UPDATE_INTERVAL;

        // Gather graph information
        {
            let tx = tx.clone();
            thread::spawn(move || {
                //TODO: use components
                let components = Components::new_with_refreshed_list();
                for component in components.list() {
                    eprintln!(
                        "{:?}: {}: {:?}",
                        component.id(),
                        component.label(),
                        component.temperature()
                    );
                }

                // Ignore first samples so disk and network speeds are accurate
                let mut ignore = 4;
                let mut sys = System::new();
                let mut disks = Disks::new();
                let mut networks = Networks::new();
                loop {
                    let time = Instant::now();
                    sys.refresh_specifics(
                        RefreshKind::nothing()
                            .with_cpu(CpuRefreshKind::nothing().with_cpu_usage())
                            .with_memory(MemoryRefreshKind::nothing().with_ram().with_swap()),
                    );
                    disks.refresh(true);
                    networks.refresh(true);

                    let graph_item = GraphItem::new(time, &sys, &disks, &networks, graph_refresh);
                    if ignore > 0 {
                        ignore -= 1;
                    } else {
                        match tx.blocking_send(Message::Graph(graph_item)) {
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
            let mut disks = Disks::new();
            let mut networks = Networks::new();
            loop {
                let time = Instant::now();
                sys.refresh_specifics(
                    RefreshKind::nothing()
                        .with_cpu(CpuRefreshKind::nothing().with_cpu_usage())
                        .with_memory(MemoryRefreshKind::nothing().with_ram().with_swap())
                        .with_processes(
                            ProcessRefreshKind::nothing()
                                .with_cpu()
                                .with_disk_usage()
                                .with_memory()
                                .with_user(UpdateKind::OnlyIfNotSet),
                        ),
                );
                disks.refresh(true);
                networks.refresh(true);

                let graph_item = GraphItem::new(time, &sys, &disks, &networks, processes_refresh);

                let processes = sys.processes();
                let mut process_items = Vec::with_capacity(processes.len());
                for (_pid, process) in processes.iter() {
                    // Do not show threads
                    if process.thread_kind().is_some() {
                        continue;
                    }
                    process_items.push(ProcessItem::new(process, &users, processes_refresh));
                }

                match tx.blocking_send(Message::Snapshot(graph_item, process_items)) {
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
