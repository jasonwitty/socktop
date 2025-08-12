// gpu.rs
use gfxinfo::active_gpu;

#[derive(Debug, Clone, serde::Serialize)]
pub struct GpuMetrics {
    pub name: String,
    pub utilization_gpu_pct: u32, // 0..100
    pub mem_used_bytes: u64,
    pub mem_total_bytes: u64,
}

pub fn collect_all_gpus() -> Result<Vec<GpuMetrics>, Box<dyn std::error::Error>> {
    let gpu = active_gpu()?; // Use ? to unwrap Result
    let info = gpu.info();

    let metrics = GpuMetrics {
        name: gpu.model().to_string(),
        utilization_gpu_pct: info.load_pct() as u32,
        mem_used_bytes: info.used_vram(),
        mem_total_bytes: info.total_vram(),
    };

    Ok(vec![metrics])
}
