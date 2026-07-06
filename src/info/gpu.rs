use std::time::Instant;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum GpuId {
    Other(u32),
    Pci {
        domain: u32,
        bus: u32,
        device: u32,
        func: u32,
    },
}

impl GpuId {
    pub fn parse_pci(string: &str) -> Option<Self> {
        let (domain_str, bus_device_func_str) = string.split_once(':')?;
        let (bus_str, device_func_str) = bus_device_func_str.split_once(':')?;
        let (device_str, func_str) = device_func_str.split_once('.')?;
        let domain = u32::from_str_radix(domain_str, 16).ok()?;
        let bus = u32::from_str_radix(bus_str, 16).ok()?;
        let device = u32::from_str_radix(device_str, 16).ok()?;
        let func = u32::from_str_radix(func_str, 16).ok()?;
        Some(Self::Pci {
            domain,
            bus,
            device,
            func,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GpuState {
    Normal,
    Idle(Instant),
    Suspended,
}

#[derive(Clone, Debug)]
pub struct GpuItem {
    pub boot_vga: bool,
    pub id: GpuId,
    pub name: String,
    pub state: GpuState,
    pub power: Option<f32>,
    pub usage: Option<f32>,
    pub temp: Option<f32>,
    pub vram_used: Option<u64>,
    pub vram_total: Option<u64>,
}
