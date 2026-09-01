use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MemorySnapshot {
    /// Total physical RAM installed
    pub total_bytes: u64,
    /// Physical pages currently available (free + standby)
    pub available_bytes: u64,
    /// Physical pages in active use
    pub used_bytes: u64,
    /// Total virtual memory committed system-wide (VirtualAlloc reservations)
    pub committed_bytes: u64,
    /// Commit ceiling = physical RAM + total pagefile size
    pub commit_limit_bytes: u64,
    /// Kernel paged pool (can be swapped out)
    pub paged_pool_bytes: u64,
    /// Kernel non-paged pool (must stay in RAM)
    pub non_paged_pool_bytes: u64,
    /// Standby/file cache (physical pages holding cached file data, reclaimable)
    pub cached_bytes: u64,
    /// Hard page faults per second (requires consecutive snapshots; 0.0 on first)
    pub hard_fault_rate: f64,
}

impl MemorySnapshot {
    pub fn commit_ratio(&self) -> f64 {
        if self.commit_limit_bytes == 0 {
            return 0.0;
        }
        self.committed_bytes as f64 / self.commit_limit_bytes as f64
    }

    pub fn physical_ratio(&self) -> f64 {
        if self.total_bytes == 0 {
            return 0.0;
        }
        self.used_bytes as f64 / self.total_bytes as f64
    }
}

pub fn collect() -> Result<MemorySnapshot> {
    collect_impl()
}

// ── Windows ──────────────────────────────────────────────────────────────────

#[cfg(target_os = "windows")]
fn collect_impl() -> Result<MemorySnapshot> {
    use windows::Win32::System::ProcessStatus::{GetPerformanceInfo, PERFORMANCE_INFORMATION};

    // SAFETY: PERFORMANCE_INFORMATION is a plain Win32 struct of fixed-size
    // integer fields (u32/usize) with no pointers or niche invariants, so an
    // all-zero bit pattern is a valid value. `cb` is set to the struct's
    // real size immediately after, as GetPerformanceInfo requires for its
    // own size validation.
    let mut pi: PERFORMANCE_INFORMATION = unsafe { std::mem::zeroed() }; // nosemgrep: rust.lang.security.unsafe-usage.unsafe-usage -- zero-init a Win32 FFI struct before GetPerformanceInfo fills it in; see SAFETY comment above
    pi.cb = std::mem::size_of::<PERFORMANCE_INFORMATION>() as u32;
    // SAFETY: `pi` is a valid, correctly-sized PERFORMANCE_INFORMATION (see
    // above) and `&mut pi` is a unique, properly-aligned pointer for the
    // duration of this call, matching GetPerformanceInfo's documented
    // contract (https://learn.microsoft.com/windows/win32/api/psapi/nf-psapi-getperformanceinfo).
    unsafe { GetPerformanceInfo(&mut pi, pi.cb)? }; // nosemgrep: rust.lang.security.unsafe-usage.unsafe-usage -- calling a Win32 API is inherently unsafe FFI; standard `windows` crate idiom, see SAFETY comment above

    let page = pi.PageSize as u64;
    let total = pi.PhysicalTotal as u64 * page;
    let available = pi.PhysicalAvailable as u64 * page;

    Ok(MemorySnapshot {
        total_bytes: total,
        available_bytes: available,
        used_bytes: total.saturating_sub(available),
        committed_bytes: pi.CommitTotal as u64 * page,
        commit_limit_bytes: pi.CommitLimit as u64 * page,
        paged_pool_bytes: pi.KernelPaged as u64 * page,
        non_paged_pool_bytes: pi.KernelNonpaged as u64 * page,
        cached_bytes: pi.SystemCache as u64 * page,
        hard_fault_rate: 0.0,
    })
}

// ── Linux ─────────────────────────────────────────────────────────────────────

#[cfg(target_os = "linux")]
fn collect_impl() -> Result<MemorySnapshot> {
    use std::collections::HashMap;

    let content = std::fs::read_to_string("/proc/meminfo")?;
    let mut map: HashMap<&str, u64> = HashMap::new();

    for line in content.lines() {
        if let Some((key, val)) = line.split_once(':') {
            let kb: u64 = val
                .split_whitespace()
                .next()
                .and_then(|v| v.parse().ok())
                .unwrap_or(0);
            map.insert(key.trim(), kb * 1024);
        }
    }

    let get = |k| *map.get(k).unwrap_or(&0);
    let total = get("MemTotal");
    let available = get("MemAvailable");

    Ok(MemorySnapshot {
        total_bytes: total,
        available_bytes: available,
        used_bytes: total.saturating_sub(available),
        committed_bytes: get("Committed_AS"),
        commit_limit_bytes: get("CommitLimit").max(total),
        paged_pool_bytes: get("Slab"),
        non_paged_pool_bytes: get("KernelStack"),
        cached_bytes: get("Cached") + get("Buffers"),
        hard_fault_rate: 0.0,
    })
}

// ── macOS ─────────────────────────────────────────────────────────────────────

#[cfg(target_os = "macos")]
fn collect_impl() -> Result<MemorySnapshot> {
    use sysinfo::System;

    let mut sys = System::new();
    sys.refresh_memory();

    let total = sys.total_memory();
    let available = sys.available_memory();
    let swap_total = sys.total_swap();

    Ok(MemorySnapshot {
        total_bytes: total,
        available_bytes: available,
        used_bytes: sys.used_memory(),
        committed_bytes: total.saturating_sub(available), // approximate
        commit_limit_bytes: total + swap_total,
        paged_pool_bytes: 0,
        non_paged_pool_bytes: 0,
        cached_bytes: 0,
        hard_fault_rate: 0.0,
    })
}

#[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
fn collect_impl() -> Result<MemorySnapshot> {
    anyhow::bail!("unsupported platform")
}
