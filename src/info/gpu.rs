#[derive(Clone, Debug)]
pub struct GpuItem {
    pub bus_id: String,
    pub name: String,
    pub usage: Option<f32>,
    pub vram_used: Option<u64>,
    pub vram_total: Option<u64>,
}
