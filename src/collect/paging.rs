use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PagefileSnapshot {
    /// Aggregate across all pagefile/swap entries
    pub total_bytes: u64,
    pub used_bytes: u64,
    /// Per-file entries (Windows: one per pagefile; Linux/macOS: one per swap device)
    pub entries: Vec<PagefileEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PagefileEntry {
    /// Path or device label
    pub path: String,
    pub total_bytes: u64,
    pub used_bytes: u64,
}

impl PagefileSnapshot {
    pub fn usage_ratio(&self) -> f64 {
        if self.total_bytes == 0 { return 0.0; }
        self.used_bytes as f64 / self.total_bytes as f64
    }
}

impl PagefileEntry {
    pub fn usage_ratio(&self) -> f64 {
        if self.total_bytes == 0 { return 0.0; }
        self.used_bytes as f64 / self.total_bytes as f64
    }
}

pub fn collect() -> Result<PagefileSnapshot> {
    collect_impl()
}

// ── Windows ───────────────────────────────────────────────────────────────────
// Uses sysinfo for aggregate swap; per-file detail requires NtQuerySystemInformation
// (SystemPagingFileInformation) which is added as a future enhancement.

#[cfg(target_os = "windows")]
fn collect_impl() -> Result<PagefileSnapshot> {
    use sysinfo::System;

    let mut sys = System::new();
    sys.refresh_memory();

    let total = sys.total_swap();
    let used = sys.used_swap();

    Ok(PagefileSnapshot {
        total_bytes: total,
        used_bytes: used,
        entries: vec![PagefileEntry {
            path: "pagefile (aggregate)".into(),
            total_bytes: total,
            used_bytes: used,
        }],
    })
}

// ── Linux ─────────────────────────────────────────────────────────────────────

#[cfg(target_os = "linux")]
fn collect_impl() -> Result<PagefileSnapshot> {
    use std::collections::HashMap;

    let content = std::fs::read_to_string("/proc/meminfo")?;
    let mut map: HashMap<&str, u64> = HashMap::new();
    for line in content.lines() {
        if let Some((key, val)) = line.split_once(':') {
            let kb: u64 = val.split_whitespace().next()
                .and_then(|v| v.parse().ok())
                .unwrap_or(0);
            map.insert(key.trim(), kb * 1024);
        }
    }

    let total = *map.get("SwapTotal").unwrap_or(&0);
    let free = *map.get("SwapFree").unwrap_or(&0);
    let used = total.saturating_sub(free);

    Ok(PagefileSnapshot {
        total_bytes: total,
        used_bytes: used,
        entries: vec![PagefileEntry {
            path: "swap (aggregate)".into(),
            total_bytes: total,
            used_bytes: used,
        }],
    })
}

// ── macOS ─────────────────────────────────────────────────────────────────────

#[cfg(target_os = "macos")]
fn collect_impl() -> Result<PagefileSnapshot> {
    use sysinfo::System;

    let mut sys = System::new();
    sys.refresh_memory();

    let total = sys.total_swap();
    let used = sys.used_swap();

    Ok(PagefileSnapshot {
        total_bytes: total,
        used_bytes: used,
        entries: vec![PagefileEntry {
            path: "swap".into(),
            total_bytes: total,
            used_bytes: used,
        }],
    })
}

#[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
fn collect_impl() -> Result<PagefileSnapshot> {
    anyhow::bail!("unsupported platform")
}
