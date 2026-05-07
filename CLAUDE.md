# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Commands

```bash
cargo build                        # debug build
cargo build --release              # release build (use for perf benchmarks)
cargo run                          # launch TUI monitor (default 2s refresh)
cargo run -- --interval 5          # TUI with 5s refresh
cargo run -- --snapshot            # one-shot JSON snapshot to stdout
cargo run -- --snapshot --ai       # snapshot + Claude AI recommendations
cargo test                         # all tests
cargo test collect::               # collector unit tests only
cargo clippy -- -D warnings        # lint (must be clean before commit)
cargo build --target x86_64-unknown-linux-gnu   # cross-compile check for Linux
```

Set `ANTHROPIC_API_KEY` in env or `~/.config/headroom/config.toml` (Windows: `%APPDATA%\headroom\config.toml`) for AI recommendations.

## Architecture

headroom is a single-binary terminal monitor focused on memory commit pressure, paging, and allocation diagnostics. Windows primary; Linux and macOS supported.

### Module layout

```
src/
  main.rs            clap CLI, tokio entry point
  config.rs          Config + SystemProfile, loaded from TOML
  collect/
    mod.rs           SystemSnapshot, collect_snapshot()
    memory.rs        MemorySnapshot — commit charge, pools, hard fault rate
    paging.rs        PagefileSnapshot — pagefile/swap usage per entry
    process.rs       ProcessSnapshot — top N processes by virtual commit
  tui/
    mod.rs           ratatui + crossterm event loop, App state machine
  ai/
    mod.rs           Claude API call, formats snapshot into diagnostic prompt
```

### Platform abstraction

No separate `platform/` directory. Each `collect/*.rs` file contains `#[cfg(target_os)]`-gated private functions (`collect_impl()`) for each OS. The public `collect()` function dispatches at compile time. Adding a new OS means adding a `#[cfg]` block in each collector — no trait objects, no dynamic dispatch.

### Key Windows metrics and why they matter

| Metric | Source API | Diagnostic value |
|--------|-----------|-----------------|
| `CommitTotal` / `CommitLimit` | `GetPerformanceInfo` | Commit pressure — allocations reserved but not yet paged in. **This is the primary failure mode on 64 GB machines.** |
| `PhysicalAvailable` | `GetPerformanceInfo` | Physical RAM actually free — distinct from commit headroom |
| `KernelPaged` / `KernelNonpaged` | `GetPerformanceInfo` | Pool exhaustion causes allocation failures independently of commit |
| `SystemCache` | `GetPerformanceInfo` | Standby cache consuming physical pages (reclaimable) |
| Swap total/used | `sysinfo` | Aggregate pagefile consumption |
| Per-process virtual bytes | `sysinfo` | Identifies top commit consumers (Hyper-V, JVMs, Chromium) |

### Commit charge vs physical RAM — the key concept

`CommitLimit = PhysicalTotal + all pagefile sizes`. Windows commits memory on `VirtualAlloc` even if pages are never touched. A machine with 64 GB RAM + 16 GB pagefile has an 80 GB ceiling. Allocation failures occur when `CommitTotal` approaches `CommitLimit`, even if physical RAM shows gigabytes free. Hyper-V, JVM runtimes, and Chromium-family processes are the heaviest over-committers.

### TUI key bindings

| Key | Action |
|-----|--------|
| `r` | Force refresh |
| `a` | Trigger AI analysis (async, requires API key) |
| `q` / Esc | Quit |

### AI integration

`src/ai/mod.rs` formats the current `SystemSnapshot` and system profile into a structured prompt sent to `claude-sonnet-4-6`. The call is async and non-blocking — the TUI shows "Analyzing..." while the request is in flight, then renders the response in a scrollable bottom pane. Use `--ai` flag for CLI mode (blocks until response).
