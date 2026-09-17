//! Base HTML layout for the rusno web UI.
//!
//! Every page renders through [`base`], which provides the document shell:
//! `<head>` with the CDN scripts and CSRF meta tag, the sidebar, and a
//! boosted `<body>`. [`nav`] is also exposed on its own so HTMX endpoints
//! can swap in just the sidebar as a partial.
//!
//! The base layout owns all the *global* styling: the design tokens (CSS
//! custom properties on `:root`), the app shell (sidebar + main), cards,
//! tables, stat cards, progress bars, buttons, status pills, and —
//! importantly — **every form control** (`input`, `select`, `textarea`,
//! checkbox, radio) is restyled here so no page ever renders a
//! browser-default widget. Page templates can add scoped extras but should
//! reference the `--*` tokens instead of hardcoded colors.

use maud::{html, Markup, PreEscaped};

/// Inline SVG icon body (Lucide-style, 24x24 viewBox) for the dashboard tab.
const ICON_DASHBOARD: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><rect width="7" height="9" x="3" y="3" rx="1"/><rect width="7" height="5" x="14" y="3" rx="1"/><rect width="7" height="9" x="14" y="12" rx="1"/><rect width="7" height="5" x="3" y="16" rx="1"/></svg>"#;

/// Inline SVG icon body for the projects tab (boxed package).
const ICON_PROJECTS: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="m7.5 4.27 9 5.15"/><path d="M21 8a2 2 0 0 0-1-1.73l-7-4a2 2 0 0 0-2 0l-7 4A2 2 0 0 0 3 8v8a2 2 0 0 0 1 1.73l7 4a2 2 0 0 0 2 0l7-4A2 2 0 0 0 21 16Z"/><path d="m3.3 7 8.7 5 8.7-5"/><path d="M12 22V12"/></svg>"#;

/// Inline SVG icon body for the deployments tab (rocket).
const ICON_DEPLOYMENTS: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M4.5 16.5c-1.5 1.26-2 5-2 5s3.74-.5 5-2c.71-.84.7-2.13-.09-2.91a2.18 2.18 0 0 0-2.91-.09z"/><path d="m12 15-3-3a22 22 0 0 1 2-3.95A12.88 12.88 0 0 1 22 2c0 2.72-.78 7.5-6 11a22.35 22.35 0 0 1-4 2z"/><path d="M9 12H4s.55-3.03 2-4c1.62-1.08 5 0 5 0"/><path d="M12 15v5s3.03-.55 4-2c1.08-1.62 0-5 0-5"/></svg>"#;

/// Inline SVG icon body for the settings tab (sliders).
const ICON_SETTINGS: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><line x1="21" x2="14" y1="4" y2="4"/><line x1="10" x2="3" y1="4" y2="4"/><line x1="21" x2="12" y1="12" y2="12"/><line x1="8" x2="3" y1="12" y2="12"/><line x1="21" x2="16" y1="20" y2="20"/><line x1="12" x2="3" y1="20" y2="20"/><line x1="14" x2="14" y1="2" y2="6"/><line x1="8" x2="8" y1="10" y2="14"/><line x1="16" x2="16" y1="18" y2="22"/></svg>"#;

/// Nav items: `(tab id, path, label, icon svg)`.
const TABS: [(&str, &str, &str, &str); 4] = [
    ("dashboard", "/", "Dashboard", ICON_DASHBOARD),
    ("projects", "/projects", "Projects", ICON_PROJECTS),
    (
        "deployments",
        "/deployments",
        "Deployments",
        ICON_DEPLOYMENTS,
    ),
    ("settings", "/settings", "Settings", ICON_SETTINGS),
];

/// Global inline styles for the app shell (light theme) + all form controls.
///
/// Design tokens live on `:root` so page templates can reuse them in inline
/// styles via `var(--muted)` etc. instead of hardcoded colors.
///
/// Class reference for page templates:
/// - `card`            — content card
/// - `stat-grid` / `stat-card` / `stat-value` / `stat-label` — dashboard stats
/// - `progress` / `progress-bar` — progress bar (set `style="width: N%"` on
///   `progress-bar`)
/// - `btn btn-primary` / `btn btn-danger` — buttons (a bare `button` renders
///   as a neutral secondary button)
/// - `pill pill-<status>` — status pills (`succeeded`, `failed`, `queued`,
///   `pulling`, `building`, `starting`, `deploying`; anything else falls back
///   to a neutral gray)
/// - `field`           — a labelled form control wrapper
///   (`<label class="field">Label <input ...></label>`)
/// - `field-label`      — standalone label text above a control
/// - `checkbox`         — a labelled checkbox/radio row
const CSS: &str = r#"
    :root {
        --bg: #f6f7f9;
        --surface: #ffffff;
        --border: #e5e7eb;
        --border-strong: #d4d8de;
        --text: #171923;
        --muted: #5f6470;
        --faint: #9199a5;
        --accent: #2563eb;
        --accent-hover: #1d4ed8;
        --accent-soft: #eef4ff;
        --ring: rgba(37, 99, 235, 0.16);
        --danger: #dc2626;
        --danger-hover: #b91c1c;
        --danger-text: #b42318;
        --danger-soft: #fef3f2;
        --success-text: #067647;
        --success-soft: #ecfdf3;
        --warn-text: #b54708;
        --warn-soft: #fffaeb;
        --neutral-soft: #f2f4f7;
        --mono: ui-monospace, "SF Mono", SFMono-Regular, Menlo, Consolas,
            "Liberation Mono", monospace;
        --radius: 8px;
        --radius-sm: 6px;
    }

    *, *::before, *::after { box-sizing: border-box; margin: 0; padding: 0; }
    html, body { height: 100%; }
    body {
        background: var(--bg);
        color: var(--text);
        font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto,
            "Helvetica Neue", Arial, sans-serif;
        line-height: 1.5;
        display: flex;
        min-height: 100vh;
        -webkit-font-smoothing: antialiased;
    }
    a { color: inherit; text-decoration: none; }
    main a:not(.btn) { color: var(--accent); }
    main a:not(.btn):hover { text-decoration: underline; }
    code {
        font-family: var(--mono);
        font-size: 0.85em;
        background: var(--neutral-soft);
        border-radius: 4px;
        padding: 0.1em 0.4em;
    }
    hr { border: none; border-top: 1px solid var(--border); }

    /* ---------- Sidebar ---------- */
    .sidebar {
        flex: 0 0 auto;
        width: 15rem;
        background: var(--surface);
        border-right: 1px solid var(--border);
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
        gap: 0.5rem;
        font-weight: 700;
        font-size: 1.05rem;
        letter-spacing: -0.01em;
        color: var(--text);
        padding: 0.25rem 0.55rem 1rem;
        margin-bottom: 0.5rem;
        border-bottom: 1px solid var(--border);
    }
    .sidebar .brand .brand-dot {
        display: inline-block;
        width: 0.5rem; height: 0.5rem;
        border-radius: 999px;
        background: var(--accent);
    }
    .sidebar .nav { display: flex; flex-direction: column; gap: 0.15rem; }
    .sidebar .nav-item {
        display: flex;
        align-items: center;
        gap: 0.6rem;
        padding: 0.5rem 0.65rem;
        border-radius: var(--radius-sm);
        color: var(--muted);
        font-size: 0.9rem;
        font-weight: 500;
        transition: background 0.15s ease, color 0.15s ease;
    }
    .sidebar .nav-item .icon { display: flex; align-items: center; }
    .sidebar .nav-item:hover { background: var(--neutral-soft); color: var(--text); }
    .sidebar .nav-item.active {
        background: var(--accent-soft);
        color: var(--accent);
        font-weight: 600;
    }

    /* ---------- Sidebar: containers section ----------
       Sits below the nav with a top border separator. The wrapper in the
       layout loads /sidebar/containers via HTMX and re-polls; the returned
       fragment re-arms itself on each swap (see templates::sidebar). */
    .sidebar-section {
        margin-top: 0.75rem;
        padding-top: 0.75rem;
        border-top: 1px solid var(--border);
        display: flex;
        flex-direction: column;
        gap: 0.35rem;
        /* Cap height so a long container list never blows out the sidebar. */
        max-height: 16rem;
        overflow-y: auto;
    }
    .sidebar-section-head {
        font-size: 0.7rem;
        font-weight: 600;
        text-transform: uppercase;
        letter-spacing: 0.05em;
        color: var(--faint);
        padding: 0 0.55rem;
    }
    .sidebar-containers {
        display: flex;
        flex-direction: column;
        gap: 0.2rem;
    }
    .sidebar-container-item {
        display: flex;
        align-items: center;
        gap: 0.45rem;
        padding: 0.35rem 0.55rem;
        border-radius: var(--radius-sm);
        font-size: 0.82rem;
        color: var(--text);
        transition: background 0.15s ease;
    }
    .sidebar-container-item:hover { background: var(--neutral-soft); }
    .sidebar-container-name {
        flex: 1 1 auto;
        min-width: 0;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
    }
    .sidebar-container-name a { color: var(--accent); }
    .sidebar-container-image {
        flex: 0 0 auto;
        font-size: 0.72rem;
        color: var(--muted);
        max-width: 5rem;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
    }
    .sidebar-muted {
        padding: 0.35rem 0.55rem;
        font-size: 0.8rem;
        color: var(--muted);
    }
    .sidebar-dot {
        flex: 0 0 auto;
        width: 0.45rem;
        height: 0.45rem;
        border-radius: 999px;
        background: var(--border-strong);
    }
    .sidebar-dot-running { background: var(--success-text); }

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
        background: var(--surface);
        border: 1px solid var(--border);
        border-radius: var(--radius);
        box-shadow: 0 1px 2px rgba(16, 24, 40, 0.04);
        padding: 1.25rem;
        margin-bottom: 1.25rem;
    }

    /* ---------- Tables ---------- */
    table { width: 100%; border-collapse: collapse; }
    th, td { padding: 0.6rem 0.75rem; text-align: left; border-bottom: 1px solid var(--border); }
    tbody tr:last-child td { border-bottom: none; }
    tbody tr:hover { background: #fafbfc; }
    th {
        color: var(--muted);
        font-size: 0.75rem;
        font-weight: 600;
        text-transform: uppercase;
        letter-spacing: 0.05em;
    }

    /* ---------- Stat cards ---------- */
    .stat-grid {
        display: grid;
        grid-template-columns: repeat(4, 1fr);
        gap: 1rem;
        margin-bottom: 1.25rem;
    }
    .stat-card {
        background: var(--surface);
        border: 1px solid var(--border);
        border-radius: var(--radius);
        box-shadow: 0 1px 2px rgba(16, 24, 40, 0.04);
        padding: 1rem 1.25rem;
    }
    .stat-value { font-size: 1.6rem; font-weight: 700; letter-spacing: -0.02em; }
    .stat-label { color: var(--muted); font-size: 0.85rem; }

    /* ---------- Progress bars ---------- */
    .progress {
        background: var(--neutral-soft);
        border-radius: 999px;
        height: 0.45rem;
        overflow: hidden;
    }
    .progress-bar {
        background: var(--accent);
        height: 100%;
        border-radius: 999px;
        transition: width 0.3s ease;
    }

    /* ---------- Buttons ----------
       A bare <button> is a neutral secondary control; .btn-primary and
       .btn-danger layer the accent / destructive fills on top. */
    .btn, button {
        display: inline-flex;
        align-items: center;
        justify-content: center;
        gap: 0.4rem;
        border: 1px solid var(--border-strong);
        border-radius: var(--radius-sm);
        padding: 0.45rem 0.9rem;
        font-size: 0.875rem;
        font-weight: 500;
        line-height: 1.3;
        font-family: inherit;
        cursor: pointer;
        color: var(--text);
        background: var(--surface);
        transition: background 0.15s ease, border-color 0.15s ease,
            box-shadow 0.15s ease;
    }
    .btn:hover, button:hover { background: var(--neutral-soft); }
    .btn-primary {
        background: var(--accent);
        border-color: var(--accent);
        color: #ffffff;
        font-weight: 600;
    }
    .btn-primary:hover, .btn-primary:active {
        background: var(--accent-hover);
        border-color: var(--accent-hover);
    }
    .btn-danger {
        background: var(--danger);
        border-color: var(--danger);
        color: #ffffff;
        font-weight: 600;
    }
    .btn-danger:hover, .btn-danger:active {
        background: var(--danger-hover);
        border-color: var(--danger-hover);
    }
    .btn:focus-visible, button:focus-visible {
        outline: none;
        box-shadow: 0 0 0 3px var(--ring);
    }
    button[disabled] { opacity: 0.5; cursor: not-allowed; }

    /* ---------- Status pills ---------- */
    .pill {
        display: inline-block;
        padding: 0.15rem 0.6rem;
        border-radius: 999px;
        font-size: 0.75rem;
        font-weight: 600;
        background: var(--neutral-soft);
        color: var(--muted);
    }
    .pill-succeeded { background: var(--success-soft); color: var(--success-text); }
    .pill-failed { background: var(--danger-soft); color: var(--danger-text); }
    .pill-queued, .pill-pulling, .pill-building, .pill-starting,
    .pill-deploying {
        background: var(--warn-soft);
        color: var(--warn-text);
    }

    /* ---------- Form controls (global, no browser defaults) ----------
       Every text-ish input, textarea, and select gets a light, custom look.
       Checkboxes and radios are fully redrawn (no native box/ring). */
    input[type="text"], input[type="url"], input[type="number"],
    input[type="password"], input[type="email"], input[type="search"],
    input[type="tel"], textarea, select {
        width: 100%;
        background: var(--surface);
        border: 1px solid var(--border-strong);
        border-radius: var(--radius-sm);
        padding: 0.5rem 0.65rem;
        color: var(--text);
        font-size: 0.9rem;
        font-family: inherit;
        line-height: 1.4;
        transition: border-color 0.15s ease, box-shadow 0.15s ease;
    }
    input::placeholder, textarea::placeholder { color: var(--faint); }
    input:focus, textarea:focus, select:focus {
        outline: none;
        border-color: var(--accent);
        box-shadow: 0 0 0 3px var(--ring);
    }
    input:disabled, textarea:disabled, select:disabled {
        background: var(--neutral-soft);
        color: var(--muted);
    }
    textarea { resize: vertical; min-height: 6rem; }

    /* Custom select: hide native arrow, draw a chevron. */
    select {
        appearance: none;
        -webkit-appearance: none;
        -moz-appearance: none;
        cursor: pointer;
        background-image: url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='14' height='14' viewBox='0 0 14 14'%3E%3Cpath d='M3 5 L7 9 L11 5' stroke='%239199a5' stroke-width='1.6' fill='none' stroke-linecap='round' stroke-linejoin='round'/%3E%3C/svg%3E");
        background-repeat: no-repeat;
        background-position: right 0.7rem center;
        background-size: 0.85rem;
        padding-right: 2.1rem;
    }

    /* Redrawn checkbox / radio — no native widget. */
    input[type="checkbox"], input[type="radio"] {
        appearance: none;
        -webkit-appearance: none;
        -moz-appearance: none;
        width: 1.1rem;
        height: 1.1rem;
        flex: 0 0 auto;
        border: 1px solid var(--border-strong);
        background: var(--surface);
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
        background: var(--accent);
        border-color: var(--accent);
    }
    input[type="checkbox"]:checked::before {
        content: "";
        width: 0.58rem;
        height: 0.32rem;
        border-left: 2px solid #fff;
        border-bottom: 2px solid #fff;
        transform: rotate(-45deg) translate(0.04rem, -0.06rem);
    }
    input[type="radio"]:checked::before {
        content: "";
        width: 0.42rem;
        height: 0.42rem;
        border-radius: 999px;
        background: #fff;
    }
    input[type="checkbox"]:focus-visible, input[type="radio"]:focus-visible {
        box-shadow: 0 0 0 3px var(--ring);
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
        color: var(--muted);
    }
    label.field input, label.field select, label.field textarea {
        display: block;
        margin-top: 0.35rem;
        color: var(--text);
    }
    label.field input[type="checkbox"], label.field input[type="radio"] {
        display: inline-grid;
        width: 1.1rem;
        margin: 0 0.45rem 0 0;
    }
    .checkbox {
        display: flex;
        align-items: center;
        margin-bottom: 1rem;
        cursor: pointer;
        font-size: 0.9rem;
        color: var(--text);
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
        /* Hide the containers panel on mobile to keep the collapsed top bar clean. */
        .sidebar-section { display: none; }
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
            span class="icon" { (PreEscaped(icon)) }
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
            // Sidebar containers panel — only on authenticated pages (login/
            // setup pass an empty active_tab). Loads the fragment via HTMX
            // on page load and re-polls every 10 s; the fragment itself also
            // re-arms on each swap (see templates::sidebar::containers_fragment).
            @if !active_tab.is_empty() {
                div
                    class="sidebar-section"
                    id="sidebar-containers"
                    hx-get="/sidebar/containers"
                    hx-trigger="load, every 10s"
                    hx-swap="innerHTML"
                {
                    // Placeholder shown until the first load completes.
                    div class="sidebar-section-head" { "Containers" }
                    div class="sidebar-muted" { "Loading…" }
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
                // open redirect / CSRF bypass surface. Passed as a plain
                // string (NOT PreEscaped) so maud escapes the quotes — a raw
                // `"` would terminate the attribute and break the JSON.
                meta name="htmx-config" content=r#"{"selfRequestsOnly":true}"#;
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
