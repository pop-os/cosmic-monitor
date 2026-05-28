use sysinfo::Pid;

#[cfg(target_os = "linux")]
mod linux;

#[cfg(feature = "nvml")]
mod nvml;

pub trait Platform {
    fn refresh_processes(&mut self) {}

    fn process_gpu_usage(&self, _pid: Pid) -> Option<f32> {
        None
    }
}

pub struct FallbackPlatform;

impl Platform for FallbackPlatform {}

pub fn default_platform() -> Box<dyn Platform> {
    #[cfg(target_os = "linux")]
    return Box::new(linux::LinuxPlatform::new());

    #[allow(unreachable_code)]
    Box::new(FallbackPlatform)
}
