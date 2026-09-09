//! Base HTML layout for the rusno web UI.
//!
//! Every page renders through [`base`], which provides the document shell:
//! `<head>` with the CDN scripts and CSRF meta tag, the nav bar, and a
//! boosted `<body>`. [`nav`] is also exposed on its own so HTMX endpoints
//! can swap in just the nav bar as a partial.

use maud::{html, Markup, PreEscaped};

/// Nav tabs: `(tab id, path, label)`.
const TABS: [(&str, &str, &str); 4] = [
    ("dashboard", "/", "Dashboard"),
    ("projects", "/projects", "Projects"),
    ("deployments", "/deployments", "Deployments"),
    ("settings", "/settings", "Settings"),
];

/// Minimal inline styles for the app shell (dark theme).
///
/// Class reference for page templates:
/// - `card`            — content card
/// - `stat-grid` / `stat-card` / `stat-value` / `stat-label` — dashboard stats
/// - `progress` / `progress-bar` — progress bar (set `style="width: N%"` on
///   `progress-bar`)
/// - `btn btn-primary` / `btn btn-danger` — buttons
/// - `pill pill-<status>` — status pills (`succeeded`, `failed`, `queued`,
///   `pulling`, `building`, `starting`)
const CSS: &str = r#"
    *, *::before, *::after { box-sizing: border-box; margin: 0; padding: 0; }
    body {
        background: #1a1a2e;
        color: #e0e0e0;
        font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto,
            "Helvetica Neue", Arial, sans-serif;
        line-height: 1.5;
    }
    a { color: inherit; text-decoration: none; }

    /* Nav */
    .topnav {
        display: flex;
        align-items: center;
        gap: 0.25rem;
        background: #16213e;
        border-bottom: 1px solid #0f3460;
        padding: 0.5rem 1.25rem;
    }
    .topnav .brand { font-weight: 700; margin-right: 1rem; }
    .topnav .tabs { display: flex; gap: 0.25rem; }
    .topnav .tab { padding: 0.45rem 0.9rem; border-radius: 6px; color: #e0e0e0; }
    .topnav .tab:hover { background: #0f3460; }
    .topnav .tab.active { background: #0f3460; color: #ffffff; font-weight: 600; }

    /* Layout */
    .container { max-width: 72rem; margin: 0 auto; padding: 1.5rem 1.25rem; }

    /* Cards */
    .card {
        background: #16213e;
        border-radius: 8px;
        padding: 1.25rem;
        margin-bottom: 1.25rem;
    }

    /* Tables */
    table { width: 100%; border-collapse: collapse; }
    th, td { padding: 0.5rem 0.75rem; text-align: left; border-bottom: 1px solid #0f3460; }
    th {
        color: #9aa0b5;
        font-size: 0.8rem;
        font-weight: 600;
        text-transform: uppercase;
        letter-spacing: 0.04em;
    }

    /* Stat cards */
    .stat-grid {
        display: grid;
        grid-template-columns: repeat(4, 1fr);
        gap: 1rem;
        margin-bottom: 1.25rem;
    }
    .stat-card { background: #16213e; border-radius: 8px; padding: 1rem 1.25rem; }
    .stat-value { font-size: 1.75rem; font-weight: 700; }
    .stat-label { color: #9aa0b5; font-size: 0.85rem; }

    /* Progress bars */
    .progress {
        background: rgba(224, 224, 224, 0.12);
        border-radius: 999px;
        height: 0.5rem;
        overflow: hidden;
    }
    .progress-bar {
        background: #0f3460;
        height: 100%;
        border-radius: 999px;
        transition: width 0.3s ease;
    }

    /* Buttons */
    .btn, button {
        display: inline-block;
        border: none;
        border-radius: 6px;
        padding: 0.5rem 1rem;
        font-size: 0.9rem;
        font-weight: 600;
        cursor: pointer;
        color: #ffffff;
    }
    .btn-primary, button { background: #0f3460; }
    .btn-primary:hover, button:hover { background: #16468f; }
    .btn-danger { background: #e74c3c; }
    .btn-danger:hover { background: #c0392b; }

    /* Status pills */
    .pill {
        display: inline-block;
        padding: 0.15rem 0.65rem;
        border-radius: 999px;
        font-size: 0.8rem;
        font-weight: 600;
    }
    .pill-succeeded { background: rgba(46, 204, 113, 0.15); color: #2ecc71; }
    .pill-failed { background: rgba(231, 76, 60, 0.15); color: #e74c3c; }
    .pill-queued, .pill-pulling, .pill-building, .pill-starting {
        background: rgba(243, 156, 18, 0.15);
        color: #f39c12;
    }
"#;

/// Render a single nav tab link, highlighted when `tab == active_tab`.
fn nav_link(tab: &str, href: &str, label: &str, active_tab: &str) -> Markup {
    let active = tab == active_tab;
    html! {
        a
            href=(href)
            class=(if active { "tab active" } else { "tab" })
            aria-current=[if active { Some("page") } else { None }]
            hx-boost="true"
        {
            (label)
        }
    }
}

/// Render the nav bar.
///
/// `active_tab` is one of `"dashboard"`, `"projects"`, `"deployments"`,
/// `"settings"`, or `""` when no tab is active (e.g. on the login page).
/// Also usable standalone for HTMX partial swaps of the nav bar itself.
pub fn nav(active_tab: &str) -> Markup {
    html! {
        nav class="topnav" {
            a class="brand" href="/" hx-boost="true" { "rusno" }
            div class="tabs" {
                @for (tab, href, label) in &TABS {
                    (nav_link(tab, href, label, active_tab))
                }
            }
        }
    }
}

/// Render a full HTML page.
///
/// * `title` — contents of `<title>`.
/// * `active_tab` — which nav tab to highlight (see [`nav`]).
/// * `csrf_token` — `Some(token)` when authenticated; `None` on the login
///   and setup pages, in which case no CSRF meta tag is rendered.
/// * `content` — page-specific markup for the main content area.
pub fn base(title: &str, active_tab: &str, csrf_token: Option<&str>, content: Markup) -> Markup {
    html! {
        (PreEscaped("<!DOCTYPE html>"))
        html {
            head {
                meta charset="utf-8";
                meta name="viewport" content="width=device-width, initial-scale=1";
                title { (title) }
                @if let Some(token) = csrf_token {
                    meta name="csrf-token" content=(token);
                }
                // htmx only sends boosted/swapped requests to the same origin,
                // blocking cross-origin requests that would otherwise be an
                // open redirect / CSRF bypass surface.
                meta name="htmx-config" content=(PreEscaped(r#"{"selfRequestsOnly":true}"#));
                script src="https://unpkg.com/htmx.org@2.0.4" { }
                script src="https://unpkg.com/alpinejs@3.14.1/dist/cdn.min.js" defer { }
                // CodeMirror 6 bundles are loaded per-page by the templates
                // that need an editor, not here in the base layout.
                style { (PreEscaped(CSS)) }
            }
            body hx-boost="true" {
                (nav(active_tab))
                main class="container" {
                    (content)
                }
            }
        }
    }
}
