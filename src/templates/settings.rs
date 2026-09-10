//! Settings page templates: service config, SSH keys, GitHub token,
//! Docker maintenance, and admin account.
//!
//! All sections render inside [`base`] with `active_tab = "settings"`.
//! Fragment helpers (`ssh_key_fragment`, `docker_summary_fragment`,
//! `docker_prune_result`) are HTMX partials swapped into specific
//! targets by the settings routes.

use std::io::Write;

use maud::{html, Markup, PreEscaped};

use crate::config::AppConfig;
use crate::docker::{format_bytes, DiskUsageSummary};
use crate::templates::layout::base;

/// Settings-page-only style extras. The shared form controls
/// (`input`, `textarea`, `select`, checkbox, radio) are styled globally in
/// the base layout; this block only adds the settings-specific scaffolding
/// (label caption spacing, radio group, <pre> blocks, helper text).
const SETTINGS_CSS: &str = r#"
    label { display: block; margin-bottom: 0.25rem; font-size: 0.85rem; color: #9aa0b5; }
    .radio-label {
        display: flex; gap: 0.35rem; align-items: center;
        cursor: pointer; margin-bottom: 0; font-size: 0.9rem; color: #e0e0e0;
    }
    small { color: #6c7293; font-size: 0.8rem; display: block; margin-bottom: 0.75rem; }
    .muted { color: #9aa0b5; font-size: 0.85rem; }
    pre {
        background: #0a0a1e; border: 1px solid #0f3460; border-radius: 4px;
        padding: 0.75rem; overflow-x: auto; white-space: pre-wrap;
        word-break: break-word; margin-bottom: 0.75rem; font-size: 0.85rem;
    }
    h2 { margin-bottom: 1rem; font-size: 1.15rem; }
    .section-note { color: #6c7293; font-size: 0.85rem; margin-bottom: 1rem; }
    .radio-group { display: flex; gap: 1.5rem; margin-bottom: 1rem; }
    .btn-row { display: flex; gap: 0.5rem; margin-top: 0.5rem; align-items: center; }
    .fragment-msg { margin-top: 0.5rem; }
"#;

/// Inject the CSRF token into every HTMX request via the
/// `htmx:configRequest` event, reading the value from the
/// `<meta name="csrf-token">` tag rendered by the base layout.
/// This covers both boosted `<form>` submissions and explicit
/// `hx-post`/`hx-get` requests.
const CSRF_SCRIPT: &str = r#"
(function() {
    var meta = document.querySelector('meta[name="csrf-token"]');
    if (meta) {
        document.body.addEventListener('htmx:configRequest', function(evt) {
            evt.detail.headers['X-CSRF-Token'] = meta.content;
        });
    }
})();
"#;

/// Loaded settings values displayed on the page.
pub struct SettingsValues {
    pub rusno_url: String,
    pub projects_root: String,
    pub deploy_concurrency: String,
    pub default_health_timeout_secs: String,
    pub ssh_mode: String,
    pub has_github_token: bool,
}

/// A host SSH key discovered in `~/.ssh/` (host-existing mode).
struct HostKey {
    name: String,
    fingerprint: Option<String>,
}

/// Compute the SHA256 fingerprint of an OpenSSH public key by piping
/// it to `ssh-keygen -lf -`. Returns `None` if `ssh-keygen` is missing
/// or the key is malformed.
fn ssh_fingerprint(pubkey: &str) -> Option<String> {
    let mut child = std::process::Command::new("ssh-keygen")
        .args(["-lf", "-"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .ok()?;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(pubkey.as_bytes());
    }
    let output = child.wait_with_output().ok()?;
    if !output.status.success() {
        return None;
    }
    // `ssh-keygen -lf -` prints: `<bits> SHA256:<base64> <comment> (<type>)`
    String::from_utf8_lossy(&output.stdout)
        .split_whitespace()
        .nth(1)
        .map(|s| s.to_string())
}

/// Scan `~/.ssh/` for `id_*` private-key files with matching `.pub`
/// counterparts (host-existing mode). Returns the key name and
/// fingerprint for each found key.
fn scan_host_ssh_keys() -> Vec<HostKey> {
    let mut out = Vec::new();
    let Some(home) = dirs::home_dir() else {
        return out;
    };
    let ssh_dir = home.join(".ssh");
    let Ok(entries) = std::fs::read_dir(&ssh_dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        // Skip `.pub` files themselves and anything not starting with `id_`.
        if !name.starts_with("id_") || name.ends_with(".pub") {
            continue;
        }
        let pub_path = entry.path().with_file_name(format!("{name}.pub"));
        let Ok(pubkey) = std::fs::read_to_string(&pub_path) else {
            continue;
        };
        let fingerprint = ssh_fingerprint(&pubkey);
        out.push(HostKey { name, fingerprint });
    }
    out
}

/// Run `ssh-add -l` and return its output (list of agent-loaded keys,
/// or an error message if no agent / no identities).
fn ssh_agent_list() -> Option<String> {
    let output = std::process::Command::new("ssh-add")
        .arg("-l")
        .output()
        .ok()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let trimmed = stderr.trim();
        return if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        };
    }
    Some(String::from_utf8_lossy(&output.stdout).to_string())
}

/// Render the full settings page.
///
/// * `csrf` — CSRF token (rendered into a `<meta>` by the base layout;
///   also injected into HTMX requests by the inline script).
/// * `config` — bootstrap config for read-only port display.
/// * `settings` — loaded DB settings values.
/// * `ssh_pubkey` — rusno-managed public key, if present (only shown
///   in rusno-managed mode).
/// * `docker_df` — docker system df summary, if already loaded.
///   `None` on the initial page render; the Docker section auto-loads
///   via HTMX (`hx-trigger="load"`).
pub fn settings_page(
    csrf: &str,
    config: &AppConfig,
    settings: &SettingsValues,
    ssh_pubkey: Option<&str>,
    docker_df: Option<&DiskUsageSummary>,
) -> Markup {
    let host_keys = if settings.ssh_mode == "host-existing" {
        scan_host_ssh_keys()
    } else {
        Vec::new()
    };
    let agent_keys = if settings.ssh_mode == "host-existing" {
        ssh_agent_list()
    } else {
        None
    };

    base(
        "Settings",
        "settings",
        Some(csrf),
        html! {
            style { (PreEscaped(SETTINGS_CSS)) }
            script { (PreEscaped(CSRF_SCRIPT)) }

            (service_config_card(config, settings))
            (ssh_keys_card(settings, ssh_pubkey, &host_keys, agent_keys.as_deref()))
            (github_card(settings))
            (docker_card(docker_df))
            (admin_card())
        },
    )
}

// ---------------------------------------------------------------------------
// Section cards
// ---------------------------------------------------------------------------

/// Service configuration: rusno URL, port (read-only), projects root,
/// deploy concurrency, default health timeout.
fn service_config_card(config: &AppConfig, settings: &SettingsValues) -> Markup {
    html! {
        div class="card" {
            h2 { "Service configuration" }

            form method="post" action="/settings" {
                label { "rusno URL" }
                input type="text" name="rusno_url" value=(settings.rusno_url);

                label { "Port" }
                input type="text" value=(config.port) disabled="";
                small { "Change via config.toml or --port" }

                label { "Projects root" }
                input type="text" name="projects_root" value=(settings.projects_root);

                label { "Deploy concurrency" }
                input type="number" name="deploy_concurrency"
                    value=(settings.deploy_concurrency) min="1";

                label { "Default health timeout (seconds)" }
                input type="number" name="default_health_timeout_secs"
                    value=(settings.default_health_timeout_secs) min="1";

                button type="submit" class="btn btn-primary" { "Save" }
            }
        }
    }
}

/// SSH keys: mode toggle + rusno-managed pubkey or host-existing list.
fn ssh_keys_card(
    settings: &SettingsValues,
    ssh_pubkey: Option<&str>,
    host_keys: &[HostKey],
    agent_keys: Option<&str>,
) -> Markup {
    html! {
        div class="card" {
            h2 { "SSH keys" }

            // Mode toggle — boosted form, posts to /settings/ssh-mode.
            form method="post" action="/settings/ssh-mode" {
                div class="radio-group" {
                    label class="radio-label" {
                        input type="radio" name="mode" value="rusno-managed"
                            checked[settings.ssh_mode == "rusno-managed"];
                        " rusno-managed"
                    }
                    label class="radio-label" {
                        input type="radio" name="mode" value="host-existing"
                            checked[settings.ssh_mode == "host-existing"];
                        " host-existing"
                    }
                }
                button type="submit" class="btn btn-primary" { "Set mode" }
            }

            @if settings.ssh_mode == "rusno-managed" {
                @if let Some(pubkey) = ssh_pubkey {
                    (ssh_key_fragment(pubkey))
                } @else {
                    p class="section-note" {
                        "No rusno-managed key yet. Generate one below."
                    }
                    button class="btn btn-primary"
                        hx-post="/settings/ssh/generate"
                        hx-target="#ssh-key-result"
                        hx-swap="innerHTML"
                    { "Generate key" }
                    div id="ssh-key-result" { }
                }
            } @else {
                p class="section-note" {
                    "rusno will use the SSH keys already on the host "
                    "(~/.ssh/). Make sure ssh-agent is running and the "
                    "desired keys are loaded."
                }

                @if host_keys.is_empty() {
                    p class="muted" { "No keys found in ~/.ssh/" }
                } @else {
                    table {
                        thead {
                            tr {
                                th { "Key" }
                                th { "Fingerprint" }
                            }
                        }
                        tbody {
                            @for key in host_keys {
                                tr {
                                    td { (key.name) }
                                    td style="font-family:monospace; font-size:0.8rem;" {
                                        @if let Some(fp) = &key.fingerprint {
                                            (fp)
                                        } @else {
                                            span class="muted" { "unavailable" }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                @if let Some(agent) = agent_keys {
                    h2 style="margin-top:1rem; font-size:1rem;" { "ssh-agent" }
                    pre { (agent) }
                }
            }
        }
    }
}

/// GitHub integration: masked token display, save/remove.
fn github_card(settings: &SettingsValues) -> Markup {
    html! {
        div class="card" {
            h2 { "GitHub integration" }
            p class="section-note" {
                "Optional. rusno works fully without a token."
            }

            @if settings.has_github_token {
                p style="margin-bottom:0.75rem;" {
                    "Token set (••••)"
                }
                form method="post" action="/settings/github-token/remove" {
                    button type="submit" class="btn btn-danger" { "Remove token" }
                }
            } @else {
                form hx-post="/settings/github-token"
                    hx-target="#github-token-msg"
                    hx-swap="innerHTML"
                {
                    label { "GitHub token" }
                    input type="password" name="token" placeholder="ghp_...";
                    button type="submit" class="btn btn-primary" { "Save token" }
                }
                div id="github-token-msg" class="fragment-msg" { }
            }
        }
    }
}

/// Docker maintenance: system df summary + safe/nuclear prune.
fn docker_card(docker_df: Option<&DiskUsageSummary>) -> Markup {
    html! {
        div class="card" {
            h2 { "Docker maintenance" }

            @if let Some(df) = docker_df {
                (docker_summary_fragment(df))
            } @else {
                div id="docker-summary"
                    hx-get="/settings/docker"
                    hx-trigger="load"
                    hx-swap="innerHTML"
                {
                    "Loading disk usage..."
                }
            }

            div class="btn-row" style="margin-top:1rem;" {
                button class="btn btn-primary"
                    hx-post="/settings/docker/prune-safe"
                    hx-target="#docker-prune-result"
                    hx-swap="innerHTML"
                    hx-confirm="Remove unused images and stopped containers? Running services are not affected."
                { "Clean cache (safe)" }
            }

            form hx-post="/settings/docker/prune-nuclear"
                hx-target="#docker-prune-result"
                hx-swap="innerHTML"
                style="margin-top:1rem;"
            {
                label { "Nuclear clean — type 'prune' to confirm" }
                input type="text" name="confirm" placeholder="prune";
                button type="submit" class="btn btn-danger" { "Nuclear clean" }
                small { "Removes ALL unused images, containers, and volumes." }
            }

            div id="docker-prune-result" class="fragment-msg" { }
        }
    }
}

/// Admin account: change password + sign out everywhere.
fn admin_card() -> Markup {
    html! {
        div class="card" {
            h2 { "Admin account" }

            h2 style="font-size:1rem; margin-bottom:0.5rem;" { "Change password" }
            form hx-post="/settings/change-password"
                hx-target="#pw-msg"
                hx-swap="innerHTML"
            {
                label { "New password" }
                input type="password" name="new_password" placeholder="New password";
                label { "Confirm password" }
                input type="password" name="confirm_password" placeholder="Confirm password";
                button type="submit" class="btn btn-primary" { "Change password" }
            }
            div id="pw-msg" class="fragment-msg" { }

            hr style="border:none; border-top:1px solid #0f3460; margin:1.5rem 0;"

            h2 style="font-size:1rem; margin-bottom:0.5rem;" { "Sessions" }
            p class="section-note" {
                "Invalidates all active sessions. You will be signed out "
                "and redirected to the login page."
            }
            form method="post" action="/settings/sign-out-everywhere" {
                button type="submit" class="btn btn-danger" { "Sign out everywhere" }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// HTMX fragments
// ---------------------------------------------------------------------------

/// HTMX partial showing a generated SSH public key + fingerprint +
/// copy button. Returned by `POST /settings/ssh/generate`.
pub fn ssh_key_fragment(pubkey: &str) -> Markup {
    let fp = ssh_fingerprint(pubkey);
    html! {
        div {
            label { "Public key (rusno_ed25519.pub)" }
            textarea id="ssh-pubkey" readonly="" rows="4"
                style="font-family:monospace;"
            { (pubkey) }
            div class="btn-row" {
                button type="button" class="btn btn-primary"
                    onclick="navigator.clipboard.writeText(document.getElementById('ssh-pubkey').value)"
                { "Copy to clipboard" }
            }
            @if let Some(fp) = fp {
                p class="muted" style="margin-top:0.5rem;" { "Fingerprint: " (fp) }
            } @else {
                p class="muted" style="margin-top:0.5rem;" { "Fingerprint unavailable" }
            }
        }
    }
}

/// HTMX partial showing the docker system df summary.
/// Returned by `GET /settings/docker`.
pub fn docker_summary_fragment(df: &DiskUsageSummary) -> Markup {
    html! {
        div class="stat-grid" style="grid-template-columns:repeat(2,1fr);" {
            div class="stat-card" {
                div class="stat-value" { (df.images) }
                div class="stat-label" { "Images" }
            }
            div class="stat-card" {
                div class="stat-value" { (df.containers) }
                div class="stat-label" { "Containers" }
            }
            div class="stat-card" {
                div class="stat-value" { (df.volumes) }
                div class="stat-label" { "Volumes" }
            }
            div class="stat-card" {
                div class="stat-value" { (df.build_cache_count) }
                div class="stat-label" { "Build cache" }
            }
        }
        p class="muted" { "Total image size: " (format_bytes(df.total_size.max(0) as u64)) }
    }
}

/// HTMX partial showing prune command output in a `<pre>`.
/// Returned by the prune-safe and prune-nuclear endpoints.
pub fn docker_prune_result(output: &str) -> Markup {
    html! {
        pre { (output) }
    }
}
