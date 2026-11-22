//! sysinfo-backed sampling of CPU, RAM, disk, network and this process.

use std::collections::HashSet;
use std::ffi::OsString;

use sysinfo::{
    CpuRefreshKind, DiskRefreshKind, Disks, MemoryRefreshKind, Networks, Pid, ProcessRefreshKind,
    ProcessesToUpdate, System,
};

use super::RawMetrics;
use crate::metrics::self_cpu_system_basis;

/// Where disk throughput comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DiskSource {
    /// Per-disk counters (`Disk::usage`).
    Disks,
    /// Sum of per-process I/O (fallback when no disks are listed).
    Processes,
}

pub struct SystemSampler {
    sys: Option<System>,
    disks: Option<Disks>,
    networks: Option<Networks>,
    pid: Option<Pid>,
    ncpus: usize,
    disk_source: DiskSource,
    /// Number of samples taken; the first one only primes the counters.
    samples: u64,
}

fn is_loopback(name: &OsString) -> bool {
    let n = name.to_string_lossy().to_ascii_lowercase();
    n == "lo" || n.starts_with("lo0") || n.contains("loopback")
}

impl SystemSampler {
    pub fn new() -> Self {
        let mut sys = System::new().ok();
        let mut ncpus = 0;
        if let Some(s) = sys.as_mut() {
            s.refresh_cpu_specifics(CpuRefreshKind::nothing().with_usage());
            ncpus = s.cpus().len();
        }
        let disks = Disks::new_with_refreshed_list_specifics(DiskRefreshKind::nothing().with_io_usage()).ok();
        let disk_source = match &disks {
            Some(d) if !d.is_empty() => DiskSource::Disks,
            _ => DiskSource::Processes,
        };
        let networks = Networks::new_with_refreshed_list().ok();
        let pid = sysinfo::get_current_pid().ok();

        let mut me = Self { sys, disks, networks, pid, ncpus: ncpus.max(1), disk_source, samples: 0 };
        // Prime the process counters so the first real sample has a baseline.
        me.refresh_processes();
        me
    }

    fn refresh_processes(&mut self) {
        let Some(sys) = self.sys.as_mut() else { return };
        match (self.disk_source, self.pid) {
            (DiskSource::Processes, _) => {
                sys.refresh_processes_specifics(
                    ProcessesToUpdate::All,
                    true,
                    ProcessRefreshKind::nothing().with_cpu().with_disk_usage(),
                );
            }
            (DiskSource::Disks, Some(pid)) => {
                sys.refresh_processes_specifics(
                    ProcessesToUpdate::Some(&[pid]),
                    true,
                    ProcessRefreshKind::nothing().with_cpu(),
                );
            }
            (DiskSource::Disks, None) => {}
        }
    }

    /// Take one sample. `elapsed` is the wall time since the previous call, seconds.
    pub fn sample(&mut self, elapsed: f64) -> RawMetrics {
        let mut m = RawMetrics { elapsed, ..RawMetrics::default() };
        let first = self.samples == 0;
        self.samples += 1;
        let dt = elapsed.max(1e-3);

        // --- CPU + memory ---
        if let Some(sys) = self.sys.as_mut() {
            sys.refresh_cpu_specifics(CpuRefreshKind::nothing().with_usage());
            sys.refresh_memory_specifics(MemoryRefreshKind::nothing().with_ram());
            let total = sys.global_cpu_usage();
            if total.is_finite() && !sys.cpus().is_empty() {
                m.cpu_total = Some(total.clamp(0.0, 100.0));
            }
            let tm = sys.total_memory();
            if tm > 0 {
                m.mem_total = Some(tm);
                m.mem_used = Some(sys.used_memory().min(tm));
            }
        }

        self.refresh_processes();
        if let (Some(sys), Some(pid)) = (self.sys.as_ref(), self.pid)
            && let Some(p) = sys.process(pid)
        {
            m.cpu_self = Some(self_cpu_system_basis(p.cpu_usage(), self.ncpus));
        }

        // --- disks ---
        match self.disk_source {
            DiskSource::Disks => {
                if let Some(disks) = self.disks.as_mut() {
                    disks.refresh_specifics(true, DiskRefreshKind::nothing().with_io_usage());
                    // De-duplicate the same device mounted twice (bind mounts,
                    // btrfs subvolumes): same name and same size.
                    let mut seen = HashSet::new();
                    let (mut r, mut w) = (0u64, 0u64);
                    for d in disks.iter() {
                        let name = d.name().to_string_lossy().into_owned();
                        let mount = d.mount_point().to_string_lossy().into_owned();
                        if !name.is_empty() && !seen.insert((name.clone(), d.total_space())) {
                            continue;
                        }
                        let u = d.usage();
                        r = r.saturating_add(u.read_bytes);
                        w = w.saturating_add(u.written_bytes);
                        if !first {
                            let (name, alt) = if name.is_empty() { (mount, None) } else { (name, Some(mount)) };
                            m.disks.push(super::IoSample {
                                name,
                                alt,
                                a_bps: u.read_bytes as f64 / dt,
                                b_bps: u.written_bytes as f64 / dt,
                            });
                        }
                    }
                    if !disks.is_empty() && !first {
                        m.disk_read_bps = Some(r as f64 / dt);
                        m.disk_write_bps = Some(w as f64 / dt);
                    }
                }
            }
            DiskSource::Processes => {
                if let Some(sys) = self.sys.as_ref() {
                    let (mut r, mut w) = (0u64, 0u64);
                    for p in sys.processes().values() {
                        let u = p.disk_usage();
                        r = r.saturating_add(u.read_bytes);
                        w = w.saturating_add(u.written_bytes);
                    }
                    if !sys.processes().is_empty() && !first {
                        m.disk_read_bps = Some(r as f64 / dt);
                        m.disk_write_bps = Some(w as f64 / dt);
                    }
                }
            }
        }

        // --- network ---
        if let Some(nets) = self.networks.as_mut() {
            nets.refresh(true);
            let (mut rx, mut tx) = (0u64, 0u64);
            let mut any = false;
            for (name, data) in nets.iter() {
                if is_loopback(name) {
                    continue;
                }
                any = true;
                rx = rx.saturating_add(data.received());
                tx = tx.saturating_add(data.transmitted());
                if !first {
                    m.nics.push(super::IoSample {
                        name: name.to_string_lossy().into_owned(),
                        alt: None,
                        a_bps: data.received() as f64 / dt,
                        b_bps: data.transmitted() as f64 / dt,
                    });
                }
            }
            m.nics.sort_by(|a, b| a.name.cmp(&b.name));
            if any && !first {
                m.net_rx_bps = Some(rx as f64 / dt);
                m.net_tx_bps = Some(tx as f64 / dt);
            }
        }

        m
    }
}

impl Default for SystemSampler {
    fn default() -> Self {
        Self::new()
    }
}
