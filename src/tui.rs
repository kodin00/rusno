//! Interactive terminal dashboard for rusno.
//!
//! When `rusno` is run with no subcommand, [`run`] enters a full-screen
//! terminal UI built on `ratatui` + `crossterm`. The dashboard mirrors the
//! web UI: a title bar with version + update banner, a stat row
//! (projects / containers / CPU / memory), a split view of running
//! containers and recent deployments, and a footer of key bindings.
//!
//! Data is pulled from the same sources the HTTP dashboard uses — the
//! SQLite database, the Docker daemon (via bollard), and sysinfo telemetry
//! — refreshed every 5 s and on-demand via `r`. Docker and update checks
//! degrade gracefully (empty list / no banner) on failure, so the TUI
//! never aborts over a transient backend issue.
//!
//! The terminal is always restored on exit (normal return, error, or
//! panic-driven unwind) via the [`TerminalGuard`] created at the top of
//! [`run`].

use std::io::stdout;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use bollard::models::ContainerSummary;
use crossterm::event::{self, Event, KeyCode, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};
use ratatui::{Frame, Terminal};
use tokio::time::MissedTickBehavior;

use crate::db::Db;
use crate::docker::{format_bytes, DockerClient, Telemetry, TelemetrySnapshot};
use crate::models::deploy::DeployWithProject;
use crate::models::project::Project;

/// Enter the interactive TUI dashboard. `rusno_home` is `~/.rusno`.
///
/// Restores the terminal on exit (even on panic/err) via a Drop guard.
pub async fn run(rusno_home: PathBuf) -> Result<()> {
    // --- Pre-flight: resolve data sources before touching the terminal ---
    // If rusno hasn't been initialized, surface a clear error *before*
    // entering raw mode so the message prints normally and the user's
    // terminal is never left in a weird state.
    if !rusno_home.exists() {
        anyhow::bail!(
            "rusno home not found at {}. Run `rusno init` first.",
            rusno_home.display()
        );
    }

    let cfg = crate::config::AppConfig::load(&rusno_home)
        .context("loading rusno config for TUI")?;
    let projects_root = Path::new(&cfg.projects_root).to_path_buf();
    let db = Db::connect(&rusno_home.join("rusno.db"))
        .await
        .context("connecting to rusno database for TUI")?;

    // --- Terminal setup ---
    // Raw mode + alternate screen let us paint a full-screen UI without
    // trashing the user's scrollback. The guard reverses both on drop —
    // including if a panic unwinds the stack past this point, so we never
    // strand the user in a broken terminal.
    enable_raw_mode().context("enabling raw mode")?;
    execute!(stdout(), EnterAlternateScreen).context("entering alternate screen")?;
    let _guard = TerminalGuard;

    let backend = CrosstermBackend::new(stdout());
    let mut terminal = Terminal::new(backend).context("initializing terminal")?;

    // --- Initial state ---
    let mut state = TuiState::empty();
    state.refresh(&db, &projects_root).await;
    state.update_version = check_update().await;

    // 5 s refresh cadence. The first tick of a tokio::time::interval fires
    // immediately, so consume it here to avoid a redundant refresh on the
    // first loop iteration (we just refreshed above). `Delay` prevents a
    // burst of catch-up ticks if a refresh ever runs long.
    let mut refresh_interval = tokio::time::interval(Duration::from_secs(5));
    refresh_interval.set_missed_tick_behavior(MissedTickBehavior::Delay);
    refresh_interval.tick().await;

    loop {
        terminal.draw(|f| draw(f, &state))?;

        // Wait on either a key event or the 5 s refresh tick — whichever
        // fires first. crossterm's `event::poll` is synchronous, so it runs
        // on the blocking pool via [`next_event`]. Cancelling that poll when
        // a refresh tick lands is harmless: the orphaned spawn_blocking
        // task finishes within 250 ms and its result is simply discarded.
        tokio::select! {
            ev = next_event() => {
                let ev = ev?;
                if let Some(Event::Key(k)) = ev {
                    match (k.code, k.modifiers) {
                        // q / Esc / Ctrl+C — quit.
                        (KeyCode::Char('q'), _) |
                        (KeyCode::Char('c'), KeyModifiers::CONTROL) |
                        (KeyCode::Esc, _) => return Ok(()),
                        // r — force a data refresh now.
                        (KeyCode::Char('r'), _) => {
                            state.refresh(&db, &projects_root).await;
                        }
                        // u — run the self-update flow. The updater may
                        // swap the binary and exit the process; either
                        // way we just return from `run` and the caller
                        // (main) exits.
                        (KeyCode::Char('u'), _) => {
                            tracing::info!("running self-update from TUI");
                            let _ = crate::update::run_update(true).await;
                            return Ok(());
                        }
                        // ? — toggle the extended help footer.
                        (KeyCode::Char('?'), _) => state.show_help = !state.show_help,
                        _ => {}
                    }
                }
            }
            _ = refresh_interval.tick() => {
                state.refresh(&db, &projects_root).await;
            }
        }
    }
}

/// Restores the terminal when dropped: disables raw mode and leaves the
/// alternate screen. Held as a guard in [`run`] so the terminal is
/// recovered even if the function returns early via `?` or unwinds.
struct TerminalGuard;

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(stdout(), LeaveAlternateScreen);
    }
}

// ============================================================
// State
// ============================================================

/// Cached snapshot of everything the TUI displays. Refreshed on demand
/// (key `r`) and every 5 s by the refresh interval.
struct TuiState {
    projects: Vec<Project>,
    deploys: Vec<DeployWithProject>,
    containers: Vec<ContainerSummary>,
    telemetry: TelemetrySnapshot,
    /// `Some(version)` only when a newer release than the running binary
    /// exists; drives the "update available" banner.
    update_version: Option<String>,
    show_help: bool,
    last_refresh: Instant,
}

impl TuiState {
    fn empty() -> Self {
        Self {
            projects: Vec::new(),
            deploys: Vec::new(),
            containers: Vec::new(),
            telemetry: TelemetrySnapshot {
                cpu_percent: 0,
                mem_used: 0,
                mem_total: 0,
                mem_percent: 0,
                storage_used: 0,
                storage_total: 0,
                storage_percent: 0,
            },
            update_version: None,
            show_help: false,
            last_refresh: Instant::now(),
        }
    }

    /// Re-fetch all data sources. Each source degrades independently: a
    /// Docker outage doesn't suppress the project list, a DB error doesn't
    /// hide telemetry, etc. This mirrors the HTTP dashboard's resilience
    /// (see `src/routes/dashboard.rs`).
    async fn refresh(&mut self, db: &Db, projects_root: &Path) {
        self.projects = db.list_projects().await.unwrap_or_else(|e| {
            tracing::warn!(error = %e, "TUI: listing projects failed");
            Vec::new()
        });

        self.deploys = db.list_recent_deploys(8).await.unwrap_or_else(|e| {
            tracing::warn!(error = %e, "TUI: listing recent deploys failed");
            Vec::new()
        });

        self.containers = match DockerClient::new().await {
            Ok(client) => client.list_rusno_containers().await.unwrap_or_else(|e| {
                tracing::debug!(error = %e, "TUI: listing containers failed");
                Vec::new()
            }),
            Err(e) => {
                // Docker daemon unreachable — common on a fresh host or a
                // dev machine without Docker running. Degrade silently.
                tracing::debug!(error = %e, "TUI: docker unavailable");
                Vec::new()
            }
        };

        let mut telemetry = Telemetry::new();
        self.telemetry = telemetry.snapshot(projects_root);
        self.last_refresh = Instant::now();
    }
}

// ============================================================
// Event polling
// ============================================================

/// Poll for a single crossterm event, waiting up to 250 ms. Runs on the
/// blocking thread pool because `crossterm::event::poll` is synchronous;
/// returning `None` means "no event within the window" (the caller just
/// loops), `Some` means an event was read.
async fn next_event() -> Result<Option<Event>> {
    let ev = tokio::task::spawn_blocking(|| -> std::io::Result<Option<Event>> {
        if event::poll(Duration::from_millis(250))? {
            Ok(Some(event::read()?))
        } else {
            Ok(None)
        }
    })
    .await??; // outer ? = JoinError, inner ? = io::Error
    Ok(ev)
}

/// Best-effort update check. Returns `Some(version)` only when a newer
/// release than the running binary exists; network failures degrade to
/// `None` (no banner) so the TUI never aborts over a transient network
/// blip.
async fn check_update() -> Option<String> {
    match crate::update::check_for_update().await {
        Ok(Some(info)) if info.version != env!("CARGO_PKG_VERSION") => Some(info.version),
        Ok(_) => None,
        Err(e) => {
            tracing::debug!(error = %e, "TUI: update check failed; suppressing banner");
            None
        }
    }
}

// ============================================================
// Rendering
// ============================================================

/// Top-level frame: title bar, stat row, main split, footer.
fn draw(frame: &mut Frame, state: &TuiState) {
    // The footer grows by one line when help is expanded so the layout
    // reflows cleanly instead of clipping the second hint line.
    let [title_area, stats_area, main_area, footer_area] = Layout::vertical([
        Constraint::Length(3), // title: 1 content line + bottom border
        Constraint::Length(4), // stat cards: border + 2 content lines
        Constraint::Min(0),    // containers | deploys — fills the rest
        Constraint::Length(if state.show_help { 2 } else { 1 }),
    ])
    .areas(frame.area());

    render_title(frame, title_area, state);
    render_stats(frame, stats_area, state);

    let [left, right] = Layout::horizontal([Constraint::Percentage(50); 2]).areas(main_area);
    render_containers(frame, left, state);
    render_deploys(frame, right, state);

    render_footer(frame, footer_area, state);
}

/// Row 0: "rusno" left, version right, optional "update available" banner.
fn render_title(frame: &mut Frame, area: Rect, state: &TuiState) {
    // A single bottom border separates the title from the stat row beneath.
    let block = Block::default().borders(Borders::BOTTOM);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let [left, right] = Layout::horizontal([Constraint::Min(0); 2]).areas(inner);

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                "rusno",
                Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("  ·  refreshed {}s ago", state.last_refresh.elapsed().as_secs()),
                Style::default().fg(Color::DarkGray),
            ),
        ])),
        left,
    );

    let mut right_spans = vec![Span::styled(
        format!("v{}", env!("CARGO_PKG_VERSION")),
        Style::default().fg(Color::DarkGray),
    )];
    if let Some(v) = &state.update_version {
        right_spans.push(Span::raw("  ·  "));
        right_spans.push(Span::styled(
            format!("update available: v{v} (press u)"),
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
        ));
    }
    frame.render_widget(
        Paragraph::new(Line::from(right_spans)).alignment(Alignment::Right),
        right,
    );
}

/// Row 1: four stat cards — Projects, Containers, CPU, Memory.
fn render_stats(frame: &mut Frame, area: Rect, state: &TuiState) {
    let [a, b, c, d] = Layout::horizontal([Constraint::Percentage(25); 4]).areas(area);

    frame.render_widget(
        stat_card(
            "Projects",
            state.projects.len().to_string(),
            Some("registered".to_string()),
            Color::Cyan,
        ),
        a,
    );
    frame.render_widget(
        stat_card(
            "Containers",
            state.containers.len().to_string(),
            Some("running".to_string()),
            Color::Green,
        ),
        b,
    );
    frame.render_widget(
        stat_card(
            "CPU",
            format!("{}%", state.telemetry.cpu_percent),
            Some("load".to_string()),
            Color::Yellow,
        ),
        c,
    );
    frame.render_widget(
        stat_card(
            "Memory",
            format!("{}%", state.telemetry.mem_percent),
            Some(format!(
                "{} / {}",
                format_bytes(state.telemetry.mem_used),
                format_bytes(state.telemetry.mem_total)
            )),
            Color::Yellow,
        ),
        d,
    );
}

/// One stat card: a bordered block titled `title` with a bold `value` and
/// an optional dim `sub` line beneath. All strings are moved into the
/// returned widget (Spans own them via `Cow::Owned`), so the card is
/// `'static` and can be rendered without holding borrows.
fn stat_card(title: &'static str, value: String, sub: Option<String>, color: Color) -> Paragraph<'static> {
    let mut lines = vec![Line::from(Span::styled(
        value,
        Style::default().fg(color).add_modifier(Modifier::BOLD),
    ))];
    if let Some(s) = sub {
        lines.push(Line::from(Span::styled(
            s,
            Style::default().fg(Color::DarkGray),
        )));
    }
    let block = Block::default()
        .borders(Borders::ALL)
        .title(Line::from(Span::styled(
            format!(" {title} "),
            Style::default().fg(color),
        )));
    Paragraph::new(lines)
        .alignment(Alignment::Center)
        .block(block)
}

/// Row 2 left: running rusno-managed containers (name | image | status).
fn render_containers(frame: &mut Frame, area: Rect, state: &TuiState) {
    let block = Block::default().borders(Borders::ALL).title(Line::from(Span::styled(
        " Running containers ",
        Style::default().fg(Color::Cyan),
    )));

    if state.containers.is_empty() {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "No running containers",
                Style::default().fg(Color::DarkGray),
            )))
            .alignment(Alignment::Center)
            .block(block),
            area,
        );
        return;
    }

    let items: Vec<ListItem> = state
        .containers
        .iter()
        .map(|c| {
            let name = container_name(c);
            let image = c.image.clone().unwrap_or_else(|| "—".to_string());
            // `status` is the human string ("Up 2 hours"); fall back to the
            // machine `state` ("running") when status is absent.
            let status = c
                .status
                .clone()
                .or_else(|| c.state.clone())
                .unwrap_or_else(|| "—".to_string());
            let col = container_status_color(&status);
            ListItem::new(Line::from(vec![
                Span::styled(fixed(&name, 24), Style::default().fg(Color::Cyan)),
                Span::raw(" │ "),
                Span::styled(fixed(&image, 28), Style::default().fg(Color::Gray)),
                Span::raw(" │ "),
                Span::styled(status, Style::default().fg(col)),
            ]))
        })
        .collect();

    frame.render_widget(List::new(items).block(block), area);
}

/// Row 2 right: recent deployments (project | short_sha | status | reltime).
fn render_deploys(frame: &mut Frame, area: Rect, state: &TuiState) {
    let block = Block::default().borders(Borders::ALL).title(Line::from(Span::styled(
        " Recent deployments ",
        Style::default().fg(Color::Cyan),
    )));

    if state.deploys.is_empty() {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "No deployments yet",
                Style::default().fg(Color::DarkGray),
            )))
            .alignment(Alignment::Center)
            .block(block),
            area,
        );
        return;
    }

    let items: Vec<ListItem> = state
        .deploys
        .iter()
        .map(|d| {
            let sha = short_sha(&d.commit_sha);
            let col = status_color(&d.status);
            let rel = relative_time(&d.started_at);
            ListItem::new(Line::from(vec![
                Span::styled(fixed(&d.project_name, 18), Style::default().fg(Color::Cyan)),
                Span::raw(" │ "),
                Span::styled(fixed(&sha, 9), Style::default().fg(Color::Magenta)),
                Span::raw(" │ "),
                Span::styled(fixed(&d.status, 12), Style::default().fg(col)),
                Span::raw(" │ "),
                Span::styled(rel, Style::default().fg(Color::DarkGray)),
            ]))
        })
        .collect();

    frame.render_widget(List::new(items).block(block), area);
}

/// Row 3: key-binding hints; expands to two lines when help is toggled.
fn render_footer(frame: &mut Frame, area: Rect, state: &TuiState) {
    let hints = Line::from(vec![
        Span::raw(" "),
        Span::styled("q", Style::default().fg(Color::Yellow)),
        Span::raw(" quit  ·  "),
        Span::styled("r", Style::default().fg(Color::Yellow)),
        Span::raw(" refresh  ·  "),
        Span::styled("u", Style::default().fg(Color::Yellow)),
        Span::raw(" update  ·  "),
        Span::styled("?", Style::default().fg(Color::Yellow)),
        Span::raw(" help"),
    ]);

    let lines = if state.show_help {
        vec![
            hints,
            Line::from(Span::styled(
                " Esc / Ctrl+C also quits  ·  u swaps the binary and may exit  ·  r re-fetches now",
                Style::default().fg(Color::DarkGray),
            )),
        ]
    } else {
        vec![hints]
    };

    frame.render_widget(
        Paragraph::new(lines)
            .alignment(Alignment::Center)
            .style(Style::default().fg(Color::DarkGray)),
        area,
    );
}

// ============================================================
// Helpers (mirror src/templates/dashboard.rs where applicable)
// ============================================================

/// First 7 characters of a commit SHA, or `"—"` when missing. Mirrors
/// `src/templates/dashboard.rs::short_sha`.
fn short_sha(sha: &Option<String>) -> String {
    sha.as_deref()
        .and_then(|s| s.get(..7))
        .unwrap_or("—")
        .to_string()
}

/// Human-friendly relative time like `"2m ago"` for an RFC-3339 timestamp.
/// Falls back to the raw timestamp when it can't be parsed (or is in the
/// future) so a malformed value never blanks the row. Mirrors
/// `src/templates/dashboard.rs::relative_time`.
fn relative_time(ts: &str) -> String {
    let parsed = match chrono::DateTime::parse_from_rfc3339(ts) {
        Ok(dt) => dt.with_timezone(&chrono::Utc),
        Err(_) => return ts.to_string(),
    };

    let secs = chrono::Utc::now()
        .signed_duration_since(parsed)
        .num_seconds();
    if secs < 0 {
        return ts.to_string();
    }

    let (n, unit) = if secs < 60 {
        (secs, "s")
    } else if secs < 3600 {
        (secs / 60, "m")
    } else if secs < 86_400 {
        (secs / 3600, "h")
    } else if secs < 2_592_000 {
        (secs / 86_400, "d")
    } else {
        (secs / 2_592_000, "mo")
    };

    format!("{n}{unit} ago")
}

/// Deploy status → color, matching the web dashboard's pill classes:
/// succeeded/healthy = green, failed = red, in-flight/unknown = yellow.
fn status_color(status: &str) -> Color {
    match status {
        "succeeded" | "healthy" => Color::Green,
        "failed" => Color::Red,
        // queued / pulling / building / starting all share amber; the
        // catch-all also covers any future/unknown status so we never
        // render unstyled.
        _ => Color::Yellow,
    }
}

/// Container status string → color. Docker's `status` field is free-form
/// ("Up 2 hours", "Exited (0) 3 minutes ago"), so we match on substrings
/// rather than exact tokens.
fn container_status_color(status: &str) -> Color {
    let s = status.to_lowercase();
    if s.contains("running") || s.contains("healthy") || s.contains("up ") {
        Color::Green
    } else if s.contains("exited") || s.contains("dead") || s.contains("restart") {
        Color::Red
    } else {
        Color::Yellow
    }
}

/// First container name with its leading `/` stripped (Docker prefixes
/// container names with `/` in `ContainerSummary::names`). Falls back to
/// `"—"` when names are absent.
fn container_name(c: &ContainerSummary) -> String {
    c.names
        .as_ref()
        .and_then(|n| n.first())
        .map(|s| s.strip_prefix('/').unwrap_or(s).to_string())
        .unwrap_or_else(|| "—".to_string())
}

/// Left-align `s` into exactly `width` cells: take the first `width` chars,
/// pad with spaces if shorter. Keeps the pipe-separated columns of the
/// container / deploy lists visually aligned regardless of content length.
fn fixed(s: &str, width: usize) -> String {
    let mut out: String = s.chars().take(width).collect();
    while out.chars().count() < width {
        out.push(' ');
    }
    out
}
