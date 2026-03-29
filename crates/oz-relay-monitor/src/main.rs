// Copyright (c) 2026 OZ Global Inc.
// Licensed under the Apache License, Version 2.0.

//! oz-relay-monitor — TUI dashboard for the OZ Relay pipeline.
//!
//! Views:
//!   Dashboard (d) — overview of pipeline, metrics, events
//!   Ledger    (l) — full scrollable event log
//!   Failed    (f) — details of failed tasks
//!   Submitted (s) — submitted intents and their descriptions
//!   Working   (w) — active builds in progress
//!   Completed (c) — completed builds with reports
//!   Bugs      (b) — incoming bug reports
//!
//! Press the key to switch views. Esc/Backspace returns to dashboard.
//! Arrow keys scroll in detail views. q quits.

use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crossterm::event::{self, Event, KeyCode, KeyModifiers};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use crossterm::ExecutableCommand;
use ratatui::prelude::*;
use ratatui::widgets::*;

#[derive(Clone, PartialEq)]
enum View {
    Dashboard,
    Ledger,
    EventDetail,
    TaskList(String),
    TaskDetail(String), // dir name — inspecting a task
    Bugs,
    BugDetail,
}

struct App {
    view: View,
    scroll: u16,
    /// Cursor position in list views (ledger, bugs, tasks).
    cursor: usize,
    /// Cached ledger lines (raw JSON) for inspection.
    ledger_lines: Vec<String>,
    /// Cached file paths for current list view (bugs/tasks).
    list_files: Vec<PathBuf>,
    data_dir: PathBuf,
}

fn main() -> io::Result<()> {
    let data_dir = std::env::args()
        .position(|a| a == "--data-dir")
        .and_then(|i| std::env::args().nth(i + 1))
        .or_else(|| std::env::args().nth(1).filter(|a| !a.starts_with('-')))
        .unwrap_or_else(|| "/opt/oz-relay".into());

    enable_raw_mode()?;
    io::stdout().execute(EnterAlternateScreen)?;
    io::stdout().execute(crossterm::event::EnableMouseCapture)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;

    let mut app = App {
        view: View::Dashboard,
        scroll: 0,
        cursor: 0,
        ledger_lines: Vec::new(),
        list_files: Vec::new(),
        data_dir: PathBuf::from(data_dir),
    };

    let result = run_app(&mut terminal, &mut app);

    io::stdout().execute(crossterm::event::DisableMouseCapture)?;
    disable_raw_mode()?;
    io::stdout().execute(LeaveAlternateScreen)?;
    result
}

fn run_app(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
) -> io::Result<()> {
    let tick_rate = Duration::from_secs(2);
    let mut last_tick = Instant::now();

    loop {
        // Pre-compute state that draw needs
        if app.view == View::Ledger || app.view == View::EventDetail {
            app.ledger_lines = load_ledger_lines(&app.data_dir);
        }
        if app.view == View::Bugs || app.view == View::BugDetail {
            app.list_files = list_json_files(&app.data_dir.join("bugs/incoming"));
        }
        if let View::TaskList(ref dir) = app.view {
            app.list_files = list_json_files(&app.data_dir.join("tasks").join(dir));
        }
        if let View::TaskDetail(ref dir) = app.view {
            app.list_files = list_json_files(&app.data_dir.join("tasks").join(dir));
        }
        let app_ref = &*app;
        terminal.draw(|f| draw_view(f, app_ref))?;

        let timeout = tick_rate.saturating_sub(last_tick.elapsed());
        if event::poll(timeout)? {
            let ev = event::read()?;

            // Mouse scroll support
            if let Event::Mouse(mouse) = ev {
                match mouse.kind {
                    crossterm::event::MouseEventKind::ScrollDown => {
                        if has_cursor_nav(&app.view) {
                            let max = list_len(app);
                            app.cursor = (app.cursor + 3).min(max);
                            auto_scroll_cursor(app);
                        } else {
                            app.scroll = app.scroll.saturating_add(3);
                        }
                    }
                    crossterm::event::MouseEventKind::ScrollUp => {
                        if has_cursor_nav(&app.view) {
                            app.cursor = app.cursor.saturating_sub(3);
                            if (app.cursor as u16) < app.scroll {
                                app.scroll = app.cursor as u16;
                            }
                        } else {
                            app.scroll = app.scroll.saturating_sub(3);
                        }
                    }
                    _ => {}
                }
                continue;
            }

            let Event::Key(key) = ev else { continue };

            match key.code {
                    KeyCode::Char('q') => return Ok(()),
                    KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        return Ok(())
                    }
                    KeyCode::Char('d') => {
                        app.view = View::Dashboard;
                        app.scroll = 0;
                    }
                    KeyCode::Char('l') => {
                        app.view = View::Ledger;
                        app.scroll = 0;
                        app.cursor = 0;
                    }
                    KeyCode::Char('f') => {
                        app.view = View::TaskList("failed".into());
                        app.scroll = 0;
                        app.cursor = 0;
                    }
                    KeyCode::Char('s') => {
                        app.view = View::TaskList("submitted".into());
                        app.scroll = 0;
                        app.cursor = 0;
                    }
                    KeyCode::Char('w') => {
                        app.view = View::TaskList("working".into());
                        app.scroll = 0;
                        app.cursor = 0;
                    }
                    KeyCode::Char('c') => {
                        app.view = View::TaskList("completed".into());
                        app.scroll = 0;
                        app.cursor = 0;
                    }
                    KeyCode::Char('b') => {
                        app.view = View::Bugs;
                        app.scroll = 0;
                        app.cursor = 0;
                    }
                    KeyCode::Enter => {
                        match &app.view {
                            View::Ledger if !app.ledger_lines.is_empty() => {
                                app.view = View::EventDetail;
                                app.scroll = 0;
                            }
                            View::Bugs if !app.list_files.is_empty() => {
                                app.view = View::BugDetail;
                                app.scroll = 0;
                            }
                            View::TaskList(dir) if !app.list_files.is_empty() => {
                                app.view = View::TaskDetail(dir.clone());
                                app.scroll = 0;
                            }
                            _ => {}
                        }
                    }
                    KeyCode::Esc | KeyCode::Backspace => {
                        app.view = match &app.view {
                            View::EventDetail => View::Ledger,
                            View::BugDetail => View::Bugs,
                            View::TaskDetail(_) => {
                                // Go back to the task list we came from
                                // We can't easily recover the dir, so go to dashboard
                                View::Dashboard
                            }
                            _ => View::Dashboard,
                        };
                        app.scroll = 0;
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        if has_cursor_nav(&app.view) {
                            let max = list_len(app);
                            app.cursor = (app.cursor + 1).min(max);
                            auto_scroll_cursor(app);
                        } else {
                            app.scroll = app.scroll.saturating_add(1);
                        }
                    }
                    KeyCode::Up | KeyCode::Char('k') => {
                        if has_cursor_nav(&app.view) {
                            app.cursor = app.cursor.saturating_sub(1);
                            if (app.cursor as u16) < app.scroll {
                                app.scroll = app.cursor as u16;
                            }
                        } else {
                            app.scroll = app.scroll.saturating_sub(1);
                        }
                    }
                    KeyCode::PageDown => {
                        if has_cursor_nav(&app.view) {
                            let max = list_len(app);
                            app.cursor = (app.cursor + 20).min(max);
                            auto_scroll_cursor(app);
                        }
                        app.scroll = app.scroll.saturating_add(20);
                    }
                    KeyCode::PageUp => {
                        if has_cursor_nav(&app.view) {
                            app.cursor = app.cursor.saturating_sub(20);
                        }
                        app.scroll = app.scroll.saturating_sub(20);
                    }
                    _ => {}
                }
        }
        if last_tick.elapsed() >= tick_rate {
            last_tick = Instant::now();
        }
    }
}

fn has_cursor_nav(view: &View) -> bool {
    matches!(view, View::Ledger | View::Bugs | View::TaskList(_))
}

fn list_len(app: &App) -> usize {
    match &app.view {
        View::Ledger => app.ledger_lines.len().saturating_sub(1),
        View::Bugs | View::TaskList(_) => app.list_files.len().saturating_sub(1),
        _ => 0,
    }
}

fn auto_scroll_cursor(app: &mut App) {
    let visible_height = 20u16;
    if app.cursor as u16 >= app.scroll + visible_height {
        app.scroll = (app.cursor as u16).saturating_sub(visible_height - 1);
    }
}

fn list_json_files(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .map(|e| e.path())
        .collect();
    files.sort_by(|a, b| b.cmp(a)); // newest first by filename
    files
}

fn load_ledger_lines(data_dir: &Path) -> Vec<String> {
    let path = data_dir.join("ledger/events.jsonl");
    std::fs::read_to_string(&path)
        .unwrap_or_default()
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.to_string())
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect()
}

fn draw_view(f: &mut Frame, app: &App) {
    match &app.view {
        View::Dashboard => draw_dashboard(f, app),
        View::Ledger => draw_ledger(f, app),
        View::EventDetail => draw_event_detail(f, app),
        View::TaskList(dir) => draw_task_list(f, app, dir),
        View::TaskDetail(dir) => draw_file_detail(f, app, &app.data_dir.join("tasks").join(dir)),
        View::Bugs => draw_bugs(f, app),
        View::BugDetail => draw_file_detail(f, app, &app.data_dir.join("bugs/incoming")),
    }
}

// ---------------------------------------------------------------------------
// Dashboard view
// ---------------------------------------------------------------------------

struct DashboardState {
    submitted: usize,
    working: Vec<TaskSummary>,
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

struct TaskSummary {
    id: String,
    developer: String,
    description: String,
}

fn count_json(dir: &Path) -> usize {
    std::fs::read_dir(dir)
        .map(|e| {
            e.filter_map(|e| e.ok())
                .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
                .count()
        })
        .unwrap_or(0)
}

fn scan_tasks(dir: &Path) -> Vec<TaskSummary> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.filter_map(|e| e.ok()) {
        let Ok(content) = std::fs::read_to_string(entry.path()) else {
            continue;
        };
        let Ok(val) = serde_json::from_str::<serde_json::Value>(&content) else {
            continue;
        };
        out.push(TaskSummary {
            id: val
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or("?")
                .chars()
                .take(8)
                .collect(),
            developer: val
                .get("owner")
                .and_then(|v| v.as_str())
                .unwrap_or("?")
                .into(),
            description: val
                .get("messages")
                .and_then(|m| m.as_array())
                .and_then(|a| a.first())
                .and_then(|m| m.get("parts"))
                .and_then(|p| p.as_array())
                .and_then(|a| a.first())
                .and_then(|p| p.get("data"))
                .and_then(|d| d.get("description"))
                .and_then(|d| d.as_str())
                .unwrap_or("(unknown)")
                .chars()
                .take(60)
                .collect(),
        });
    }
    out
}

fn scan_dashboard(data_dir: &Path) -> DashboardState {
    let tasks = data_dir.join("tasks");
    let promos = data_dir.join("promotions");
    let bugs = data_dir.join("bugs");
    let ledger = data_dir.join("ledger/events.jsonl");

    let mut recent = Vec::new();
    let mut total = 0;
    let mut daily_cost = 0.0;
    let mut daily_tokens = 0u64;
    let mut daily_builds = 0usize;
    let today = chrono::Utc::now().format("%Y-%m-%d").to_string();

    // Build owner map: task_id → developer name
    let mut owner_map: std::collections::HashMap<String, String> = std::collections::HashMap::new();

    if let Ok(content) = std::fs::read_to_string(&ledger) {
        // First pass: build owner map
        for line in content.lines() {
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(line) {
                if let (Some(tid), Some(owner)) = (
                    val.get("task_id").and_then(|v| v.as_str()),
                    val.get("owner").and_then(|v| v.as_str()),
                ) {
                    if !owner.is_empty() {
                        owner_map.insert(tid.to_string(), owner.to_string());
                    }
                }
            }
        }

        // Second pass: build event lines with resolved owners
        for line in content.lines() {
            if line.trim().is_empty() {
                continue;
            }
            total += 1;
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(line) {
                let ts = val.get("ts").and_then(|v| v.as_str()).unwrap_or("");
                let event = val.get("event").and_then(|v| v.as_str()).unwrap_or("");
                let tid = val.get("task_id").and_then(|v| v.as_str()).unwrap_or("");
                let bug_id = val.get("bug_id").and_then(|v| v.as_str()).unwrap_or("");

                // Resolve owner: from event itself, or from owner_map by task_id
                let owner = val.get("owner").and_then(|v| v.as_str())
                    .or_else(|| owner_map.get(tid).map(|s| s.as_str()))
                    .unwrap_or("");

                // Pick the right ID to show
                let id_str = if !tid.is_empty() {
                    &tid[..tid.len().min(8)]
                } else if !bug_id.is_empty() {
                    bug_id
                } else {
                    ""
                };

                // Build extra context based on event type
                let context = if !owner.is_empty() {
                    owner.to_string()
                } else if let Some(fp) = val.get("fingerprint").and_then(|v| v.as_str()) {
                    format!("fp:{}", &fp[..fp.len().min(8)])
                } else if let Some(ver) = val.get("arcflow_version").and_then(|v| v.as_str()) {
                    format!("v{}", ver)
                } else {
                    String::new()
                };

                // Add cost info for build events
                let cost_str = val.get("cost_usd")
                    .and_then(|v| v.as_str())
                    .map(|c| format!(" ${}", c))
                    .unwrap_or_default();

                // Add occurrences for bug duplicates
                let occ_str = val.get("occurrences")
                    .and_then(|v| v.as_u64())
                    .map(|n| format!(" (x{})", n))
                    .unwrap_or_default();

                recent.push(format!(
                    "{}  {:25}  {:10}  {}{}{}",
                    &ts[..ts.len().min(19)],
                    event,
                    id_str,
                    context,
                    cost_str,
                    occ_str,
                ));

                if ts.starts_with(&today) && event.starts_with("build.") {
                    daily_builds += 1;
                    if let Some(c) = val
                        .get("cost_usd")
                        .and_then(|v| v.as_str())
                        .and_then(|s| s.parse::<f64>().ok())
                    {
                        daily_cost += c;
                    }
                    if let Some(t) = val.get("total_tokens").and_then(|v| v.as_u64()) {
                        daily_tokens += t;
                    }
                }
            }
        }
    }
    recent.reverse();
    recent.truncate(20);

    DashboardState {
        submitted: count_json(&tasks.join("submitted")),
        working: scan_tasks(&tasks.join("working")),
        completed: count_json(&tasks.join("completed")),
        failed: count_json(&tasks.join("failed")),
        canceled: count_json(&tasks.join("canceled")),
        promo_pending: count_json(&promos.join("pending")),
        promo_approved: count_json(&promos.join("approved")),
        promo_merged: count_json(&promos.join("merged")),
        promo_rejected: count_json(&promos.join("rejected")),
        bugs_incoming: count_json(&bugs.join("incoming")),
        bugs_triaged: count_json(&bugs.join("triaged")),
        bugs_resolved: count_json(&bugs.join("resolved")),
        recent_events: recent,
        total_events: total,
        daily_cost,
        daily_tokens,
        daily_builds,
    }
}

fn draw_dashboard(f: &mut Frame, app: &App) {
    let state = scan_dashboard(&app.data_dir);
    let area = f.area();

    let layout = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(10),
        Constraint::Length(3),
    ])
    .split(area);

    // Header
    let header = Block::default()
        .borders(Borders::ALL)
        .title(format!(
            " OZ Relay Monitor — {} ",
            app.data_dir.display()
        ))
        .title_alignment(Alignment::Center);
    f.render_widget(header, layout[0]);

    // Body: 3 columns
    let body = Layout::horizontal([
        Constraint::Percentage(28),
        Constraint::Percentage(32),
        Constraint::Percentage(40),
    ])
    .split(layout[1]);

    // Left: Pipeline + Promotions + Bugs
    let left = Layout::vertical([
        Constraint::Length(9),
        Constraint::Length(8),
        Constraint::Min(6),
    ])
    .split(body[0]);

    let wc = state.working.len();
    let pipeline = Paragraph::new(vec![
        Line::from(vec![
            Span::styled("  submitted/  ", Style::default().fg(Color::Yellow)),
            Span::raw(format!("{}", state.submitted)),
        ]),
        Line::from(vec![
            Span::styled(
                "  working/    ",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("{}", wc),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(format!(" {}", "█".repeat(wc.min(10)))),
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
    ])
    .block(Block::default().borders(Borders::ALL).title(" Pipeline "));
    f.render_widget(pipeline, left[0]);

    let promos = Paragraph::new(vec![
        Line::from(format!("  pending/   {}", state.promo_pending)),
        Line::from(format!("  approved/  {}", state.promo_approved)),
        Line::from(format!("  merged/    {}", state.promo_merged)),
        Line::from(format!("  rejected/  {}", state.promo_rejected)),
    ])
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Promotions "),
    );
    f.render_widget(promos, left[1]);

    let bugs = Paragraph::new(vec![
        Line::from(format!("  incoming/  {}", state.bugs_incoming)),
        Line::from(format!("  triaged/   {}", state.bugs_triaged)),
        Line::from(format!("  resolved/  {}", state.bugs_resolved)),
    ])
    .block(Block::default().borders(Borders::ALL).title(" Bugs "));
    f.render_widget(bugs, left[2]);

    // Middle: Active builds + Metrics
    let mid = Layout::vertical([Constraint::Length(10), Constraint::Min(6)]).split(body[1]);

    let mut build_lines = Vec::new();
    if state.working.is_empty() {
        build_lines.push(Line::from(Span::styled(
            "  (no active builds)",
            Style::default().fg(Color::DarkGray),
        )));
    } else {
        for t in &state.working {
            build_lines.push(Line::from(vec![
                Span::styled(format!("  {} ", t.id), Style::default().fg(Color::Cyan)),
                Span::raw(&t.developer),
            ]));
            build_lines.push(Line::from(format!("    {}", t.description)));
            build_lines.push(Line::from(""));
        }
    }
    let active = Paragraph::new(build_lines)
        .block(Block::default().borders(Borders::ALL).title(" Active Builds "));
    f.render_widget(active, mid[0]);

    let total = state.submitted + wc + state.completed + state.failed + state.canceled;
    let rate = if state.completed + state.failed > 0 {
        (state.completed as f64 / (state.completed + state.failed) as f64 * 100.0) as u32
    } else {
        0
    };
    let metrics = Paragraph::new(vec![
        Line::from(format!("  Builds today    {}", state.daily_builds)),
        Line::from(format!(
            "  Tokens today    {}",
            fmt_tokens(state.daily_tokens)
        )),
        Line::from(format!("  Cost today      ${:.2}", state.daily_cost)),
        Line::from(format!(
            "  Avg cost/build  ${:.2}",
            if state.daily_builds > 0 {
                state.daily_cost / state.daily_builds as f64
            } else {
                0.0
            }
        )),
        Line::from(""),
        Line::from(format!("  Total tasks     {}", total)),
        Line::from(format!("  Success rate    {}%", rate)),
        Line::from(format!("  Ledger events   {}", state.total_events)),
    ])
    .block(Block::default().borders(Borders::ALL).title(" Metrics "));
    f.render_widget(metrics, mid[1]);

    // Right: Recent events
    let event_lines: Vec<Line> = state
        .recent_events
        .iter()
        .map(|e| {
            let style = if e.contains("completed") || e.contains("approved") {
                Style::default().fg(Color::Green)
            } else if e.contains("failed") || e.contains("rejected") {
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
        .block(Block::default().borders(Borders::ALL).title(" Recent Events "));
    f.render_widget(events, body[2]);

    // Footer
    let footer = Paragraph::new(Line::from(vec![
        Span::styled("  d", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw("ashboard "),
        Span::styled("l", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw("edger "),
        Span::styled("f", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw("ailed "),
        Span::styled("s", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw("ubmitted "),
        Span::styled("w", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw("orking "),
        Span::styled("c", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw("ompleted "),
        Span::styled("b", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw("ugs "),
        Span::styled("q", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw("uit   "),
        Span::styled(
            chrono::Utc::now().format("%H:%M:%S UTC").to_string(),
            Style::default().fg(Color::DarkGray),
        ),
    ]))
    .block(Block::default().borders(Borders::ALL));
    f.render_widget(footer, layout[2]);
}

// ---------------------------------------------------------------------------
// Ledger view — scrollable full event log
// ---------------------------------------------------------------------------

fn format_event_line(val: &serde_json::Value, owner_map: &std::collections::HashMap<String, String>) -> (String, Style) {
    let ts = val.get("ts").and_then(|v| v.as_str()).unwrap_or("");
    let event = val.get("event").and_then(|v| v.as_str()).unwrap_or("");
    let tid = val.get("task_id").and_then(|v| v.as_str()).unwrap_or("");
    let owner = val.get("owner").and_then(|v| v.as_str())
        .or_else(|| owner_map.get(tid).map(|s| s.as_str()))
        .unwrap_or("");

    let mut fields = Vec::new();
    if !owner.is_empty() {
        fields.push(format!("owner={}", owner));
    }
    if let Some(m) = val.as_object() {
        for (k, v) in m.iter() {
            if matches!(k.as_str(), "ts" | "event" | "owner") { continue; }
            fields.push(format!("{}={}", k, v.as_str().map(|s| s.to_string()).unwrap_or_else(|| v.to_string())));
        }
    }

    let style = if event.contains("completed") || event.contains("approved") {
        Style::default().fg(Color::Green)
    } else if event.contains("failed") || event.contains("rejected") {
        Style::default().fg(Color::Red)
    } else if event.contains("bug") {
        Style::default().fg(Color::Yellow)
    } else if event.contains("duplicate") {
        Style::default().fg(Color::DarkGray)
    } else {
        Style::default()
    };

    let text = format!("{}  {:25}  {}", &ts[..ts.len().min(19)], event, fields.join("  "));
    (text, style)
}

fn build_owner_map(lines: &[String]) -> std::collections::HashMap<String, String> {
    let mut map = std::collections::HashMap::new();
    for line in lines {
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(line) {
            if let (Some(tid), Some(owner)) = (
                val.get("task_id").and_then(|v| v.as_str()),
                val.get("owner").and_then(|v| v.as_str()),
            ) {
                if !owner.is_empty() {
                    map.insert(tid.to_string(), owner.to_string());
                }
            }
        }
    }
    map
}

fn draw_ledger(f: &mut Frame, app: &App) {
    let area = f.area();
    let layout = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(5),
        Constraint::Length(3),
    ]).split(area);

    let count = app.ledger_lines.len();
    let header = Block::default()
        .borders(Borders::ALL)
        .title(format!(" Ledger — {} events (newest first) — ↑↓ navigate, Enter to inspect ", count))
        .title_alignment(Alignment::Center);
    f.render_widget(header, layout[0]);

    let owner_map = build_owner_map(&app.ledger_lines);

    let lines: Vec<Line> = app.ledger_lines.iter().enumerate().map(|(i, raw)| {
        let is_selected = i == app.cursor;
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(raw) {
            let (text, mut style) = format_event_line(&val, &owner_map);
            if is_selected {
                style = style.bg(Color::DarkGray).add_modifier(Modifier::BOLD);
            }
            Line::from(Span::styled(format!("  {}", text), style))
        } else {
            let style = if is_selected {
                Style::default().bg(Color::DarkGray)
            } else {
                Style::default()
            };
            Line::from(Span::styled(format!("  {}", raw), style))
        }
    }).collect();

    let para = Paragraph::new(lines)
        .scroll((app.scroll, 0))
        .block(Block::default().borders(Borders::ALL));
    f.render_widget(para, layout[1]);

    let footer = Paragraph::new(Line::from(vec![
        Span::raw("  ↑↓ select   "),
        Span::styled("Enter", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(" inspect   "),
        Span::styled("Esc", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(" back   "),
        Span::styled("q", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(" quit"),
    ]))
    .block(Block::default().borders(Borders::ALL));
    f.render_widget(footer, layout[2]);
}

// ---------------------------------------------------------------------------
// Event detail view — full-screen inspection of a single event
// ---------------------------------------------------------------------------

fn draw_event_detail(f: &mut Frame, app: &App) {
    let area = f.area();
    let layout = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(5),
        Constraint::Length(3),
    ]).split(area);

    let header = Block::default()
        .borders(Borders::ALL)
        .title(" Event Detail ")
        .title_alignment(Alignment::Center);
    f.render_widget(header, layout[0]);

    let mut lines = Vec::new();

    if let Some(raw) = app.ledger_lines.get(app.cursor) {
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(raw) {
            let owner_map = build_owner_map(&app.ledger_lines);

            // Event header
            let event = val.get("event").and_then(|v| v.as_str()).unwrap_or("?");
            let ts = val.get("ts").and_then(|v| v.as_str()).unwrap_or("?");
            lines.push(Line::from(Span::styled(
                format!("  Event:     {}", event),
                Style::default().add_modifier(Modifier::BOLD),
            )));
            lines.push(Line::from(format!("  Timestamp: {}", ts)));
            lines.push(Line::from(""));

            // All fields
            lines.push(Line::from(Span::styled("  Fields:", Style::default().add_modifier(Modifier::BOLD))));
            if let Some(obj) = val.as_object() {
                for (k, v) in obj {
                    if k == "ts" || k == "event" { continue; }
                    let val_str = v.as_str().map(|s| s.to_string()).unwrap_or_else(|| {
                        serde_json::to_string_pretty(v).unwrap_or_else(|_| v.to_string())
                    });
                    lines.push(Line::from(format!("    {}: {}", k, val_str)));
                }
            }

            // Resolve owner
            let tid = val.get("task_id").and_then(|v| v.as_str()).unwrap_or("");
            let owner = val.get("owner").and_then(|v| v.as_str())
                .or_else(|| owner_map.get(tid).map(|s| s.as_str()));
            if let Some(o) = owner {
                if !val.as_object().map_or(false, |m| m.contains_key("owner")) {
                    lines.push(Line::from(format!("    owner: {} (resolved)", o)));
                }
            }

            // If this event has a task_id, try to load the full task file
            if !tid.is_empty() {
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled("  Associated Task:", Style::default().add_modifier(Modifier::BOLD))));

                let mut found = false;
                for dir in &["submitted", "working", "completed", "failed", "canceled"] {
                    let path = app.data_dir.join("tasks").join(dir).join(format!("{}.json", tid));
                    if let Ok(content) = std::fs::read_to_string(&path) {
                        found = true;
                        lines.push(Line::from(format!("    Location: tasks/{}/", dir)));

                        if let Ok(task) = serde_json::from_str::<serde_json::Value>(&content) {
                            let state = task.get("state").and_then(|v| v.as_str()).unwrap_or("?");
                            let owner = task.get("owner").and_then(|v| v.as_str()).unwrap_or("?");
                            lines.push(Line::from(format!("    State:    {}", state)));
                            lines.push(Line::from(format!("    Owner:    {}", owner)));

                            // Show intent description
                            if let Some(desc) = task.get("messages")
                                .and_then(|m| m.as_array())
                                .and_then(|a| a.first())
                                .and_then(|m| m.get("parts"))
                                .and_then(|p| p.as_array())
                                .and_then(|a| a.first())
                                .and_then(|p| p.get("data"))
                                .and_then(|d| d.get("description"))
                                .and_then(|d| d.as_str())
                            {
                                lines.push(Line::from(""));
                                lines.push(Line::from(Span::styled("  Intent:", Style::default().add_modifier(Modifier::BOLD))));
                                // Word-wrap the description
                                for chunk in desc.as_bytes().chunks(80) {
                                    if let Ok(s) = std::str::from_utf8(chunk) {
                                        lines.push(Line::from(format!("    {}", s)));
                                    }
                                }
                            }

                            // Show agent response if present
                            if let Some(msgs) = task.get("messages").and_then(|m| m.as_array()) {
                                for msg in msgs {
                                    if msg.get("role").and_then(|r| r.as_str()) == Some("agent") {
                                        lines.push(Line::from(""));
                                        lines.push(Line::from(Span::styled("  Agent Response:", Style::default().add_modifier(Modifier::BOLD))));
                                        if let Some(parts) = msg.get("parts").and_then(|p| p.as_array()) {
                                            for part in parts {
                                                if let Some(text) = part.get("text").and_then(|t| t.as_str()) {
                                                    for chunk in text.as_bytes().chunks(80) {
                                                        if let Ok(s) = std::str::from_utf8(chunk) {
                                                            lines.push(Line::from(format!("    {}", s)));
                                                        }
                                                    }
                                                }
                                                if let Some(data) = part.get("data") {
                                                    let pretty = serde_json::to_string_pretty(data).unwrap_or_default();
                                                    for pline in pretty.lines() {
                                                        lines.push(Line::from(format!("    {}", pline)));
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        break;
                    }
                }
                if !found {
                    lines.push(Line::from(Span::styled("    (task file not found on disk)", Style::default().fg(Color::DarkGray))));
                }
            }

            // If this event has a bug_id, try to load the bug file
            let bug_id = val.get("bug_id").and_then(|v| v.as_str()).unwrap_or("");
            if !bug_id.is_empty() {
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled("  Associated Bug:", Style::default().add_modifier(Modifier::BOLD))));

                let mut found = false;
                for dir in &["incoming", "triaged", "resolved"] {
                    let path = app.data_dir.join("bugs").join(dir).join(format!("{}.json", bug_id));
                    if let Ok(content) = std::fs::read_to_string(&path) {
                        found = true;
                        lines.push(Line::from(format!("    Location: bugs/{}/", dir)));

                        if let Ok(bug) = serde_json::from_str::<serde_json::Value>(&content) {
                            if let Some(err) = bug.get("report").and_then(|r| r.get("errorMessage")).and_then(|e| e.as_str()) {
                                lines.push(Line::from(format!("    Error: {}", err)));
                            }
                            if let Some(q) = bug.get("report").and_then(|r| r.get("query")).and_then(|q| q.as_str()) {
                                lines.push(Line::from(format!("    Query: {}", q)));
                            }
                            if let Some(occ) = bug.get("occurrences").and_then(|o| o.as_u64()) {
                                lines.push(Line::from(format!("    Occurrences: {}", occ)));
                            }
                            if let Some(ver) = bug.get("report").and_then(|r| r.get("arcflowVersion")).and_then(|v| v.as_str()) {
                                lines.push(Line::from(format!("    Version: {}", ver)));
                            }
                        }
                        break;
                    }
                }
                if !found {
                    lines.push(Line::from(Span::styled("    (bug file not found on disk)", Style::default().fg(Color::DarkGray))));
                }
            }

            // Raw JSON at the bottom
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled("  Raw JSON:", Style::default().fg(Color::DarkGray))));
            let pretty = serde_json::to_string_pretty(&val).unwrap_or_default();
            for pline in pretty.lines() {
                lines.push(Line::from(Span::styled(format!("    {}", pline), Style::default().fg(Color::DarkGray))));
            }
        }
    }

    let para = Paragraph::new(lines)
        .scroll((app.scroll, 0))
        .block(Block::default().borders(Borders::ALL));
    f.render_widget(para, layout[1]);

    let footer = Paragraph::new(Line::from(vec![
        Span::raw("  ↑↓ scroll   "),
        Span::styled("Esc", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(" back to ledger   "),
        Span::styled("q", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(" quit"),
    ]))
    .block(Block::default().borders(Borders::ALL));
    f.render_widget(footer, layout[2]);
}

// ---------------------------------------------------------------------------
// Task list view — show tasks from a specific directory
// ---------------------------------------------------------------------------

fn draw_task_list(f: &mut Frame, app: &App, dir_name: &str) {
    let area = f.area();
    let layout = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(5),
        Constraint::Length(3),
    ])
    .split(area);

    let title = format!(" tasks/{}/  ", dir_name);
    let color = match dir_name {
        "failed" => Color::Red,
        "completed" => Color::Green,
        "working" => Color::Cyan,
        "submitted" => Color::Yellow,
        _ => Color::White,
    };
    let header = Block::default()
        .borders(Borders::ALL)
        .title(Span::styled(title, Style::default().fg(color)))
        .title_alignment(Alignment::Center);
    f.render_widget(header, layout[0]);

    let task_dir = app.data_dir.join("tasks").join(dir_name);
    let mut lines = Vec::new();

    if let Ok(entries) = std::fs::read_dir(&task_dir) {
        let mut files: Vec<_> = entries.filter_map(|e| e.ok()).collect();
        files.sort_by_key(|e| std::cmp::Reverse(e.metadata().ok().and_then(|m| m.modified().ok())));

        for entry in files {
            if entry.path().extension().is_some_and(|x| x == "json") {
                if let Ok(content) = std::fs::read_to_string(entry.path()) {
                    if let Ok(val) = serde_json::from_str::<serde_json::Value>(&content) {
                        let id = val.get("id").and_then(|v| v.as_str()).unwrap_or("?");
                        let owner = val.get("owner").and_then(|v| v.as_str()).unwrap_or("?");
                        let state = val.get("state").and_then(|v| v.as_str()).unwrap_or("?");
                        let updated = val
                            .get("updatedAt")
                            .and_then(|v| v.as_str())
                            .unwrap_or("");

                        // Get description from intent
                        let desc = val
                            .get("messages")
                            .and_then(|m| m.as_array())
                            .and_then(|a| a.first())
                            .and_then(|m| m.get("parts"))
                            .and_then(|p| p.as_array())
                            .and_then(|a| a.first())
                            .and_then(|p| p.get("data"))
                            .and_then(|d| d.get("description"))
                            .and_then(|d| d.as_str())
                            .unwrap_or("(no description)");

                        // Get agent response if present
                        let agent_msg = val
                            .get("messages")
                            .and_then(|m| m.as_array())
                            .and_then(|a| {
                                a.iter().find(|m| {
                                    m.get("role").and_then(|r| r.as_str()) == Some("agent")
                                })
                            })
                            .and_then(|m| m.get("parts"))
                            .and_then(|p| p.as_array())
                            .and_then(|a| a.first());

                        let agent_text = agent_msg
                            .and_then(|p| p.get("text"))
                            .and_then(|t| t.as_str())
                            .unwrap_or("");
                        let build_report = agent_msg
                            .and_then(|p| p.get("data"))
                            .and_then(|d| d.get("summary"))
                            .and_then(|s| s.as_str())
                            .unwrap_or("");

                        lines.push(Line::from(Span::styled(
                            format!("  ┌─ {} ──── {} ──── {}", &id[..id.len().min(8)], owner, state),
                            Style::default().fg(color).add_modifier(Modifier::BOLD),
                        )));
                        lines.push(Line::from(format!(
                            "  │ Intent: {}",
                            &desc[..desc.len().min(80)]
                        )));
                        lines.push(Line::from(format!(
                            "  │ Updated: {}",
                            &updated[..updated.len().min(19)]
                        )));
                        if !agent_text.is_empty() {
                            lines.push(Line::from(Span::styled(
                                format!("  │ Agent: {}", &agent_text[..agent_text.len().min(80)]),
                                Style::default().fg(Color::DarkGray),
                            )));
                        }
                        if !build_report.is_empty() {
                            lines.push(Line::from(Span::styled(
                                format!(
                                    "  │ Report: {}",
                                    &build_report[..build_report.len().min(80)]
                                ),
                                Style::default().fg(Color::DarkGray),
                            )));
                        }
                        lines.push(Line::from("  └─"));
                        lines.push(Line::from(""));
                    }
                }
            }
        }
    }

    if lines.is_empty() {
        lines.push(Line::from(Span::styled(
            format!("  (no tasks in {}/)", dir_name),
            Style::default().fg(Color::DarkGray),
        )));
    }

    let para = Paragraph::new(lines)
        .scroll((app.scroll, 0))
        .block(Block::default().borders(Borders::ALL));
    f.render_widget(para, layout[1]);

    let footer = Paragraph::new(Line::from(vec![
        Span::raw("  ↑↓ scroll   "),
        Span::styled("Esc", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(" back   "),
        Span::styled("q", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(" quit"),
    ]))
    .block(Block::default().borders(Borders::ALL));
    f.render_widget(footer, layout[2]);
}

// ---------------------------------------------------------------------------
// Bugs view
// ---------------------------------------------------------------------------

fn draw_bugs(f: &mut Frame, app: &App) {
    let area = f.area();
    let layout = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(5),
        Constraint::Length(3),
    ]).split(area);

    let count = app.list_files.len();
    let header = Block::default()
        .borders(Borders::ALL)
        .title(format!(" Bugs — incoming/ ({}) — ↑↓ mouse/keys, Enter to inspect ", count))
        .title_alignment(Alignment::Center);
    f.render_widget(header, layout[0]);

    let mut lines = Vec::new();
    for (i, path) in app.list_files.iter().enumerate() {
        let is_selected = i == app.cursor;
        let bg = if is_selected { Color::DarkGray } else { Color::Reset };

        if let Ok(content) = std::fs::read_to_string(path) {
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(&content) {
                let id = val.get("id").and_then(|v| v.as_str()).unwrap_or("?");
                let error = val.get("report").and_then(|r| r.get("errorMessage")).and_then(|e| e.as_str()).unwrap_or("?");
                let version = val.get("report").and_then(|r| r.get("arcflowVersion")).and_then(|v| v.as_str()).unwrap_or("?");
                let occ = val.get("occurrences").and_then(|o| o.as_u64()).unwrap_or(1);
                let query = val.get("report").and_then(|r| r.get("query")).and_then(|q| q.as_str()).unwrap_or("");

                let occ_str = if occ > 1 { format!(" (x{})", occ) } else { String::new() };
                let style = Style::default().fg(Color::Yellow).bg(bg);
                let bold = if is_selected { style.add_modifier(Modifier::BOLD) } else { style };

                lines.push(Line::from(Span::styled(
                    format!("  {} │ v{}{} │ {}", id, version, occ_str, &error[..error.len().min(60)]),
                    bold,
                )));
                if !query.is_empty() {
                    lines.push(Line::from(Span::styled(
                        format!("  {:>28} │ query: {}", "", &query[..query.len().min(50)]),
                        Style::default().fg(Color::DarkGray).bg(bg),
                    )));
                }
            }
        }
    }

    if lines.is_empty() {
        lines.push(Line::from(Span::styled("  (no incoming bugs)", Style::default().fg(Color::DarkGray))));
    }

    let para = Paragraph::new(lines)
        .scroll((app.scroll, 0))
        .block(Block::default().borders(Borders::ALL));
    f.render_widget(para, layout[1]);

    let footer = Paragraph::new(Line::from(vec![
        Span::raw("  ↑↓/mouse scroll   "),
        Span::styled("Enter", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(" inspect   "),
        Span::styled("Esc", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(" back   "),
        Span::styled("q", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(" quit"),
    ]))
    .block(Block::default().borders(Borders::ALL));
    f.render_widget(footer, layout[2]);
}

// ---------------------------------------------------------------------------
// File detail view — full-screen JSON inspection of a bug or task
// ---------------------------------------------------------------------------

fn draw_file_detail(f: &mut Frame, app: &App, _dir: &Path) {
    let area = f.area();
    let layout = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(5),
        Constraint::Length(3),
    ]).split(area);

    let Some(path) = app.list_files.get(app.cursor) else {
        let empty = Paragraph::new("  (no file selected)")
            .block(Block::default().borders(Borders::ALL).title(" Detail "));
        f.render_widget(empty, area);
        return;
    };

    let filename = path.file_name().and_then(|f| f.to_str()).unwrap_or("?");

    let header = Block::default()
        .borders(Borders::ALL)
        .title(format!(" {} ", filename))
        .title_alignment(Alignment::Center);
    f.render_widget(header, layout[0]);

    let mut lines = Vec::new();
    if let Ok(content) = std::fs::read_to_string(path) {
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(&content) {
            // For bug reports: show structured fields first
            if let Some(report) = val.get("report") {
                let id = val.get("id").and_then(|v| v.as_str()).unwrap_or("?");
                let occ = val.get("occurrences").and_then(|o| o.as_u64()).unwrap_or(1);
                let fp = val.get("fingerprint").and_then(|v| v.as_str()).unwrap_or("");
                let received = val.get("receivedAt").and_then(|v| v.as_str()).unwrap_or("");
                let last_seen = val.get("lastSeenAt").and_then(|v| v.as_str()).unwrap_or("—");

                lines.push(Line::from(Span::styled("  Bug Report", Style::default().add_modifier(Modifier::BOLD).fg(Color::Yellow))));
                lines.push(Line::from(format!("  ID:          {}", id)));
                lines.push(Line::from(format!("  Occurrences: {}", occ)));
                lines.push(Line::from(format!("  Fingerprint: {}", fp)));
                lines.push(Line::from(format!("  Received:    {}", received)));
                lines.push(Line::from(format!("  Last seen:   {}", last_seen)));
                lines.push(Line::from(""));

                let err = report.get("errorMessage").and_then(|e| e.as_str()).unwrap_or("");
                let ver = report.get("arcflowVersion").and_then(|v| v.as_str()).unwrap_or("");
                let cat = report.get("category").and_then(|c| c.as_str()).unwrap_or("");
                let query = report.get("query").and_then(|q| q.as_str()).unwrap_or("");
                let trace = report.get("stackTrace").and_then(|t| t.as_str()).unwrap_or("");
                let ctx = report.get("context").and_then(|c| c.as_str()).unwrap_or("");
                let target = report.get("targetTriple").and_then(|t| t.as_str()).unwrap_or("");

                lines.push(Line::from(Span::styled("  Error Message:", Style::default().add_modifier(Modifier::BOLD))));
                for l in word_wrap(err, 80) {
                    lines.push(Line::from(format!("    {}", l)));
                }
                lines.push(Line::from(""));

                lines.push(Line::from(format!("  Version:     {}", ver)));
                lines.push(Line::from(format!("  Category:    {}", cat)));
                if !target.is_empty() {
                    lines.push(Line::from(format!("  Target:      {}", target)));
                }
                lines.push(Line::from(""));

                if !query.is_empty() {
                    lines.push(Line::from(Span::styled("  Query:", Style::default().add_modifier(Modifier::BOLD))));
                    lines.push(Line::from(format!("    {}", query)));
                    lines.push(Line::from(""));
                }

                if !trace.is_empty() {
                    lines.push(Line::from(Span::styled("  Stack Trace:", Style::default().add_modifier(Modifier::BOLD))));
                    for l in trace.lines() {
                        lines.push(Line::from(format!("    {}", l)));
                    }
                    lines.push(Line::from(""));
                }

                if !ctx.is_empty() {
                    lines.push(Line::from(Span::styled("  Context:", Style::default().add_modifier(Modifier::BOLD))));
                    for l in word_wrap(ctx, 80) {
                        lines.push(Line::from(format!("    {}", l)));
                    }
                    lines.push(Line::from(""));
                }
            } else {
                // For tasks: show structured fields
                let id = val.get("id").and_then(|v| v.as_str()).unwrap_or("?");
                let state = val.get("state").and_then(|v| v.as_str()).unwrap_or("?");
                let owner = val.get("owner").and_then(|v| v.as_str()).unwrap_or("?");

                lines.push(Line::from(Span::styled("  Task", Style::default().add_modifier(Modifier::BOLD).fg(Color::Cyan))));
                lines.push(Line::from(format!("  ID:    {}", id)));
                lines.push(Line::from(format!("  State: {}", state)));
                lines.push(Line::from(format!("  Owner: {}", owner)));
                lines.push(Line::from(""));

                // Intent
                if let Some(desc) = val.get("messages").and_then(|m| m.as_array()).and_then(|a| a.first())
                    .and_then(|m| m.get("parts")).and_then(|p| p.as_array()).and_then(|a| a.first())
                    .and_then(|p| p.get("data"))
                {
                    lines.push(Line::from(Span::styled("  Intent:", Style::default().add_modifier(Modifier::BOLD))));
                    if let Some(d) = desc.get("description").and_then(|d| d.as_str()) {
                        for l in word_wrap(d, 80) { lines.push(Line::from(format!("    {}", l))); }
                    }
                    if let Some(m) = desc.get("motivation").and_then(|m| m.as_str()) {
                        lines.push(Line::from(""));
                        lines.push(Line::from(Span::styled("  Motivation:", Style::default().add_modifier(Modifier::BOLD))));
                        for l in word_wrap(m, 80) { lines.push(Line::from(format!("    {}", l))); }
                    }
                    lines.push(Line::from(""));
                }

                // Agent responses
                if let Some(msgs) = val.get("messages").and_then(|m| m.as_array()) {
                    for msg in msgs {
                        if msg.get("role").and_then(|r| r.as_str()) == Some("agent") {
                            lines.push(Line::from(Span::styled("  Agent Response:", Style::default().add_modifier(Modifier::BOLD))));
                            if let Some(parts) = msg.get("parts").and_then(|p| p.as_array()) {
                                for part in parts {
                                    if let Some(text) = part.get("text").and_then(|t| t.as_str()) {
                                        for l in word_wrap(text, 80) { lines.push(Line::from(format!("    {}", l))); }
                                    }
                                    if let Some(data) = part.get("data") {
                                        let pretty = serde_json::to_string_pretty(data).unwrap_or_default();
                                        for l in pretty.lines() { lines.push(Line::from(format!("    {}", l))); }
                                    }
                                }
                            }
                            lines.push(Line::from(""));
                        }
                    }
                }
            }

            // Raw JSON at bottom
            lines.push(Line::from(Span::styled("  Raw JSON:", Style::default().fg(Color::DarkGray))));
            let pretty = serde_json::to_string_pretty(&val).unwrap_or_default();
            for l in pretty.lines() {
                lines.push(Line::from(Span::styled(format!("    {}", l), Style::default().fg(Color::DarkGray))));
            }
        } else {
            lines.push(Line::from(format!("  {}", content)));
        }
    }

    let para = Paragraph::new(lines)
        .scroll((app.scroll, 0))
        .block(Block::default().borders(Borders::ALL));
    f.render_widget(para, layout[1]);

    let footer = Paragraph::new(Line::from(vec![
        Span::raw("  ↑↓/mouse scroll   "),
        Span::styled("Esc", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(" back   "),
        Span::styled("q", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(" quit"),
    ]))
    .block(Block::default().borders(Borders::ALL));
    f.render_widget(footer, layout[2]);
}

fn word_wrap(text: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        if current.len() + word.len() + 1 > width && !current.is_empty() {
            lines.push(current);
            current = String::new();
        }
        if !current.is_empty() { current.push(' '); }
        current.push_str(word);
    }
    if !current.is_empty() { lines.push(current); }
    if lines.is_empty() { lines.push(String::new()); }
    lines
}

fn fmt_tokens(tokens: u64) -> String {
    if tokens >= 1_000_000 {
        format!("{:.1}M", tokens as f64 / 1_000_000.0)
    } else if tokens >= 1_000 {
        format!("{:.1}K", tokens as f64 / 1_000.0)
    } else {
        format!("{}", tokens)
    }
}
