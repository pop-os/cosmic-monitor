use sysinfo::Cpu;

#[derive(Clone, Debug)]
pub struct CpuItem {
    pub brand: String,
    pub name: String,
    pub usage: f32,
}

impl CpuItem {
    pub fn new(cpu: &Cpu) -> Self {
        Self {
            brand: cpu.brand().into(),
            name: cpu.name().into(),
            usage: cpu.cpu_usage(),
        }
    }
}
