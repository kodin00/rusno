//! Maud templates for the Projects tab: list, register form, detail page,
//! inline HTMX partials (autodetect, rollback picker, webhook toggle), and
//! the compose/env CodeMirror editors.

use maud::{html, Markup, PreEscaped};

use crate::models::{deploy::Deploy, project::Project};
use crate::templates::layout::base;

/// First 7 chars of a commit SHA, or an em dash when absent.
fn short_sha(sha: &Option<String>) -> String {
    match sha {
        Some(s) if !s.is_empty() => s.chars().take(7).collect(),
        _ => "—".to_string(),
    }
}

/// Mask a secret, keeping only the first and last 4 characters.
fn mask_secret(secret: &str) -> String {
    let len = secret.len();
    if len <= 8 {
        "••••••••".to_string()
    } else {
        format!("{}••••••••{}", &secret[..4], &secret[len - 4..])
    }
}

/// GET /projects — full project list page.
pub fn projects_page(csrf: &str, projects: &[Project]) -> Markup {
    base(
        "Projects",
        "projects",
        Some(csrf),
        html! {
            div style="display:flex;justify-content:space-between;align-items:center;margin-bottom:1rem" {
                h2 style="margin:0" { "Projects" }
                a class="btn btn-primary" href="/projects/new" hx-boost="true" { "+ New Project" }
            }

            @if projects.is_empty() {
                div class="card" {
                    p { "No projects yet. " }
                    a href="/projects/new" hx-boost="true" { "Register your first project." }
                }
            } @else {
                div class="card" {
                    table {
                        thead {
                            tr {
                                th { "Name" }
                                th { "Source" }
                                th { "Branch" }
                                th { "Auto-start" }
                                th { "Webhook" }
                                th { "Actions" }
                            }
                        }
                        tbody {
                            @for p in projects {
                                tr {
                                    td {
                                        a href=(format!("/projects/{}", p.slug)) hx-boost="true" {
                                            (p.display_name)
                                        }
                                    }
                                    td { (p.source_type) }
                                    td { (p.branch) }
                                    td { (if p.auto_start { "yes" } else { "no" }) }
                                    td { (webhook_toggle_fragment(&p.slug, p.webhook_enabled)) }
                                    td style="white-space:nowrap" {
                                        form method="post"
                                            hx-post=(format!("/projects/{}/deploy", p.slug))
                                            hx-boost="true"
                                            style="display:inline"
                                        {
                                            button class="btn btn-primary" type="submit" { "Deploy" }
                                        }
                                        form method="post"
                                            hx-post=(format!("/projects/{}/stop", p.slug))
                                            hx-boost="true"
                                            style="display:inline"
                                        {
                                            button type="submit" { "Stop" }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        },
    )
}

/// GET /projects/new — the register form.
pub fn new_project_form_page(csrf: &str, error: Option<&str>) -> Markup {
    base(
        "New project",
        "projects",
        Some(csrf),
        html! {
            h2 style="margin-bottom:1rem" { "Register a project" }

            @if let Some(msg) = error {
                div class="card" style="background:rgba(231,76,60,0.12);border:1px solid #e74c3c;margin-bottom:1rem" {
                    strong { (msg) }
                }
            }

            form class="card" method="post" action="/projects" hx-boost="true" {
                label class="field" {
                    "Source URL"
                    input type="url"
                        name="source_url"
                        placeholder="git@github.com:owner/repo.git or https://github.com/owner/repo.git"
                        required
                        autocomplete="off"
                        hx-get="/projects/new/autodetect"
                        hx-trigger="blur"
                        hx-target="#autodetect-result"
                        hx-swap="innerHTML";
                    div id="autodetect-result" style="margin-top:0.5rem" { }
                }

                label class="field" {
                    "Project name"
                    input type="text"
                        name="display_name"
                        placeholder="My app"
                        autocomplete="off";
                }

                label class="field" {
                    "Folder name"
                    input type="text"
                        name="folder_name"
                        id="folder_name"
                        placeholder="repo"
                        autocomplete="off";
                }

                label class="field" {
                    "Branch"
                    input type="text"
                        name="branch"
                        id="branch"
                        placeholder="main"
                        autocomplete="off";
                }

                label class="field" {
                    "Compose file path"
                    input type="text"
                        name="compose_path"
                        value="docker-compose.yml"
                        placeholder="docker/docker-compose.yml"
                        autocomplete="off";
                }

                label class="field" {
                    "Compose-up command"
                    input type="text"
                        name="compose_command"
                        value="up -d --build --remove-orphans"
                        autocomplete="off";
                }

                label class="checkbox" {
                    input type="checkbox" name="auto_start" value="on" checked="checked";
                    "Auto-start"
                }

                label class="field" {
                    "Health timeout (seconds)"
                    input type="number"
                        name="health_timeout_secs"
                        value="60"
                        min="1"
                        style="width:8rem";
                }

                button type="submit" class="btn btn-primary" { "Create project" }
                a href="/projects" hx-boost="true" style="margin-left:0.75rem" { "Cancel" }
            }
        },
    )
}

/// GET /projects/<slug> — the project detail page.
pub fn project_detail_page(
    csrf: &str,
    project: &Project,
    webhook_url: &str,
    deploys: &[Deploy],
    is_deploying: bool,
) -> Markup {
    let slug = &project.slug;
    let status = if is_deploying {
        "deploying".to_string()
    } else {
        deploys
            .first()
            .map(|d| d.status.clone())
            .unwrap_or_else(|| "idle".to_string())
    };

    base(
        "Project",
        "projects",
        Some(csrf),
        html! {
            div style="display:flex;justify-content:space-between;align-items:center;margin-bottom:1rem" {
                h2 style="margin:0" { (project.display_name) }
                span class=(format!("pill pill-{}", status)) { (status) }
            }

            // Header summary
            div class="card" {
                table {
                    tr {
                        th style="width:10rem" { "Source" }
                        td { (project.source_type) " — " (project.source_url) }
                    }
                    tr {
                        th { "Branch" }
                        td { (project.branch) }
                    }
                    tr {
                        th { "Folder" }
                        td { (project.folder_path) }
                    }
                    tr {
                        th { "Compose" }
                        td { (project.compose_path) " (" (project.compose_command) ")" }
                    }
                }
            }

            // Action buttons
            div class="card" {
                h3 style="margin-top:0" { "Actions" }
                form method="post" hx-post=(format!("/projects/{}/deploy", slug)) hx-boost="true" style="display:inline;margin-right:0.5rem" {
                    button type="submit" class="btn btn-primary" disabled[is_deploying] { "Deploy now" }
                }
                form method="post" hx-post=(format!("/projects/{}/stop", slug)) hx-boost="true" style="display:inline;margin-right:0.5rem" {
                    button type="submit" { "Stop" }
                }
                form method="post" hx-post=(format!("/projects/{}/restart", slug)) hx-boost="true" style="display:inline;margin-right:0.5rem" {
                    button type="submit" { "Restart" }
                }
                button type="button"
                    hx-get=(format!("/projects/{}/rollback", slug))
                    hx-target="#rollback-modal"
                    hx-swap="innerHTML"
                    style="margin-right:0.5rem"
                { "Rollback" }
                form method="post" hx-post=(format!("/projects/{}/remove", slug)) hx-boost="true"
                    hx-confirm="Remove this project? Containers will be torn down and the folder deleted. This cannot be undone."
                    style="display:inline"
                {
                    button type="submit" class="btn btn-danger" { "Remove" }
                }
                div id="rollback-modal" { }
            }

            // Webhook section
            div class="card" x-data=(r#"{ revealed: false }"#) {
                h3 style="margin-top:0" { "Webhook" }
                @if project.webhook_enabled {
                    p { "Webhook is " strong style="color:#2ecc71" { "enabled" } " — POST pushes to the URL below to trigger a deploy." }
                } @else {
                    p { "Webhook is " strong { "disabled" } " — enable it to allow push-triggered deploys." }
                }

                label style="display:block;margin-bottom:0.5rem" {
                    "Webhook URL"
                    div style="display:flex;gap:0.5rem;align-items:center" {
                        code id="webhook-url" style="flex:1" { (webhook_url) }
                        button type="button"
                            onclick=(r#"navigator.clipboard.writeText(document.getElementById('webhook-url').textContent)"#)
                        { "Copy" }
                    }
                }

                label style="display:block;margin-bottom:0.5rem" {
                    "Webhook secret"
                    div style="display:flex;gap:0.5rem;align-items:center" {
                        code id="webhook-secret-masked" x-show="!revealed" style="flex:1" { (mask_secret(&project.webhook_secret)) }
                        code id="webhook-secret" x-show="revealed" style="flex:1;display:none" { (project.webhook_secret) }
                        button type="button" x-on:click="revealed = !revealed" x-text="revealed ? 'Hide' : 'Reveal'" { "Reveal" }
                        button type="button"
                            onclick=(r#"navigator.clipboard.writeText(document.getElementById('webhook-secret').textContent)"#)
                        { "Copy" }
                    }
                }

                // Inline toggle (self-swapping form)
                (webhook_toggle_fragment(slug, project.webhook_enabled))
            }

            // Compose editor
            div class="card" {
                h3 style="margin-top:0" { "Compose file" }
                button type="button"
                    hx-get=(format!("/projects/{}/compose", slug))
                    hx-target="#compose-area"
                    hx-swap="innerHTML"
                    class="btn btn-primary"
                { "Load editor" }
                div id="compose-area" style="margin-top:1rem" { }
            }

            // Env editor
            div class="card" {
                h3 style="margin-top:0" { "Environment (.env)" }
                button type="button"
                    hx-get=(format!("/projects/{}/env", slug))
                    hx-target="#env-area"
                    hx-swap="innerHTML"
                    class="btn btn-primary"
                { "Load editor" }
                div id="env-area" style="margin-top:1rem" { }
            }

            // Deploy history
            div class="card" {
                h3 style="margin-top:0;margin-bottom:0.75rem" { "Deploy history" }
                @if deploys.is_empty() {
                    p { "No deploys yet." }
                } @else {
                    table {
                        thead {
                            tr {
                                th { "Commit" }
                                th { "Message" }
                                th { "Trigger" }
                                th { "Status" }
                                th { "Started" }
                                th { "Action" }
                            }
                        }
                        tbody {
                            @for d in deploys {
                                tr {
                                    td { (short_sha(&d.commit_sha)) }
                                    td { (d.commit_msg.as_deref().unwrap_or("—")) }
                                    td { (d.trigger) }
                                    td {
                                        span class=(format!("pill pill-{}", d.status)) { (d.status) }
                                    }
                                    td { (d.started_at) }
                                    td {
                                        @if d.status == "succeeded" && d.commit_sha.as_deref().is_some() && !d.is_rollback {
                                            form method="post"
                                                hx-post=(format!("/projects/{}/rollback", slug))
                                                hx-boost="true"
                                                hx-confirm="Roll back to this commit?"
                                            {
                                                input type="hidden" name="commit_sha" value=(d.commit_sha.as_deref().unwrap_or(""));
                                                button type="submit" { "Rollback" }
                                            }
                                        } @else {
                                            "—"
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        },
    )
}

/// HTMX partial: autodetect response shown under the source URL field.
pub fn autodetect_response(repo_name: &str, default_branch: &str) -> Markup {
    html! {
        div class="autodetect-result" style="color:#9aa0b5;font-size:0.85rem" {
            "Detected repo: " strong { (repo_name) }
            " · default branch: "
            button type="button"
                onclick=(format!("document.getElementById('branch').value = '{}'", default_branch))
                style="background:none;border:none;color:#5eb1ff;cursor:pointer;padding:0"
            { (default_branch) }
            " · "
            button type="button"
                onclick=(format!("document.getElementById('folder_name').value = '{}'", repo_name))
                style="background:none;border:none;color:#5eb1ff;cursor:pointer;padding:0"
            { "use as folder name" }
        }
    }
}

/// HTMX partial: prior succeeded deploys as a rollback picker.
pub fn rollback_picker_fragment(deploys: &[Deploy]) -> Markup {
    html! {
        div class="card" style="margin-top:0.75rem" {
            h3 style="margin-top:0" { "Roll back to a previous deploy" }
            @if deploys.is_empty() {
                p { "No rollback targets yet. A succeeded deploy is required." }
            } @else {
                form method="post" hx-boost="true" {
                    table {
                        thead {
                            tr {
                                th style="width:2rem" { }
                                th { "Commit" }
                                th { "Message" }
                                th { "Deployed" }
                            }
                        }
                        tbody {
                            @for (i, d) in deploys.iter().enumerate() {
                                tr {
                                    td {
                                        input type="radio"
                                            name="commit_sha"
                                            value=(d.commit_sha.as_deref().unwrap_or(""))
                                            checked[i == 0];
                                    }
                                    td { (short_sha(&d.commit_sha)) }
                                    td { (d.commit_msg.as_deref().unwrap_or("—")) }
                                    td { (d.started_at) }
                                }
                            }
                        }
                    }
                    button type="submit" class="btn btn-primary" { "Roll back" }
                    a href=".." hx-boost="true" style="margin-left:0.75rem" { "Cancel" }
                }
            }
        }
    }
}

/// HTMX partial: inline webhook on/off toggle. Self-swapping on click.
pub fn webhook_toggle_fragment(slug: &str, enabled: bool) -> Markup {
    html! {
        form method="post"
            hx-post=(format!("/projects/{}/webhook-toggle", slug))
            hx-target="this"
            hx-swap="outerHTML"
            style="display:inline;margin-top:0.5rem"
        {
            button type="submit"
                class=(if enabled { "btn btn-danger" } else { "btn btn-primary" })
                style="font-size:0.8rem;padding:0.25rem 0.6rem"
            {
                (if enabled { "Webhook: on (click to disable)" } else { "Webhook: off (click to enable)" })
            }
        }
    }
}

/// The CodeMirror 6 init script. When `yaml_mode` is true, the yaml language
/// extension is loaded; otherwise the editor is plain text (used for .env).
fn codemirror_script(textarea_id: &str, yaml_mode: bool) -> Markup {
    let lang_import: String = if yaml_mode {
        r#"import { yaml } from "https://esm.sh/@codemirror/lang-yaml@6";
        const langExt = yaml();"#
            .to_string()
    } else {
        "const langExt = [];".to_string()
    };
    let script = format!(
        r#"
        import {{ EditorView, basicSetup }} from "https://esm.sh/codemirror@6.60.0";
        {lang_import}
        const ta = document.getElementById('{textarea_id}');
        if (ta && !ta.__cmBound) {{
            ta.__cmBound = true;
            const host = ta.parentElement.insertBefore(document.createElement('div'), ta);
            const view = new EditorView({{
                doc: ta.value,
                parent: host,
                extensions: [basicSetup, langExt]
            }});
            const form = ta.closest('form');
            if (form) form.addEventListener('submit', () => {{ ta.value = view.state.doc.toString(); }});
        }}
        "#,
        lang_import = lang_import,
        textarea_id = textarea_id,
    );
    PreEscaped(format!("<script type=\"module\">{}</script>", script))
}

/// GET /projects/<slug>/compose — the compose file editor.
pub fn compose_editor(slug: &str, content: &str) -> Markup {
    html! {
        form hx-post=(format!("/projects/{}/compose", slug)) hx-target="#compose-status" hx-swap="innerHTML" {
            textarea id="compose-editor" name="content"
                style="width:100%;min-height:24rem;font-family:monospace;background:#0d0d1a;color:#e0e0e0;border:1px solid #0f3460;border-radius:6px;padding:0.5rem"
            { (content) }
            div style="margin-top:0.5rem;display:flex;gap:0.5rem;align-items:center" {
                button type="submit" class="btn btn-primary" { "Save" }
                button type="button"
                    hx-post=(format!("/projects/{}/compose", slug))
                    hx-target="#compose-area"
                    hx-swap="innerHTML"
                    hx-vals=(r#"{"reset":"1"}"#)
                    hx-confirm="Reset the compose file to the repo version? Local changes will be lost."
                { "Reset to repo" }
                span id="compose-status" style="color:#9aa0b5" { }
            }
        }
        (codemirror_script("compose-editor", true))
    }
}

/// GET /projects/<slug>/env — the .env editor.
pub fn env_editor(slug: &str, content: &str, has_example: bool) -> Markup {
    html! {
        form hx-post=(format!("/projects/{}/env", slug)) hx-target="#env-status" hx-swap="innerHTML" {
            textarea id="env-editor" name="content"
                style="width:100%;min-height:20rem;font-family:monospace;background:#0d0d1a;color:#e0e0e0;border:1px solid #0f3460;border-radius:6px;padding:0.5rem"
            { (content) }
            div style="margin-top:0.5rem;display:flex;gap:0.5rem;align-items:center" {
                button type="submit" class="btn btn-primary" { "Save" }
                @if has_example {
                    button type="button"
                        hx-get=(format!("/projects/{}/env/example", slug))
                        hx-target="#env-example-preview"
                        hx-swap="innerHTML"
                    { "Copy from .env.example" }
                }
                span id="env-status" style="color:#9aa0b5" { }
            }
        }
        div id="env-example-preview" style="margin-top:0.5rem" { }
        (codemirror_script("env-editor", false))
    }
}
