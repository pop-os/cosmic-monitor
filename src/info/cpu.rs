use sysinfo::{Components, Cpu};

#[derive(Clone, Debug)]
pub struct CpuItem {
    pub brand: String,
    pub frequency: u64,
    pub name: String,
    pub power: Option<f32>,
    pub temp: Option<f32>,
    pub usage: f32,
}

impl CpuItem {
    pub fn new(cpu: &Cpu, components: &Components) -> Self {
        //TODO: implement for other OSes than Linux and actually match hwmon id to CPU
        let mut temp = None;
        match cpu.vendor_id() {
            "AuthenticAMD" => {
                for component in components {
                    let label = component.label();
                    if label.starts_with("k10temp") {
                        if let Some(c_temp) = component.temperature() {
                            temp = Some(temp.map_or(c_temp, |x| c_temp.max(x)));
                        }
                    }
                }
            }
            "GenuineIntel" => {
                //TODO: per-core temp
                for component in components {
                    let label = component.label();
                    if label.starts_with("coretemp") {
                        if let Some(c_temp) = component.temperature() {
                            temp = Some(temp.map_or(c_temp, |x| c_temp.max(x)));
                        }
                    }
                }
            }
            //TODO: more CPU vendors
            _ => {}
        }
        Self {
            brand: cpu.brand().into(),
            frequency: cpu.frequency(),
            name: cpu.name().into(),
            power: None,
            temp,
            usage: cpu.cpu_usage(),
        }
    }
}
