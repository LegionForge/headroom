use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::cmp::Reverse;
use sysinfo::{ProcessesToUpdate, System};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessSnapshot {
    pub pid: u32,
    pub name: String,
    /// Virtual address space committed (VirtualAlloc total)
    pub virtual_bytes: u64,
    /// Physical pages currently resident (working set)
    pub memory_bytes: u64,
    /// CPU usage 0–100 per logical core
    pub cpu_percent: f32,
}

/// Returns up to `n` processes sorted by virtual memory descending.
pub fn collect_top(n: usize) -> Result<Vec<ProcessSnapshot>> {
    let mut sys = System::new();
    sys.refresh_processes(ProcessesToUpdate::All, true);

    let mut procs: Vec<ProcessSnapshot> = sys
        .processes()
        .values()
        .map(|p| ProcessSnapshot {
            pid: p.pid().as_u32(),
            name: p.name().to_string_lossy().into_owned(),
            virtual_bytes: p.virtual_memory(),
            memory_bytes: p.memory(),
            cpu_percent: p.cpu_usage(),
        })
        .collect();

    procs.sort_by_key(|b| Reverse(b.memory_bytes));
    procs.truncate(n);
    Ok(procs)
}
