//! Base HTML layout for the rusno web UI.
//!
//! Every page renders through [`base`], which provides the document shell:
//! `<head>` with the CDN scripts and CSRF meta tag, the sidebar, and a
//! boosted `<body>`. [`nav`] is also exposed on its own so HTMX endpoints
//! can swap in just the sidebar as a partial.
//!
//! The base layout owns all the *global* styling: the app shell (sidebar +
//! main), cards, tables, stat cards, progress bars, buttons, status pills,
//! and — importantly — **every form control** (`input`, `select`, `textarea`,
//! checkbox, radio) is restyled here so no page ever renders a
//! browser-default widget on the dark theme. Page templates can add scoped
//! extras but should not redefine these controls.

use maud::{html, Markup, PreEscaped};

/// Nav items: `(tab id, path, label, icon)`.
const TABS: [(&str, &str, &str, &str); 4] = [
    ("dashboard", "/", "Dashboard", "📊"),
    ("projects", "/projects", "Projects", "📦"),
    ("deployments", "/deployments", "Deployments", "🚀"),
    ("settings", "/settings", "Settings", "⚙️"),
];

/// Global inline styles for the app shell (dark theme) + all form controls.
///
/// Class reference for page templates:
/// - `card`            — content card
/// - `stat-grid` / `stat-card` / `stat-value` / `stat-label` — dashboard stats
/// - `progress` / `progress-bar` — progress bar (set `style="width: N%"` on
///   `progress-bar`)
/// - `btn btn-primary` / `btn btn-danger` — buttons
/// - `pill pill-<status>` — status pills (`succeeded`, `failed`, `queued`,
///   `pulling`, `building`, `starting`)
/// - `field`           — a labelled form control wrapper
///   (`<label class="field">Label <input ...></label>`)
/// - `field-label`      — standalone label text above a control
/// - `checkbox`         — a labelled checkbox/radio row
const CSS: &str = r#"
    *, *::before, *::after { box-sizing: border-box; margin: 0; padding: 0; }
    html, body { height: 100%; }
    body {
        background: #1a1a2e;
        color: #e0e0e0;
        font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto,
            "Helvetica Neue", Arial, sans-serif;
        line-height: 1.5;
        display: flex;
        min-height: 100vh;
    }
    a { color: inherit; text-decoration: none; }

    /* ---------- Sidebar ---------- */
    .sidebar {
        flex: 0 0 auto;
        width: 16rem;
        background: #16213e;
        border-right: 1px solid #0f3460;
        display: flex;
        flex-direction: column;
        gap: 0.25rem;
        padding: 1.1rem 0.75rem;
        position: sticky;
        top: 0;
        align-self: flex-start;
        height: 100vh;
        overflow-y: auto;
    }
    .sidebar .brand {
        display: flex;
        align-items: center;
        gap: 0.45rem;
        font-weight: 700;
        font-size: 1.15rem;
        padding: 0.25rem 0.55rem 1rem;
        margin-bottom: 0.5rem;
        border-bottom: 1px solid #0f3460;
    }
    .sidebar .brand .brand-dot {
        display: inline-block;
        width: 0.55rem; height: 0.55rem;
        border-radius: 999px;
        background: #5eb1ff;
        box-shadow: 0 0 8px rgba(94,177,255,0.6);
    }
    .sidebar .nav { display: flex; flex-direction: column; gap: 0.2rem; }
    .sidebar .nav-item {
        display: flex;
        align-items: center;
        gap: 0.65rem;
        padding: 0.55rem 0.7rem;
        border-radius: 8px;
        color: #c7cad9;
        font-size: 0.95rem;
        transition: background 0.15s ease, color 0.15s ease;
    }
    .sidebar .nav-item .icon { font-size: 1.05rem; line-height: 1; }
    .sidebar .nav-item:hover { background: #0f3460; color: #ffffff; }
    .sidebar .nav-item.active {
        background: #0f3460;
        color: #ffffff;
        font-weight: 600;
    }

    /* ---------- Main ---------- */
    .container {
        flex: 1 1 auto;
        min-width: 0;
        max-width: 72rem;
        margin: 0 auto;
        padding: 1.75rem 1.5rem;
    }

    /* ---------- Cards ---------- */
    .card {
        background: #16213e;
        border: 1px solid rgba(15, 52, 96, 0.6);
        border-radius: 10px;
        padding: 1.35rem;
        margin-bottom: 1.25rem;
    }

    /* ---------- Tables ---------- */
    table { width: 100%; border-collapse: collapse; }
    th, td { padding: 0.55rem 0.75rem; text-align: left; border-bottom: 1px solid #0f3460; }
    th {
        color: #9aa0b5;
        font-size: 0.8rem;
        font-weight: 600;
        text-transform: uppercase;
        letter-spacing: 0.04em;
    }

    /* ---------- Stat cards ---------- */
    .stat-grid {
        display: grid;
        grid-template-columns: repeat(4, 1fr);
        gap: 1rem;
        margin-bottom: 1.25rem;
    }
    .stat-card { background: #16213e; border: 1px solid rgba(15,52,96,0.6); border-radius: 8px; padding: 1rem 1.25rem; }
    .stat-value { font-size: 1.75rem; font-weight: 700; }
    .stat-label { color: #9aa0b5; font-size: 0.85rem; }

    /* ---------- Progress bars ---------- */
    .progress {
        background: rgba(224, 224, 224, 0.12);
        border-radius: 999px;
        height: 0.5rem;
        overflow: hidden;
    }
    .progress-bar {
        background: #16468f;
        height: 100%;
        border-radius: 999px;
        transition: width 0.3s ease;
    }

    /* ---------- Buttons ---------- */
    .btn, button {
        display: inline-flex;
        align-items: center;
        justify-content: center;
        gap: 0.4rem;
        border: 1px solid transparent;
        border-radius: 7px;
        padding: 0.5rem 1rem;
        font-size: 0.9rem;
        font-weight: 600;
        font-family: inherit;
        cursor: pointer;
        color: #ffffff;
        background: #0f3460;
        transition: background 0.15s ease, border-color 0.15s ease, box-shadow 0.15s ease;
    }
    .btn-primary, button { background: #0f3460; }
    .btn-primary:hover, button:hover { background: #16468f; }
    .btn-danger { background: #e74c3c; }
    .btn-danger:hover { background: #c0392b; }
    .btn:focus-visible, button:focus-visible {
        outline: none;
        box-shadow: 0 0 0 3px rgba(22, 70, 143, 0.45);
    }
    button[disabled] { opacity: 0.5; cursor: not-allowed; }

    /* ---------- Status pills ---------- */
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

    /* ---------- Form controls (global, no browser defaults) ----------
       Every text-ish input, textarea, and select gets a dark, custom look.
       Checkboxes and radios are fully redrawn (no native box/ring). */
    input[type="text"], input[type="url"], input[type="number"],
    input[type="password"], input[type="email"], input[type="search"],
    input[type="tel"], textarea, select {
        width: 100%;
        background: #0d0d1a;
        border: 1px solid #0f3460;
        border-radius: 7px;
        padding: 0.55rem 0.7rem;
        color: #e0e0e0;
        font-size: 0.92rem;
        font-family: inherit;
        line-height: 1.4;
        transition: border-color 0.15s ease, box-shadow 0.15s ease;
    }
    input::placeholder, textarea::placeholder { color: #6c7293; }
    input:focus, textarea:focus, select:focus {
        outline: none;
        border-color: #16468f;
        box-shadow: 0 0 0 3px rgba(22, 70, 143, 0.3);
    }
    textarea { resize: vertical; min-height: 6rem; }

    /* Custom select: hide native arrow, draw a chevron. */
    select {
        appearance: none;
        -webkit-appearance: none;
        -moz-appearance: none;
        cursor: pointer;
        background-image: url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='14' height='14' viewBox='0 0 14 14'%3E%3Cpath d='M3 5 L7 9 L11 5' stroke='%239aa0b5' stroke-width='1.6' fill='none' stroke-linecap='round' stroke-linejoin='round'/%3E%3C/svg%3E");
        background-repeat: no-repeat;
        background-position: right 0.7rem center;
        background-size: 0.85rem;
        padding-right: 2.1rem;
    }
    select option { background: #16213e; color: #e0e0e0; }

    /* Redrawn checkbox / radio — no native widget. */
    input[type="checkbox"], input[type="radio"] {
        appearance: none;
        -webkit-appearance: none;
        -moz-appearance: none;
        width: 1.15rem;
        height: 1.15rem;
        flex: 0 0 auto;
        border: 1px solid #2a3a63;
        background: #0d0d1a;
        border-radius: 4px;
        display: inline-grid;
        place-content: center;
        cursor: pointer;
        margin: 0 0.45rem 0 0;
        vertical-align: middle;
        position: relative;
        top: -0.06em;
        transition: background 0.15s ease, border-color 0.15s ease;
    }
    input[type="radio"] { border-radius: 999px; }
    input[type="checkbox"]:checked, input[type="radio"]:checked {
        background: #16468f;
        border-color: #16468f;
    }
    input[type="checkbox"]:checked::before {
        content: "";
        width: 0.6rem;
        height: 0.32rem;
        border-left: 2px solid #fff;
        border-bottom: 2px solid #fff;
        transform: rotate(-45deg) translate(0.04rem, -0.06rem);
    }
    input[type="radio"]:checked::before {
        content: "";
        width: 0.5rem;
        height: 0.5rem;
        border-radius: 999px;
        background: #fff;
    }
    input[type="checkbox"]:focus-visible, input[type="radio"]:focus-visible {
        box-shadow: 0 0 0 3px rgba(22, 70, 143, 0.3);
        outline: none;
    }
    input[type="hidden"] { display: none !important; }

    /* Field helpers for labelled controls.
       Usage: `<label class="field">Caption <input .../></label>` — the caption
       inherits the muted label color, the control sits below it. */
    .field { display: block; margin-bottom: 1rem; }
    label.field {
        display: block;
        margin-bottom: 1rem;
        font-size: 0.82rem;
        font-weight: 500;
        color: #9aa0b5;
    }
    label.field input, label.field select, label.field textarea {
        display: block;
        margin-top: 0.35rem;
        color: #e0e0e0;
    }
    label.field input[type="checkbox"], label.field input[type="radio"] {
        display: inline-grid;
        width: 1.15rem;
        margin: 0 0.45rem 0 0;
    }
    .checkbox {
        display: flex;
        align-items: center;
        margin-bottom: 1rem;
        cursor: pointer;
        font-size: 0.92rem;
        color: #e0e0e0;
    }

    /* ---------- Responsive: collapse sidebar to a top bar on narrow screens ---------- */
    @media (max-width: 720px) {
        body { flex-direction: column; }
        .sidebar {
            width: 100%;
            height: auto;
            position: relative;
            flex-direction: row;
            flex-wrap: wrap;
            align-items: center;
            padding: 0.5rem 0.75rem;
            gap: 0.5rem;
        }
        .sidebar .brand { padding: 0 0.5rem; margin: 0 0.75rem 0 0; border: none; }
        .sidebar .nav { flex-direction: row; flex-wrap: wrap; gap: 0.15rem; }
        .container { padding: 1.25rem 1rem; }
    }
"#;

/// Render a single nav item link, highlighted when `tab == active_tab`.
fn nav_link(tab: &str, href: &str, label: &str, icon: &str, active_tab: &str) -> Markup {
    let active = tab == active_tab;
    html! {
        a
            href=(href)
            class=(if active { "nav-item active" } else { "nav-item" })
            aria-current=[if active { Some("page") } else { None }]
            hx-boost="true"
        {
            span class="icon" { (icon) }
            span { (label) }
        }
    }
}

/// Render the sidebar.
///
/// `active_tab` is one of `"dashboard"`, `"projects"`, `"deployments"`,
/// `"settings"`, or `""` when no tab is active (e.g. on the login page).
/// Also usable standalone for HTMX partial swaps of the sidebar itself.
pub fn nav(active_tab: &str) -> Markup {
    html! {
        aside class="sidebar" {
            a class="brand" href="/" hx-boost="true" {
                span class="brand-dot" { }
                "rusno"
            }
            nav class="nav" {
                @for (tab, href, label, icon) in &TABS {
                    (nav_link(tab, href, label, icon, active_tab))
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
