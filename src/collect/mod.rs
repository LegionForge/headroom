pub mod memory;
pub mod paging;
pub mod process;

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemSnapshot {
    pub timestamp: DateTime<Utc>,
    pub memory: memory::MemorySnapshot,
    pub paging: paging::PagefileSnapshot,
    pub top_processes: Vec<process::ProcessSnapshot>,
}

pub fn collect_snapshot() -> Result<SystemSnapshot> {
    Ok(SystemSnapshot {
        timestamp: Utc::now(),
        memory: memory::collect()?,
        paging: paging::collect()?,
        top_processes: process::collect_top(20)?,
    })
}
