//! Sidebar HTMX partial: rusno-managed running containers.
//!
//! [`containers_fragment`] is returned by `GET /sidebar/containers` and
//! polled every 10 s from the sidebar layout. The fragment carries its own
//! `hx-get` / `hx-trigger` / `hx-swap="outerHTML"` so each refresh re-arms
//! the next poll — the same self-reloading pattern used by the dashboard's
//! stats and telemetry fragments. When Docker is unreachable or no rusno
//! containers are running, the fragment renders a muted "No running
//! containers" line rather than erroring the sidebar.

use bollard::models::ContainerSummary;
use maud::{html, Markup};

/// Render the sidebar containers list as a self-rearming HTMX fragment.
///
/// Each container is shown as a row: a green status dot (running state), the
/// container name (linked to its project page when the `rusno.project` label
/// is present), and the short image reference. The whole fragment reloads
/// itself every 10 s via `hx-swap="outerHTML"`.
pub fn containers_fragment(containers: &[ContainerSummary]) -> Markup {
    html! {
        div
            class="sidebar-containers"
            hx-get="/sidebar/containers"
            hx-trigger="every 10s"
            hx-swap="outerHTML"
        {
            div class="sidebar-section-head" { "Containers" }
            @if containers.is_empty() {
                div class="sidebar-muted" { "No running containers" }
            } @else {
                @for c in containers {
                    (container_row(c))
                }
            }
        }
    }
}

/// One container row: status dot + name (linked to project) + image.
///
/// Docker prefixes container names with `/`; we strip it for display. The
/// `rusno.project` label, when present, becomes a link to the project page
/// so the sidebar doubles as quick navigation into a running stack.
fn container_row(c: &ContainerSummary) -> Markup {
    // Docker prefixes container names with '/'; strip it for display.
    let name = c
        .names
        .as_ref()
        .and_then(|n| n.first())
        .map(|n| n.trim_start_matches('/').to_string())
        .unwrap_or_else(|| "—".to_string());

    let image = c.image.as_deref().unwrap_or("—");
    let state = c.state.as_deref().unwrap_or("");
    // rusno.project label → project slug → link to the project page.
    let slug = c
        .labels
        .as_ref()
        .and_then(|l| l.get("rusno.project"))
        .map(|s| s.as_str());

    let is_running = state == "running";

    html! {
        div class="sidebar-container-item" {
            span
                class=(if is_running { "sidebar-dot sidebar-dot-running" } else { "sidebar-dot" })
                { }
            span class="sidebar-container-name" {
                @if let Some(slug) = slug {
                    a href=(format!("/projects/{slug}")) hx-boost="true" { (name) }
                } @else {
                    (name)
                }
            }
            span class="sidebar-container-image" { (image) }
        }
    }
}
