//! Templates for the Deployments tab: paginated deploy list and deploy detail
//! (with live log polling and a raw log link).

use maud::{html, Markup};

use crate::models::deploy::{Deploy, DeployWithProject};
use crate::templates::layout::base;

/// Page size for the deployments list — kept in sync with the route handler.
const PER_PAGE: i64 = 25;

/// Inline styles for the build-log `<pre>` block (monospace, dark bg).
const LOG_STYLE: &str = "background:#0d0d1a; color:#e0e0e0; padding:1rem; border-radius:6px; font-family:'SF Mono',Menlo,Consolas,monospace; font-size:0.82rem; line-height:1.45; overflow:auto; max-height:36rem; white-space:pre-wrap; word-break:break-word;";

/// Full deployments page: a paginated table of deploys across all projects.
pub fn deployments_page(
    csrf: &str,
    deploys: &[DeployWithProject],
    page: u32,
    total: i64,
) -> Markup {
    let total_pages = ((total as f64) / (PER_PAGE as f64)).ceil() as u32;
    let total_pages = total_pages.max(1);
    let has_older = page < total_pages;
    let has_newer = page > 1;

    base(
        "Deployments",
        "deployments",
        Some(csrf),
        html! {
            h2 { "Deployments" }

            .card {
                @if deploys.is_empty() {
                    p style="color:#9aa0b5; padding:1rem 0;" { "No deployments yet" }
                } @else {
                    table {
                        thead {
                            tr {
                                th { "Project" }
                                th { "Commit" }
                                th { "Message" }
                                th { "Trigger" }
                                th { "Status" }
                                th { "Started" }
                            }
                        }
                        tbody {
                            @for d in deploys {
                                tr {
                                    td {
                                        a href=(format!("/deployments/{}", d.id)) hx-boost="true" {
                                            (d.project_name)
                                        }
                                    }
                                    td {
                                        code style="font-family:monospace;" {
                                            (commit_sha_short(d.commit_sha.as_deref()))
                                        }
                                    }
                                    td { (truncate(d.commit_msg.as_deref(), 50)) }
                                    td { (trigger_badge(&d.trigger)) }
                                    td { (status_pill(&d.status)) }
                                    td { (relative_time(&d.started_at)) }
                                }
                            }
                        }
                    }
                }
            }

            div style="display:flex; justify-content:space-between; align-items:center; padding:0 0.5rem;" {
                @if has_older {
                    a href=(format!("/deployments?page={}", page + 1))
                       hx-boost="true"
                       style="color:#9aa0b5;" {
                        "← older"
                    }
                } @else {
                    span style="color:#5a5a6e; opacity:0.5;" { "← older" }
                }

                span style="color:#9aa0b5; font-size:0.85rem;" {
                    "page " (page) " of " (total_pages)
                }

                @if has_newer {
                    a href=(format!("/deployments?page={}", page - 1))
                       hx-boost="true"
                       style="color:#9aa0b5;" {
                        "newer →"
                    }
                } @else {
                    span style="color:#5a5a6e; opacity:0.5;" { "newer →" }
                }
            }
        },
    )
}

/// Deploy detail page: header card + log view (live-polling while in flight).
pub fn deploy_detail_page(
    csrf: &str,
    deploy: &Deploy,
    project_name: &str,
    project_slug: &str,
) -> Markup {
    let in_flight = deploy.is_in_flight();
    let log_tail = deploy.log_tail.as_deref().unwrap_or("");

    base(
        "Deploy detail",
        "deployments",
        Some(csrf),
        html! {
            // Header card
            .card {
                div style="display:flex; justify-content:space-between; align-items:center; flex-wrap:wrap; gap:0.5rem; margin-bottom:0.75rem;" {
                    h2 style="margin:0;" {
                        a href=(format!("/projects/{}", project_slug)) hx-boost="true" {
                            (project_name)
                        }
                    }
                    (status_pill(&deploy.status))
                }

                div style="display:grid; grid-template-columns:auto 1fr; gap:0.35rem 1rem; font-size:0.92rem;" {
                    @if let Some(sha) = deploy.commit_sha.as_deref() {
                        span style="color:#9aa0b5;" { "Commit" }
                        span style="font-family:monospace;" { (sha) }
                    }
                    @if let Some(msg) = deploy.commit_msg.as_deref() {
                        span style="color:#9aa0b5;" { "Message" }
                        span { (msg) }
                    }
                    span style="color:#9aa0b5;" { "Trigger" }
                    span { (trigger_badge(&deploy.trigger)) }
                    @if deploy.is_rollback {
                        span style="color:#9aa0b5;" { "Type" }
                        span { "rollback" }
                    }
                    span style="color:#9aa0b5;" { "Started" }
                    span { (relative_time(&deploy.started_at)) }
                    @if let Some(finished) = deploy.finished_at.as_deref() {
                        span style="color:#9aa0b5;" { "Finished" }
                        span { (relative_time(finished)) }
                        span style="color:#9aa0b5;" { "Duration" }
                        span { (duration(&deploy.started_at, finished)) }
                    }
                    @if let Some(err) = deploy.error.as_deref() {
                        span style="color:#e74c3c;" { "Error" }
                        span style="color:#e74c3c; font-family:monospace; white-space:pre-wrap;" { (err) }
                    }
                }

                @if !project_slug.is_empty() && deploy.status == "succeeded" {
                    div style="margin-top:0.75rem;" {
                        form action=(format!("/projects/{}/rollback", project_slug))
                              hx-post=(format!("/projects/{}/rollback", project_slug))
                              hx-headers=(csrf_headers(csrf))
                              style="display:inline;" {
                            input type="hidden" name="deploy_id" value=(deploy.id);
                            button type="submit" class="btn btn-danger" {
                                "Rollback to this commit"
                            }
                        }
                    }
                }
            }

            // Log view
            .card {
                div style="display:flex; justify-content:space-between; align-items:center; margin-bottom:0.5rem;" {
                    h3 style="margin:0; font-size:1.05rem;" { "Build log" }
                    a href=(format!("/deployments/{}/log/full", deploy.id))
                       target="_blank"
                       rel="noopener"
                       style="font-size:0.85rem; color:#9aa0b5;" {
                        "View full log"
                    }
                }
                @if in_flight {
                    pre id="deploy-log"
                        hx-get=(format!("/deployments/{}/log", deploy.id))
                        hx-trigger="every 5s"
                        hx-swap="innerHTML"
                        style=(LOG_STYLE) {
                        (log_tail)
                    }
                } @else {
                    pre id="deploy-log" style=(LOG_STYLE) {
                        (log_tail)
                    }
                }
            }
        },
    )
}

/// HTMX partial: just the text inside the log `<pre>`, for the 5s poll swap.
/// The `id`/`hx-*` attributes live on the `<pre>` in the detail page; this
/// returns only its inner content, HTML-escaped.
pub fn deploy_log_fragment(log_tail: &str) -> Markup {
    html! {
        (log_tail)
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Render a colored status pill. Colors follow the layout CSS:
/// succeeded/healthy=green, failed=red, queued/pulling/building/starting=amber.
fn status_pill(status: &str) -> Markup {
    let class = match status {
        "succeeded" | "healthy" => "pill pill-succeeded",
        "failed" => "pill pill-failed",
        _ => "pill pill-queued",
    };
    html! {
        span class=(class) { (status) }
    }
}

/// A small badge for the deploy trigger (manual / webhook / auto-start).
fn trigger_badge(trigger: &str) -> Markup {
    html! {
        span style="font-size:0.8rem; color:#9aa0b5; background:rgba(15,52,96,0.4); padding:0.15rem 0.5rem; border-radius:4px;" {
            (trigger)
        }
    }
}

/// JSON string for the `hx-headers` attribute carrying the CSRF token.
fn csrf_headers(csrf: &str) -> String {
    format!("{{\"X-CSRF-Token\":\"{}\"}}", csrf)
}

/// Short commit SHA: first 7 characters, or `—` when absent.
fn commit_sha_short(sha: Option<&str>) -> String {
    sha.map(|s| s.chars().take(7).collect::<String>())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "—".to_string())
}

/// Truncate a string to `max` chars, appending an ellipsis when cut.
fn truncate(s: Option<&str>, max: usize) -> String {
    let s = s.unwrap_or("—");
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let cut: String = s.chars().take(max.saturating_sub(1)).collect();
        format!("{cut}…")
    }
}

/// Human-relative time ("2m ago", "1h ago", "3d ago"). Falls back to the raw
/// string when the timestamp is unparseable.
fn relative_time(rfc3339: &str) -> String {
    use chrono::DateTime;
    let parsed = match DateTime::parse_from_rfc3339(rfc3339) {
        Ok(t) => t.with_timezone(&chrono::Utc),
        Err(_) => return rfc3339.to_string(),
    };
    let now = chrono::Utc::now();
    let dur = now.signed_duration_since(parsed);
    let secs = dur.num_seconds();

    if secs < 0 {
        return rfc3339.to_string();
    }
    if secs < 60 {
        return format!("{secs}s ago");
    }
    let mins = dur.num_minutes();
    if mins < 60 {
        return format!("{mins}m ago");
    }
    let hours = dur.num_hours();
    if hours < 24 {
        return format!("{hours}h ago");
    }
    format!("{}d ago", dur.num_days())
}

/// Duration between two RFC 3339 timestamps, formatted as e.g. "1m 23s".
fn duration(start: &str, end: &str) -> String {
    use chrono::DateTime;
    let (Ok(s), Ok(e)) = (
        DateTime::parse_from_rfc3339(start),
        DateTime::parse_from_rfc3339(end),
    ) else {
        return "—".to_string();
    };
    let secs = e
        .with_timezone(&chrono::Utc)
        .signed_duration_since(s.with_timezone(&chrono::Utc))
        .num_seconds()
        .max(0);
    let (h, m, sec) = (secs / 3600, (secs % 3600) / 60, secs % 60);
    if h > 0 {
        format!("{h}h {m}m")
    } else if m > 0 {
        format!("{m}m {sec}s")
    } else {
        format!("{sec}s")
    }
}
