//! GPU utilisation backends, tried in order:
//! 1. NVIDIA NVML (cargo feature `nvidia`), loaded at runtime — no link-time dependency.
//! 2. Linux sysfs `gpu_busy_percent` (AMD amdgpu, some Intel Xe).
//! 3. sysinfo's `Gpus` (Windows PDH GPU engine counters, macOS IOKit, some Linux).
//!
//! Every backend failure degrades to `None`; nothing here may panic the collector.

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GpuSample {
    /// 0..=100
    pub util: f32,
    /// Memory-controller utilisation, 0..=100, if the backend provides it.
    pub mem_util: Option<f32>,
}

enum Backend {
    #[cfg(feature = "nvidia")]
    Nvml { nvml: Box<nvml_wrapper::Nvml>, count: u32 },
    #[cfg(target_os = "linux")]
    Sysfs(Vec<std::path::PathBuf>),
    Sysinfo(sysinfo::Gpus),
    None,
}

pub struct GpuSampler {
    backend: Backend,
}

#[cfg(feature = "nvidia")]
fn try_nvml() -> Option<Backend> {
    // NVML loads a shared library; guard against any panic inside the wrapper.
    let r = std::panic::catch_unwind(|| {
        let nvml = nvml_wrapper::Nvml::init().ok()?;
        let count = nvml.device_count().ok()?;
        (count > 0).then(|| Backend::Nvml { nvml: Box::new(nvml), count })
    });
    r.ok().flatten()
}

#[cfg(target_os = "linux")]
fn try_sysfs() -> Option<Backend> {
    let mut paths = Vec::new();
    let rd = std::fs::read_dir("/sys/class/drm").ok()?;
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        // card0, card1 ... but not card0-DP-1 connectors
        if !name.starts_with("card") || name.contains('-') {
            continue;
        }
        let p = e.path().join("device").join("gpu_busy_percent");
        if std::fs::read_to_string(&p).ok().and_then(|s| s.trim().parse::<f32>().ok()).is_some() {
            paths.push(p);
        }
    }
    (!paths.is_empty()).then_some(Backend::Sysfs(paths))
}

fn try_sysinfo() -> Option<Backend> {
    let gpus = sysinfo::Gpus::new_with_refreshed_list().ok()?;
    (!gpus.is_empty()).then_some(Backend::Sysinfo(gpus))
}

#[cfg(target_os = "linux")]
fn read_pct(p: &std::path::Path) -> Option<f32> {
    std::fs::read_to_string(p).ok()?.trim().parse::<f32>().ok().map(|v| v.clamp(0.0, 100.0))
}

impl GpuSampler {
    pub fn new() -> Self {
        let mut backend = None;
        #[cfg(feature = "nvidia")]
        {
            backend = backend.or_else(try_nvml);
        }
        #[cfg(target_os = "linux")]
        {
            backend = backend.or_else(try_sysfs);
        }
        backend = backend.or_else(try_sysinfo);
        Self { backend: backend.unwrap_or(Backend::None) }
    }

    pub fn backend_name(&self) -> &'static str {
        match self.backend {
            #[cfg(feature = "nvidia")]
            Backend::Nvml { .. } => "nvml",
            #[cfg(target_os = "linux")]
            Backend::Sysfs(_) => "sysfs",
            Backend::Sysinfo(_) => "sysinfo",
            Backend::None => "none",
        }
    }

    /// Highest utilisation across all GPUs.
    pub fn sample(&mut self) -> Option<GpuSample> {
        match &mut self.backend {
            #[cfg(feature = "nvidia")]
            Backend::Nvml { nvml, count } => {
                let mut best: Option<GpuSample> = None;
                for i in 0..*count {
                    let Ok(dev) = nvml.device_by_index(i) else { continue };
                    let Ok(u) = dev.utilization_rates() else { continue };
                    let s = GpuSample {
                        util: (u.gpu as f32).clamp(0.0, 100.0),
                        mem_util: Some((u.memory as f32).clamp(0.0, 100.0)),
                    };
                    if best.is_none_or(|b| s.util > b.util) {
                        best = Some(s);
                    }
                }
                best
            }
            #[cfg(target_os = "linux")]
            Backend::Sysfs(paths) => {
                let mut best: Option<GpuSample> = None;
                for p in paths.iter() {
                    let Some(util) = read_pct(p) else { continue };
                    let mem_util = p.parent().and_then(|d| read_pct(&d.join("mem_busy_percent")));
                    if best.is_none_or(|b| util > b.util) {
                        best = Some(GpuSample { util, mem_util });
                    }
                }
                best
            }
            Backend::Sysinfo(gpus) => {
                gpus.refresh(false);
                gpus.iter()
                    .filter_map(|g| g.usage())
                    .filter(|u| u.is_finite())
                    .map(|u| u.clamp(0.0, 100.0))
                    .reduce(f32::max)
                    .map(|util| GpuSample { util, mem_util: None })
            }
            Backend::None => None,
        }
    }
}

impl Default for GpuSampler {
    fn default() -> Self {
        Self::new()
    }
}
