//! Dashboard page + HTMX partial fragments.
//!
//! [`dashboard_page`] renders the full page through the base layout. The
//! remaining functions render self-contained fragments that the dashboard
//! polls every 5 s via HTMX:
//!
//! - [`stats_fragment`]      → `GET /dashboard/stats`
//! - [`telemetry_fragment`]  → `GET /dashboard/telemetry`
//! - [`recent_deploys_fragment`] → `GET /dashboard/recent-deploys`
//!
//! The stats and telemetry fragments carry their own `hx-get` / `hx-trigger`
//! / `hx-swap="outerHTML"` attributes so each poll response re-arms the next
//! one — the polling loop survives any number of swaps.

use maud::{html, Markup};

use crate::docker::{format_bytes, TelemetrySnapshot};
use crate::models::deploy::DeployWithProject;
use crate::templates::layout::base;

/// Full dashboard page wrapped in the base layout (`active_tab` is
/// `"dashboard"`): stat cards, telemetry bars, and the recent-deploys card.
pub fn dashboard_page(
    csrf: &str,
    project_count: usize,
    container_count: usize,
    telemetry: &TelemetrySnapshot,
    deploys: &[DeployWithProject],
) -> Markup {
    let content = html! {
        (stats_fragment(project_count, container_count, telemetry))
        (telemetry_fragment(telemetry))

        div class="card" {
            div style="display:flex; justify-content:space-between; align-items:center; margin-bottom:1rem;" {
                h2 style="font-size:1.1rem;" { "Recent deployments" }
                a href="/deployments" class="btn btn-primary" hx-boost="true" { "View all" }
            }
            table {
                thead {
                    tr {
                        th { "Project" }
                        th { "Commit" }
                        th { "Trigger" }
                        th { "Status" }
                        th { "Started" }
                    }
                }
                tbody
                    id="recent-deploys"
                    hx-get="/dashboard/recent-deploys"
                    hx-trigger="every 5s"
                    hx-swap="innerHTML"
                {
                    (recent_deploys_fragment(deploys))
                }
            }
        }
    };

    base("Dashboard", "dashboard", Some(csrf), content)
}

/// Four stat cards: projects registered, containers running, CPU usage,
/// memory usage. Self-re-arming via `hx-swap="outerHTML"` so the 5 s poll
/// keeps running after every swap.
pub fn stats_fragment(
    project_count: usize,
    container_count: usize,
    telemetry: &TelemetrySnapshot,
) -> Markup {
    html! {
        div
            class="stat-grid"
            id="stats"
            hx-get="/dashboard/stats"
            hx-trigger="every 5s"
            hx-swap="outerHTML"
        {
            div class="stat-card" {
                div class="stat-value" { (project_count) }
                div class="stat-label" { "Projects registered" }
            }
            div class="stat-card" {
                div class="stat-value" { (container_count) }
                div class="stat-label" { "Containers running" }
            }
            div class="stat-card" {
                div class="stat-value" { (telemetry.cpu_percent) "%" }
                div class="stat-label" { "CPU usage" }
            }
            div class="stat-card" {
                div class="stat-value" { (telemetry.mem_percent) "%" }
                div class="stat-label" { "Memory usage" }
            }
        }
    }
}

/// Three progress bars — CPU load, memory used/total, storage used/total
/// for the disk holding `projects_root` — with the numeric values shown
/// beneath each bar. Like the stats fragment it re-arms itself on swap.
pub fn telemetry_fragment(telemetry: &TelemetrySnapshot) -> Markup {
    html! {
        div
            class="card"
            id="telemetry"
            hx-get="/dashboard/telemetry"
            hx-trigger="every 5s"
            hx-swap="outerHTML"
        {
            h2 style="font-size:1.1rem; margin-bottom:1rem;" { "Host telemetry" }

            div id="telemetry-cpu" style="margin-bottom:1rem;" {
                div class="stat-label" style="margin-bottom:0.35rem;" {
                    "CPU load — "
                    (telemetry.cpu_percent)
                    "%"
                }
                div class="progress" {
                    div class="progress-bar" style=(format!("width: {}%", telemetry.cpu_percent.min(100))) { }
                }
            }

            div id="telemetry-memory" style="margin-bottom:1rem;" {
                div class="stat-label" style="margin-bottom:0.35rem;" {
                    "Memory — "
                    (format_bytes(telemetry.mem_used))
                    " / "
                    (format_bytes(telemetry.mem_total))
                    " ("
                    (telemetry.mem_percent)
                    "%)"
                }
                div class="progress" {
                    div class="progress-bar" style=(format!("width: {}%", telemetry.mem_percent.min(100))) { }
                }
            }

            div id="telemetry-storage" {
                div class="stat-label" style="margin-bottom:0.35rem;" {
                    "Storage — "
                    (format_bytes(telemetry.storage_used))
                    " / "
                    (format_bytes(telemetry.storage_total))
                    " ("
                    (telemetry.storage_percent)
                    "%)"
                }
                div class="progress" {
                    div class="progress-bar" style=(format!("width: {}%", telemetry.storage_percent.min(100))) { }
                }
            }
        }
    }
}

/// Table rows for the last few deploys, swapped into `#recent-deploys`
/// (`hx-swap="innerHTML"`). When there are no deploys yet, a single
/// full-width "No deployments yet" row is shown.
pub fn recent_deploys_fragment(deploys: &[DeployWithProject]) -> Markup {
    if deploys.is_empty() {
        return html! {
            tr {
                td colspan="5" style="text-align:center; color:#9aa0b5;" {
                    "No deployments yet"
                }
            }
        };
    }

    html! {
        @for d in deploys {
            tr {
                td { (d.project_name) }
                td { (short_sha(&d.commit_sha)) }
                td { (d.trigger) }
                td { (status_pill(&d.status)) }
                td { (relative_time(&d.started_at)) }
            }
        }
    }
}

// ---- helpers ----------------------------------------------------------

/// First 7 characters of a commit SHA, or `"—"` when missing.
fn short_sha(sha: &Option<String>) -> String {
    sha.as_deref()
        .and_then(|s| s.get(..7))
        .unwrap_or("—")
        .to_string()
}

/// Render a colored status pill for a deploy status string.
///
/// Colors follow the layout's `.pill-*` classes: succeeded/healthy = green,
/// failed = red, in-flight statuses (queued/pulling/building/starting) =
/// amber. Unknown statuses fall back to the amber style so the pill never
/// renders unstyled.
fn status_pill(status: &str) -> Markup {
    let class = match status {
        "succeeded" | "healthy" => "pill pill-succeeded",
        "failed" => "pill pill-failed",
        // queued / pulling / building / starting all share the amber style;
        // the generic class also covers any future/unknown status.
        _ => "pill pill-building",
    };
    html! {
        span class=(class) { (status) }
    }
}

/// Human-friendly relative time like `"2m ago"` for an RFC-3339 timestamp.
///
/// Falls back to the raw timestamp when it can't be parsed (or is in the
/// future) so a malformed value never blanks the row.
fn relative_time(ts: &str) -> String {
    let parsed = match chrono::DateTime::parse_from_rfc3339(ts) {
        Ok(dt) => dt.with_timezone(&chrono::Utc),
        Err(_) => return ts.to_string(),
    };

    let secs = chrono::Utc::now().signed_duration_since(parsed).num_seconds();
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
