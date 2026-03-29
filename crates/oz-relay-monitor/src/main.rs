// Copyright (c) 2026 OZ Global Inc.
// Licensed under the Apache License, Version 2.0.

//! oz-relay-monitor — TUI dashboard for the OZ Relay pipeline.
//! Reads filesystem state directly. No server API dependency.
//!
//! Usage: oz-relay-monitor [--data-dir /opt/oz-relay]

use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crossterm::event::{self, Event, KeyCode, KeyModifiers};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::ExecutableCommand;
use ratatui::prelude::*;
use ratatui::widgets::*;

fn main() -> io::Result<()> {
    let data_dir = std::env::args()
        .nth(1)
        .filter(|a| !a.starts_with('-'))
        .or_else(|| {
            std::env::args()
                .position(|a| a == "--data-dir")
                .and_then(|i| std::env::args().nth(i + 1))
        })
        .unwrap_or_else(|| "/opt/oz-relay".into());

    let data_dir = PathBuf::from(data_dir);

    // Setup terminal
    enable_raw_mode()?;
    io::stdout().execute(EnterAlternateScreen)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;

    let result = run_app(&mut terminal, &data_dir);

    // Restore terminal
    disable_raw_mode()?;
    io::stdout().execute(LeaveAlternateScreen)?;

    result
}

fn run_app(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>, data_dir: &Path) -> io::Result<()> {
    let tick_rate = Duration::from_secs(2);
    let mut last_tick = Instant::now();

    loop {
        let state = scan_state(data_dir);
        terminal.draw(|f| ui(f, &state, data_dir))?;

        let timeout = tick_rate.saturating_sub(last_tick.elapsed());
        if event::poll(timeout)? {
            if let Event::Key(key) = event::read()? {
                match key.code {
                    KeyCode::Char('q') => return Ok(()),
                    KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        return Ok(())
                    }
                    _ => {}
                }
            }
        }
        if last_tick.elapsed() >= tick_rate {
            last_tick = Instant::now();
        }
    }
}

/// All state derived from the filesystem.
struct DashboardState {
    submitted: usize,
    working: Vec<WorkingTask>,
    completed: usize,
    failed: usize,
    canceled: usize,

    promo_pending: usize,
    promo_approved: usize,
    promo_merged: usize,
    promo_rejected: usize,

    bugs_incoming: usize,
    bugs_triaged: usize,
    bugs_resolved: usize,

    recent_events: Vec<String>,
    total_events: usize,

    daily_cost: f64,
    daily_tokens: u64,
    daily_builds: usize,
}

struct WorkingTask {
    id: String,
    developer: String,
    description: String,
}

fn count_files(dir: &Path) -> usize {
    std::fs::read_dir(dir)
        .map(|entries| entries.filter_map(|e| e.ok()).filter(|e| {
            e.path().extension().map_or(false, |ext| ext == "json")
        }).count())
        .unwrap_or(0)
}

fn scan_working(dir: &Path) -> Vec<WorkingTask> {
    let mut tasks = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.filter_map(|e| e.ok()) {
            if let Ok(content) = std::fs::read_to_string(entry.path()) {
                if let Ok(val) = serde_json::from_str::<serde_json::Value>(&content) {
                    tasks.push(WorkingTask {
                        id: val.get("id").and_then(|v| v.as_str()).unwrap_or("?").chars().take(8).collect(),
                        developer: val.get("owner").and_then(|v| v.as_str()).unwrap_or("?").into(),
                        description: val.get("messages")
                            .and_then(|m| m.as_array())
                            .and_then(|a| a.first())
                            .and_then(|m| m.get("parts"))
                            .and_then(|p| p.as_array())
                            .and_then(|a| a.first())
                            .and_then(|p| p.get("data"))
                            .and_then(|d| d.get("description"))
                            .and_then(|d| d.as_str())
                            .unwrap_or("(unknown)")
                            .chars().take(50).collect(),
                    });
                }
            }
        }
    }
    tasks
}

fn scan_ledger(ledger_path: &Path) -> (Vec<String>, usize, f64, u64, usize) {
    let mut recent = Vec::new();
    let mut total = 0;
    let mut daily_cost = 0.0;
    let mut daily_tokens = 0u64;
    let mut daily_builds = 0usize;

    let today = chrono::Utc::now().format("%Y-%m-%d").to_string();

    if let Ok(content) = std::fs::read_to_string(ledger_path) {
        for line in content.lines() {
            if line.trim().is_empty() { continue; }
            total += 1;

            if let Ok(val) = serde_json::from_str::<serde_json::Value>(line) {
                let ts = val.get("ts").and_then(|v| v.as_str()).unwrap_or("");
                let event = val.get("event").and_then(|v| v.as_str()).unwrap_or("");
                let tid = val.get("task_id").and_then(|v| v.as_str()).unwrap_or("");
                let short_tid = &tid[..tid.len().min(8)];

                recent.push(format!("{}  {:25}  {}", &ts[..ts.len().min(19)], event, short_tid));

                // Count daily costs from build.completed/build.failed events
                if ts.starts_with(&today) && event.starts_with("build.") {
                    daily_builds += 1;
                    if let Some(cost) = val.get("cost_usd").and_then(|v| v.as_str()).and_then(|s| s.parse::<f64>().ok()) {
                        daily_cost += cost;
                    }
                    if let Some(tokens) = val.get("total_tokens").and_then(|v| v.as_u64()) {
                        daily_tokens += tokens;
                    }
                }
            }
        }
    }

    // Keep last 15 events
    let recent: Vec<String> = recent.into_iter().rev().take(15).collect();
    (recent, total, daily_cost, daily_tokens, daily_builds)
}

fn scan_state(data_dir: &Path) -> DashboardState {
    let tasks = data_dir.join("tasks");
    let promos = data_dir.join("promotions");
    let bugs = data_dir.join("bugs");
    let ledger = data_dir.join("ledger/events.jsonl");

    let (recent_events, total_events, daily_cost, daily_tokens, daily_builds) = scan_ledger(&ledger);

    DashboardState {
        submitted: count_files(&tasks.join("submitted")),
        working: scan_working(&tasks.join("working")),
        completed: count_files(&tasks.join("completed")),
        failed: count_files(&tasks.join("failed")),
        canceled: count_files(&tasks.join("canceled")),

        promo_pending: count_files(&promos.join("pending")),
        promo_approved: count_files(&promos.join("approved")),
        promo_merged: count_files(&promos.join("merged")),
        promo_rejected: count_files(&promos.join("rejected")),

        bugs_incoming: count_files(&bugs.join("incoming")),
        bugs_triaged: count_files(&bugs.join("triaged")),
        bugs_resolved: count_files(&bugs.join("resolved")),

        recent_events,
        total_events,
        daily_cost,
        daily_tokens,
        daily_builds,
    }
}

fn ui(f: &mut Frame, state: &DashboardState, data_dir: &Path) {
    let area = f.area();

    // Main layout: header + 3 columns + footer
    let main_layout = Layout::vertical([
        Constraint::Length(3),   // header
        Constraint::Min(10),    // body
        Constraint::Length(3),  // footer
    ]).split(area);

    // Header
    let header = Block::default()
        .borders(Borders::ALL)
        .title(format!(" OZ Relay Monitor — {} ", data_dir.display()))
        .title_alignment(Alignment::Center);
    f.render_widget(header, main_layout[0]);

    // Body: 3 columns
    let body = Layout::horizontal([
        Constraint::Percentage(30),
        Constraint::Percentage(35),
        Constraint::Percentage(35),
    ]).split(main_layout[1]);

    // Left column: Pipeline + Promotions + Bugs
    let left = Layout::vertical([
        Constraint::Length(9),
        Constraint::Length(8),
        Constraint::Min(6),
    ]).split(body[0]);

    // Pipeline
    let working_count = state.working.len();
    let pipeline_items = vec![
        Line::from(vec![
            Span::styled("  submitted/  ", Style::default().fg(Color::Yellow)),
            Span::raw(format!("{}", state.submitted)),
        ]),
        Line::from(vec![
            Span::styled("  working/    ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            Span::styled(format!("{}", working_count), Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            Span::raw(format!(" {}", "█".repeat(working_count.min(10)))),
        ]),
        Line::from(vec![
            Span::styled("  completed/  ", Style::default().fg(Color::Green)),
            Span::raw(format!("{}", state.completed)),
        ]),
        Line::from(vec![
            Span::styled("  failed/     ", Style::default().fg(Color::Red)),
            Span::raw(format!("{}", state.failed)),
        ]),
        Line::from(vec![
            Span::styled("  canceled/   ", Style::default().fg(Color::DarkGray)),
            Span::raw(format!("{}", state.canceled)),
        ]),
    ];
    let pipeline = Paragraph::new(pipeline_items)
        .block(Block::default().borders(Borders::ALL).title(" Pipeline "));
    f.render_widget(pipeline, left[0]);

    // Promotions
    let promo_items = vec![
        Line::from(format!("  pending/   {}", state.promo_pending)),
        Line::from(format!("  approved/  {}", state.promo_approved)),
        Line::from(format!("  merged/    {}", state.promo_merged)),
        Line::from(format!("  rejected/  {}", state.promo_rejected)),
    ];
    let promos = Paragraph::new(promo_items)
        .block(Block::default().borders(Borders::ALL).title(" Promotions "));
    f.render_widget(promos, left[1]);

    // Bugs
    let bug_items = vec![
        Line::from(format!("  incoming/  {}", state.bugs_incoming)),
        Line::from(format!("  triaged/   {}", state.bugs_triaged)),
        Line::from(format!("  resolved/  {}", state.bugs_resolved)),
    ];
    let bugs = Paragraph::new(bug_items)
        .block(Block::default().borders(Borders::ALL).title(" Bugs "));
    f.render_widget(bugs, left[2]);

    // Middle column: Active builds + Cost
    let middle = Layout::vertical([
        Constraint::Length(10),
        Constraint::Min(6),
    ]).split(body[1]);

    // Active builds
    let mut build_lines = Vec::new();
    if state.working.is_empty() {
        build_lines.push(Line::from(Span::styled("  (no active builds)", Style::default().fg(Color::DarkGray))));
    } else {
        for task in &state.working {
            build_lines.push(Line::from(vec![
                Span::styled(format!("  {} ", task.id), Style::default().fg(Color::Cyan)),
                Span::raw(&task.developer),
            ]));
            build_lines.push(Line::from(format!("    {}", task.description)));
            build_lines.push(Line::from(""));
        }
    }
    let active = Paragraph::new(build_lines)
        .block(Block::default().borders(Borders::ALL).title(" Active Builds "));
    f.render_widget(active, middle[0]);

    // Cost
    let total_tasks = state.submitted + state.working.len() + state.completed + state.failed + state.canceled;
    let success_rate = if state.completed + state.failed > 0 {
        (state.completed as f64 / (state.completed + state.failed) as f64 * 100.0) as u32
    } else {
        0
    };

    let cost_items = vec![
        Line::from(format!("  Builds today    {}", state.daily_builds)),
        Line::from(format!("  Tokens today    {}", format_tokens(state.daily_tokens))),
        Line::from(format!("  Cost today      ${:.2}", state.daily_cost)),
        Line::from(format!("  Avg cost/build  ${:.2}", if state.daily_builds > 0 { state.daily_cost / state.daily_builds as f64 } else { 0.0 })),
        Line::from(""),
        Line::from(format!("  Total tasks     {}", total_tasks)),
        Line::from(format!("  Success rate    {}%", success_rate)),
        Line::from(format!("  Ledger events   {}", state.total_events)),
    ];
    let cost = Paragraph::new(cost_items)
        .block(Block::default().borders(Borders::ALL).title(" Metrics "));
    f.render_widget(cost, middle[1]);

    // Right column: Recent events
    let event_lines: Vec<Line> = state.recent_events.iter()
        .map(|e| {
            let style = if e.contains("completed") {
                Style::default().fg(Color::Green)
            } else if e.contains("failed") {
                Style::default().fg(Color::Red)
            } else if e.contains("bug") {
                Style::default().fg(Color::Yellow)
            } else {
                Style::default()
            };
            Line::from(Span::styled(format!("  {}", e), style))
        })
        .collect();

    let events = Paragraph::new(event_lines)
        .block(Block::default().borders(Borders::ALL).title(" Recent Events (newest first) "));
    f.render_widget(events, body[2]);

    // Footer
    let footer = Paragraph::new(Line::from(vec![
        Span::styled("  q", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(": quit   "),
        Span::raw(format!("refreshes every 2s   {}", chrono::Utc::now().format("%Y-%m-%d %H:%M:%S UTC"))),
    ]))
    .block(Block::default().borders(Borders::ALL));
    f.render_widget(footer, main_layout[2]);
}

fn format_tokens(tokens: u64) -> String {
    if tokens >= 1_000_000 {
        format!("{:.1}M", tokens as f64 / 1_000_000.0)
    } else if tokens >= 1_000 {
        format!("{:.1}K", tokens as f64 / 1_000.0)
    } else {
        format!("{}", tokens)
    }
}
