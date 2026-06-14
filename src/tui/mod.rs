use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{
        Block, Borders, Gauge, Paragraph, Row, Scrollbar, ScrollbarOrientation, ScrollbarState,
        Table, Wrap,
    },
    Frame, Terminal,
};
use std::{
    io::{self, Stdout},
    time::{Duration, Instant},
};
use tokio::sync::mpsc;

use crate::{ai, ai::fmt_bytes, collect, collect::SystemSnapshot, config::Config};

const GB: f64 = 1_073_741_824.0;

enum AiState {
    Idle,
    Loading,
    Done(String),
    Error(String),
}

struct App {
    snapshot: SystemSnapshot,
    ai_state: AiState,
    ai_scroll: u16,
}

pub async fn run(cfg: Config, interval_secs: u64) -> Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = event_loop(&mut terminal, cfg, interval_secs).await;

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    result
}

async fn event_loop(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    cfg: Config,
    interval_secs: u64,
) -> Result<()> {
    let (ai_tx, mut ai_rx) = mpsc::channel::<Result<String, String>>(1);
    let tick = Duration::from_secs(interval_secs);
    let mut last_tick = Instant::now();

    let mut app = App {
        snapshot: collect::collect_snapshot()?,
        ai_state: AiState::Idle,
        ai_scroll: 0,
    };

    loop {
        terminal.draw(|f| draw(f, &app))?;

        if let Ok(result) = ai_rx.try_recv() {
            app.ai_state = match result {
                Ok(text) => AiState::Done(text),
                Err(e) => AiState::Error(e),
            };
            app.ai_scroll = 0;
        }

        let poll_timeout = Duration::from_millis(100);
        if event::poll(poll_timeout)? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    let ai_has_content =
                        matches!(app.ai_state, AiState::Done(_) | AiState::Error(_));
                    match key.code {
                        KeyCode::Char('q') | KeyCode::Esc => break,
                        KeyCode::Char('r') => {
                            app.snapshot = collect::collect_snapshot()?;
                            last_tick = Instant::now();
                        }
                        KeyCode::Char('a') => {
                            if !matches!(app.ai_state, AiState::Loading) {
                                app.ai_state = AiState::Loading;
                                app.ai_scroll = 0;
                                let snap = app.snapshot.clone();
                                let cfg_clone = cfg.clone();
                                let tx = ai_tx.clone();
                                tokio::spawn(async move {
                                    let result = ai::get_recommendations(&snap, &cfg_clone)
                                        .await
                                        .map_err(|e| e.to_string());
                                    let _ = tx.send(result).await;
                                });
                            }
                        }
                        KeyCode::Up if ai_has_content => {
                            app.ai_scroll = app.ai_scroll.saturating_sub(1);
                        }
                        KeyCode::Down if ai_has_content => {
                            app.ai_scroll = app.ai_scroll.saturating_add(1);
                        }
                        KeyCode::PageUp if ai_has_content => {
                            app.ai_scroll = app.ai_scroll.saturating_sub(10);
                        }
                        KeyCode::PageDown if ai_has_content => {
                            app.ai_scroll = app.ai_scroll.saturating_add(10);
                        }
                        _ => {}
                    }
                }
            }
        }

        if last_tick.elapsed() >= tick {
            app.snapshot = collect::collect_snapshot()?;
            last_tick = Instant::now();
        }
    }

    Ok(())
}

fn draw(f: &mut Frame, app: &App) {
    let has_ai = !matches!(app.ai_state, AiState::Idle);
    let ai_height = if has_ai { 12 } else { 0 };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(7),         // memory
            Constraint::Length(4),         // pagefile
            Constraint::Min(6),            // processes
            Constraint::Length(ai_height), // ai pane (collapsed when idle)
            Constraint::Length(1),         // help bar
        ])
        .split(f.area());

    draw_memory(f, &app.snapshot, chunks[0]);
    draw_paging(f, &app.snapshot, chunks[1]);
    draw_processes(f, &app.snapshot, chunks[2]);

    if has_ai {
        draw_ai(f, &app.ai_state, app.ai_scroll, chunks[3]);
    }

    let scrollable = matches!(app.ai_state, AiState::Done(_) | AiState::Error(_));
    let help = Paragraph::new(Line::from(if scrollable {
        vec![
            key_span("r"),
            Span::raw("refresh  "),
            key_span("a"),
            Span::raw("AI  "),
            key_span("↑↓"),
            Span::raw("scroll  "),
            key_span("PgUp/Dn"),
            Span::raw("fast scroll  "),
            key_span("q"),
            Span::raw("quit"),
        ]
    } else {
        vec![
            key_span("r"),
            Span::raw("refresh  "),
            key_span("a"),
            Span::raw("AI analysis  "),
            key_span("q"),
            Span::raw("quit"),
        ]
    }));
    f.render_widget(help, chunks[4]);
}

fn draw_memory(f: &mut Frame, snap: &SystemSnapshot, area: Rect) {
    let mem = &snap.memory;
    let block = Block::default().title(" Memory ").borders(Borders::ALL);
    let inner = block.inner(area);
    f.render_widget(block, area);

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(inner);

    // Physical RAM gauge
    f.render_widget(
        Gauge::default()
            .ratio(mem.physical_ratio().min(1.0))
            .gauge_style(Style::default().fg(ratio_color(mem.physical_ratio())))
            .label(format!(
                "Physical   {:.1} / {:.1} GB  ({:.0}%)",
                mem.used_bytes as f64 / GB,
                mem.total_bytes as f64 / GB,
                mem.physical_ratio() * 100.0
            )),
        rows[0],
    );

    // Commit charge gauge — this is the critical one
    f.render_widget(
        Gauge::default()
            .ratio(mem.commit_ratio().min(1.0))
            .gauge_style(Style::default().fg(ratio_color(mem.commit_ratio())))
            .label(format!(
                "Committed  {:.1} / {:.1} GB  ({:.0}%)  ← commit pressure",
                mem.committed_bytes as f64 / GB,
                mem.commit_limit_bytes as f64 / GB,
                mem.commit_ratio() * 100.0
            )),
        rows[1],
    );

    // Pool and cache info line
    f.render_widget(
        Paragraph::new(format!(
            "  Paged pool: {:.2} GB   Non-paged pool: {:.2} GB   Standby cache: {:.1} GB",
            mem.paged_pool_bytes as f64 / GB,
            mem.non_paged_pool_bytes as f64 / GB,
            mem.cached_bytes as f64 / GB,
        )),
        rows[2],
    );

    // Available / hard fault rate
    f.render_widget(
        Paragraph::new(format!(
            "  Available: {:.1} GB   Hard faults: {:.0}/sec   Gap (committed-used): {:.1} GB",
            mem.available_bytes as f64 / GB,
            mem.hard_fault_rate,
            mem.committed_bytes.saturating_sub(mem.used_bytes) as f64 / GB,
        )),
        rows[3],
    );
}

fn draw_paging(f: &mut Frame, snap: &SystemSnapshot, area: Rect) {
    let paging = &snap.paging;
    let block = Block::default()
        .title(" Pagefile / Swap ")
        .borders(Borders::ALL);
    let inner = block.inner(area);
    f.render_widget(block, area);

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Length(1)])
        .split(inner);

    f.render_widget(
        Gauge::default()
            .ratio(paging.usage_ratio().min(1.0))
            .gauge_style(Style::default().fg(ratio_color(paging.usage_ratio())))
            .label(format!(
                "Total  {:.1} / {:.1} GB  ({:.0}%)",
                paging.used_bytes as f64 / GB,
                paging.total_bytes as f64 / GB,
                paging.usage_ratio() * 100.0
            )),
        rows[0],
    );

    let entry_text: String = paging
        .entries
        .iter()
        .map(|e| {
            format!(
                "  {}  {:.1}/{:.1} GB  ({:.0}%)",
                e.path,
                e.used_bytes as f64 / GB,
                e.total_bytes as f64 / GB,
                e.usage_ratio() * 100.0
            )
        })
        .collect::<Vec<_>>()
        .join("   ");
    f.render_widget(Paragraph::new(entry_text), rows[1]);
}

fn draw_processes(f: &mut Frame, snap: &SystemSnapshot, area: Rect) {
    let rows: Vec<Row> = snap
        .top_processes
        .iter()
        .map(|p| {
            Row::new(vec![
                format!("{:>7}", p.pid),
                p.name.clone(),
                format!("{:>7}", fmt_bytes(p.virtual_bytes)),
                format!("{:>7}", fmt_bytes(p.memory_bytes)),
                format!("{:>5.1}%", p.cpu_percent),
            ])
        })
        .collect();

    let header = Row::new(["PID", "Name", "Virt", "Phys", "CPU"])
        .style(Style::default().add_modifier(Modifier::BOLD));

    let table = Table::new(
        rows,
        [
            Constraint::Length(7),
            Constraint::Fill(1),
            Constraint::Length(8),
            Constraint::Length(8),
            Constraint::Length(7),
        ],
    )
    .header(header)
    .block(
        Block::default()
            .title(" Top Processes (by working set — Virt includes Chromium VA reservations) ")
            .borders(Borders::ALL),
    );

    f.render_widget(table, area);
}

fn draw_ai(f: &mut Frame, state: &AiState, scroll: u16, area: Rect) {
    let (title, content, style) = match state {
        AiState::Loading => (
            " AI Analysis — analyzing... ",
            "Waiting on AI response...".into(),
            Style::default().fg(Color::Yellow),
        ),
        AiState::Done(text) => (" AI Analysis ", text.clone(), Style::default()),
        AiState::Error(e) => (
            " AI Analysis — error ",
            format!("Error: {e}"),
            Style::default().fg(Color::Red),
        ),
        AiState::Idle => return,
    };

    let block = Block::default().title(title).borders(Borders::ALL);
    let inner = block.inner(area);
    f.render_widget(block, area);

    // Reserve 1 column on the right for the scrollbar.
    let text_width = inner.width.saturating_sub(1) as usize;
    let visible_lines = inner.height as usize;
    let total_lines = count_wrapped_lines(&content, text_width);

    let text_area = Rect {
        width: inner.width.saturating_sub(1),
        ..inner
    };

    f.render_widget(
        Paragraph::new(content)
            .style(style)
            .wrap(Wrap { trim: false })
            .scroll((scroll, 0)),
        text_area,
    );

    // Only draw the scrollbar when content overflows the pane.
    if total_lines > visible_lines {
        let mut sb_state = ScrollbarState::new(total_lines).position(scroll as usize);
        f.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .begin_symbol(Some("↑"))
                .end_symbol(Some("↓")),
            inner,
            &mut sb_state,
        );
    }
}

/// Estimates wrapped line count for `text` given a terminal width.
/// Uses character count per line as a proxy for word-wrapped layout.
fn count_wrapped_lines(text: &str, width: usize) -> usize {
    if width == 0 {
        return text.lines().count().max(1);
    }
    text.lines()
        .map(|line| {
            if line.is_empty() {
                1
            } else {
                (line.chars().count() + width - 1) / width
            }
        })
        .sum::<usize>()
        .max(1)
}

fn ratio_color(ratio: f64) -> Color {
    if ratio > 0.85 {
        Color::Red
    } else if ratio > 0.70 {
        Color::Yellow
    } else {
        Color::Green
    }
}

fn key_span(key: &str) -> Span<'_> {
    Span::styled(
        format!("[{key}]"),
        Style::default().add_modifier(Modifier::BOLD),
    )
}
