# headroom

**Windows-first system stability monitor focused on memory commit pressure, paging, and allocation diagnostics.**

## Why This Exists

Modern development machines (especially with VMs, large IDEs, Chromium, and JVM runtimes) can exhaust virtual memory ceiling without showing physical RAM pressure. Windows allocates memory on `VirtualAlloc()` even if pages are never touched — reserving from the **commit ceiling**, not physical RAM.

**The problem:** A machine with 64 GB RAM + 16 GB pagefile has an 80 GB ceiling. When `CommitTotal` approaches `CommitLimit`, allocations fail — but Task Manager shows "40 GB free." The two numbers measure different things.

**headroom** shows the real constraint: commit pressure, pool exhaustion, hard fault rate, and per-process virtual memory consumption. It's designed to surface the failures *before* they crash your machine.

### Key Insight: Commit Charge vs Physical RAM

```
CommitLimit = PhysicalTotal + sum(pagefile sizes)
CommitTotal = all reserved memory (touched or not)

Allocation failures happen when CommitTotal → CommitLimit, 
even if PhysicalAvailable is large.
```

See [Windows Memory Metrics](#windows-memory-metrics) below for the full breakdown.

---

## Installation

### Requirements

- **Windows 10/11** (primary platform)
- **Rust 1.70+** ([rustup.rs](https://rustup.rs))
- Linux and macOS supported via inline platform-specific code (`#[cfg]`)

### Build

```bash
cargo build                    # debug binary: target/debug/headroom.exe
cargo build --release          # optimized: target/release/headroom.exe (recommended)
```

### Install (Windows)

1. Build release: `cargo build --release`
2. Move `target/release/headroom.exe` to a directory in `%PATH%`, e.g., `C:\Users\<user>\bin\`
3. Create config file: `%APPDATA%\headroom\config.toml` (see [Configuration](#configuration))

---

## Usage

### TUI Monitor (Interactive)

```bash
headroom                      # refresh every 2 seconds (default)
headroom --interval 5         # refresh every 5 seconds
```

**Key bindings:**

| Key | Action |
|-----|--------|
| `r` | Force immediate refresh |
| `a` | Trigger AI analysis (requires API key) |
| `q` / `Esc` | Quit |
| `PgUp` / `PgDn` | Scroll AI pane (if present) |

The TUI displays:
- **Commit charge:** CommitTotal / CommitLimit (primary failure mode)
- **Physical RAM:** Available / Total
- **Kernel pools:** Paged and non-paged (pool exhaustion causes independent failures)
- **System cache:** Standby pages (reclaimable)
- **Top processes:** Ranked by virtual commit (who's over-reserving?)
- **Paging activity:** Pagefile usage by drive

### Snapshot Mode (One-shot JSON)

```bash
headroom --snapshot           # prints JSON to stdout
headroom --snapshot --ai      # JSON + Claude AI recommendation
```

Useful for:
- Scripting and monitoring integration
- CI/CD diagnostics
- Piping to `jq` for specific metrics

---

## Configuration

Create `%APPDATA%\headroom\config.toml`:

```toml
[system_profile]
cpu = "i7-13650HX"
ram_gb = 64
gpu = "RTX 4060"
use_cases = ["development", "gaming", "media"]

[[system_profile.drives]]
label = "OS"
drive_letter = "C"
kind = "nvme"

[[system_profile.drives]]
label = "Data"
drive_letter = "D"
kind = "nvme"

[ai]
# Optional: set ANTHROPIC_API_KEY env var instead
api_key = "sk-ant-..."
# Supported: "claude", "ollama", "openai"
provider = "claude"
# For local providers (ollama, lmstudio): http://127.0.0.1:11434
local_endpoint = ""
```

**AI Analysis:**

- **Claude (recommended):** Set `ANTHROPIC_API_KEY` environment variable or use config
- **Ollama (local):** `headroom --snapshot --ai` with Ollama running on `127.0.0.1:11434`
- **OpenAI-compatible:** Set `local_endpoint` and `OPENAI_API_KEY`

---

## Testing

### Unit Tests

```bash
cargo test                    # all tests
cargo test collect::          # collector tests only
cargo test -- --nocapture    # show println output
```

### Manual Testing Checklist

1. **Verify Windows API reads match Task Manager:**
   ```bash
   cargo run -- --snapshot | jq .memory
   ```
   Compare `commit_total` / `commit_limit` and `physical_available` to Task Manager → Performance → Memory.

2. **Stress test commit pressure:**
   - Open a Hyper-V VM with 32 GB virtual memory
   - Watch CommitTotal climb; verify `--ai` analysis identifies Hyper-V as the consumer

3. **Test per-process ranking:**
   - Run Chrome (multi-tab) and a JVM app (e.g., IntelliJ)
   - Verify top processes reflect known over-committers

4. **AI analysis latency:**
   - Run `headroom --snapshot --ai` and measure response time (should be < 5s for Claude)

### Cross-Platform Build Check

```bash
# Verify Linux/macOS compilation (no binary produced, only check syntax)
cargo build --target x86_64-unknown-linux-gnu --no-default-features
```

---

## Architecture

### Module Layout

```
src/
  main.rs            clap CLI, tokio entry point
  config.rs          Config + SystemProfile, loaded from TOML
  collect/
    mod.rs           SystemSnapshot, collect_snapshot() dispatcher
    memory.rs        MemorySnapshot — GetPerformanceInfo (Windows)
    paging.rs        PagefileSnapshot — aggregate swap/pagefile
    process.rs       ProcessSnapshot — top N by virtual commit
  tui/
    mod.rs           ratatui + crossterm event loop, App state machine
  ai/
    mod.rs           Claude/Ollama/OpenAI API call, diagnostic prompt
```

### Platform Abstraction

**No separate `platform/` directory.** Each `collect/*.rs` file contains `#[cfg(target_os)]`-gated `collect_impl()` functions. At compile time, the public `collect()` function dispatches to the correct OS implementation. This avoids circular dependencies and keeps the module graph flat.

---

## Windows Memory Metrics

| Metric | API | What it measures | Why it matters |
|--------|-----|------------------|----------------|
| **CommitTotal** | `GetPerformanceInfo` | Virtual memory reserved (not yet paged in) | Primary failure mode — allocations fail when this approaches CommitLimit |
| **CommitLimit** | `GetPerformanceInfo` | Ceiling = Physical RAM + all pagefile sizes | Hard cap on total reservations |
| **PhysicalAvailable** | `GetPerformanceInfo` | RAM actually free (not cache, not standby) | Distinct from commit headroom |
| **KernelPaged** | `GetPerformanceInfo` | Kernel memory in page pool | Pool exhaustion causes allocation failures independently of commit |
| **KernelNonpaged** | `GetPerformanceInfo` | Kernel memory in non-paged pool | Fixed resource; depletion is catastrophic |
| **SystemCache** | `GetPerformanceInfo` | Standby pages held by file cache | Reclaimable; not a failure mode |
| Per-process virtual bytes | `sysinfo` | Virtual memory reserved by each process | Identifies over-committers (Hyper-V, JVM, Chromium) |
| Hard fault rate | `GetPerformanceInfo` (PageFaultCount) | Pages brought from disk per second | Indicates swap thrashing |

---

## Planned Features (v0.2+)

- [ ] Per-pagefile detail: `NtQuerySystemInformation(SystemPagingFileInformation)` for C: vs D: breakdown
- [ ] Hard fault rate: track PageFaultCount diff between snapshots to measure swap thrashing
- [ ] MCP server mode: expose collectors as Model Context Protocol tools
- [ ] Persistent metrics: SQLite or CSV logging for trend analysis

---

## License

MIT. See LICENSE file.

---

## Author

**JP Cruz** — [GitHub](https://github.com/jp-cruz) | [LegionForge](https://github.com/LegionForge)

Developed for systems where commit pressure is the real bottleneck, not physical RAM.
