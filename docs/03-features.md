# 03 — Features

This document describes the *behavior* of every user-facing surface in rusno: the four dashboard tabs, project lifecycle, deployments, and settings. Pipeline internals (state machine, scheduling, health) live in [04-deployment-pipeline.md](./04-deployment-pipeline.md); webhook protocol in [05-webhooks.md](./05-webhooks.md).

## Conventions

- **HTMX**: all tab navigation is HTMX-driven (no full page reloads after first paint). The active tab is reflected in the URL via `hx-push-url`.
- **Polling**: dashboard and active-deploy rows use `hx-trigger="every 5s"` to refresh telemetry/status.
- **CSRF**: every state-changing HTMX request sends `X-CSRF-Token` from a meta tag injected into each page; the server validates it against the session.
- **Code editor**: CodeMirror 6 loaded from CDN, used for compose + `.env` editing. No build step; it's a single `<script>` + `<link>`.

---

## Dashboard tab

Route: `GET /` (or `/dashboard`).

### Top stats row

Four stat cards, refreshed every 5s via HTMX:

| Card | Source |
|---|---|
| Projects registered | `SELECT COUNT(*) FROM projects` |
| Containers running | bollard list filtered to containers with label `rusno.project` and state `running` |
| CPU usage | `sysinfo` — aggregate CPU load % |
| Memory usage | `sysinfo` — used / total |

### Server telemetry

Three horizontal progress bars, refreshed every 5s:

1. **CPU load** — `sysinfo` `cpu_load()` (1-min average), bar fills proportionally to 100%.
2. **Memory load** — `sysinfo` `used_memory() / total_memory()`.
3. **Storage** — `sysinfo` filesystem usage for the disk holding `projects_root` (not `/`, since that's where the bulk grows).

Each bar shows numeric `used / total (XX%)` beneath it.

### Last 5 deployments

A table of the 5 most recent deploys across all projects:

```
| Project   | Commit   | Trigger      | Status    | Started        |
|-----------|----------|--------------|-----------|----------------|
| my-app    | a1b2c3d  | webhook      | succeeded | 2 min ago      |
| api-svc   | f4e5d6c  | manual       | failed    | 10 min ago     |
```

- Each row links to the per-deploy log page.
- Row status is color-coded (`succeeded` green, `failed` red, in-flight amber).

---

## Projects tab

Route: `GET /projects`.

### Project list

Table of all projects:

```
| Name        | Source              | Branch | Auto-start | Webhook         |
|-------------|---------------------|--------|------------|-----------------|
| my-app      | git@github.com:...  | main   | on         | ✓ enabled       |
```

- Clicking a project name opens the project detail page.
- Each row has quick actions: Deploy now, Stop, Restart, Webhook toggle (inline HTMX).

### Project detail page

Route: `GET /projects/<slug>`.

Sections, in order:

1. **Header**: display name, source URL, branch, status pill (running/stopped/deploying).
2. **Webhook**: shows webhook URL (`<rusno_url>/hook/<slug>`) and secret, both copy-only (no edit). Toggle to disable webhook + HTTP deployment.
3. **Actions**: Deploy now, Stop, Restart, Rollback, Remove.
4. **Compose file editor**: CodeMirror editor showing the file at `<compose_path>`. Save button writes back to disk. "Reset to repo version" button runs `git checkout -- <compose_path>`.
5. **`.env` editor**: CodeMirror editor for `.env` (or the compose's `env_file` path). "Copy from `.env.example`" button — only visible if `.env.example` exists; copies its contents into the editor buffer (unsaved) so you can adjust before saving.
6. **Deploy history** (paginated, 25 per page): commit SHA, message, trigger, status, started, finished. Rollback button per row.

### Register project flow

Route: `GET /projects/new`.

Field order (link at top, as specified):

1. **Source URL** (top) — git SSH or HTTPS URL. On blur, rusno tries to auto-detect:
   - repo name (last URL segment minus `.git`) → used as placeholder/default for both name fields.
   - default branch (via `git ls-remote --symref` or GitHub API if token present) → placeholder for branch field.
2. **Project name** — placeholder = repo name, editable. Used as `display_name`.
3. **Folder name** — placeholder = repo name, editable. Used as on-disk folder name.
4. **Branch** — placeholder = `main` (or auto-detected default), editable.
5. **Compose file location** — text input, placeholder `docker/docker-compose.yml`. Free path; rusno resolves it relative to the project folder.
6. **Compose-up command** — text input, default `up -d --build --remove-orphans`. Free text, space-split, validated to start with `up`.
7. **Auto start after restart** — toggle, default on.
8. **Health timeout** — number input, default 60s.

On confirm:
- rusno resolves `folder_path = <projects_root>/<folder_name>`, erroring if the folder already exists (non-empty).
- Generates a random `webhook_secret` (32 bytes, base64url).
- Inserts the project row, then **immediately triggers the first deploy** (clone + compose up).
- On success, redirects to the project list. On failure, stays on the form with the error + the project is saved in a `draft`-like state (marked `webhook_enabled=0` so it doesn't auto-deploy from webhooks until you fix it).

### Project actions

- **Deploy now**: triggers a fresh deploy (git pull + compose up). Does not change branch.
- **Stop**: `docker compose -f <path> stop`. Containers remain, can be restarted.
- **Restart**: `docker compose -f <path> restart`.
- **Rollback**: opens a picker of previously-deployed commits (from `deploys` where `status='succeeded'`); selecting one triggers a rollback deploy (`git checkout <sha>` + compose up).
- **Remove**: confirms, then `docker compose down -v --remove-orphans`, deletes `folder_path`, deletes the project row. Webhook becomes 404.
- **Webhook toggle**: flips `webhook_enabled`; when off, the webhook endpoint returns 404, and the project won't deploy from any HTTP trigger.

### Edit compose / `.env`

- The editor always reads from and writes to the on-disk file at the resolved path.
- Save is atomic: write to `<path>.tmp`, `fsync`, rename over the original.
- If the compose path doesn't exist when first opened, the editor starts empty and saving creates it (including parent dirs).
- `.env.example` copy: the button loads `.env.example` contents into the editor buffer as an unsaved draft — you edit then save to `.env`. It does **not** shell-copy the file; it's a content load so you stay in the editor.

---

## Deployments tab

Route: `GET /deployments`.

### Latest deployments (paginated)

A paginated table of deploys across all projects, latest first, 25 per page:

```
| Project | Commit   | Message              | Trigger       | Status    | Started   |
|---------|----------|----------------------|---------------|-----------|-----------|
| my-app  | a1b2c3d  | feat: add cache      | webhook       | succeeded | 2m ago    |
| api-svc | f4e5d6c  | fix: nil pointer    | manual        | failed    | 10m ago   |
| my-app  | 9e8d7c6  | chore: bump deps    | webhook_plain | succeeded | 1h ago    |
```

- `trigger` shows the source: `webhook` (GitHub push), `webhook_plain` (HTTP POST), `manual`, `rollback`, `restart`.
- Clicking a row opens the deploy detail page.
- In-flight deploys show a live status pill (queued/pulling/building/starting/healthy) that polls every 5s.
- Pagination at the bottom: "← older | newer →", HTMX swaps the table body.

### Deploy detail page

Route: `GET /deployments/<id>`.

- Full header: project name, commit SHA + message, trigger, status, started/finished timestamps, duration.
- **Log view**: shows the full `log_tail` inline (last ~200 lines), with a "view full log" link that serves the raw `<log_path>` file as `text/plain` (auth-gated).
- If the deploy is in-flight, the log view polls every 5s and appends new lines.
- **Rollback to this commit** button (only on succeeded deploys).

---

## Settings page

Route: `GET /settings`.

### Service configuration

- **rusno URL / domain** — text field, e.g. `https://rusno.example.com`. Used to construct webhook URLs shown in project detail. Saved to `settings.rusno_url`.
- **Port** — read-only display of the current bind port (port is set in `config.toml` or CLI flag; changing it requires a restart). Shows the current value + a note: "change via `config.toml` or `--port` flag".
- **Projects root** — text field showing current root; changing it writes to `settings.projects_root`. Only affects future project registrations (existing `folder_path` values are absolute and frozen).
- **Deploy concurrency** — number input (default 2). Global semaphore cap for simultaneous deploys.

### SSH keys

- Shows current ssh mode toggle: `rusno-managed` | `host-existing`.
- **rusno-managed mode**:
  - If `~/.rusno/ssh/rusno_ed25519` exists: shows the pubkey in a read-only textarea with a copy button, plus the fingerprint.
  - If not: shows a "Generate key" button. On click, rusno runs `ssh-keygen -t ed25519 -f ~/.rusno/ssh/rusno_ed25519 -N ""` and displays the pubkey.
  - "Copy public key" button — copies pubkey to clipboard (for pasting into GitHub/GitLab deploy keys).
- **host-existing mode**:
  - rusno scans `~/.ssh/` for `id_*` / `*.pub` pairs, lists each with fingerprint + type.
  - Shows which keys are loaded in `ssh-agent` (`ssh-add -l`).
  - "Copy public key" per listed key.
  - Note: "rusno will use whatever ssh-agent provides; ensure your key is loaded (`ssh-add ~/.ssh/id_ed25519`)."

### GitHub integration (optional)

- **GitHub token** — password-type field, masked. If a token is stored, shows "Token set (••••)" with "Remove" button. If empty, shows a paste field + "Save".
  - Stored encrypted (AES-256-GCM) in `settings.github_token_encrypted`.
  - Unlocks: HTTPS-private clones (token used as the git credential), webhook auto-register (when a project is created from a GitHub HTTPS URL), repo autocomplete on the register form.
- Note in the UI: "Optional. rusno works fully without a token — use SSH deploy keys or public repos. The token only enables private HTTPS clones, webhook auto-registration, and repo autocomplete."

### Docker maintenance

- Shows `docker system df` output as a summary: reclaimable images, build cache, stopped containers, unused volumes.
- **Clean cache (safe)** button — runs `docker image prune -a -f` + `docker builder prune -f`. Confirmation modal: "This removes all unused images and build cache. Running containers are not affected. Continue?"
- **Nuclear clean** button — runs `docker system prune -a --volumes -f`. Stronger confirmation modal with a text-typed confirm ("type `prune` to confirm"): "⚠ This also removes unused volumes. Any data in them is lost. Only use this if you know no stopped stack needs its volumes."

### Admin account

- **Change password** — two fields (new + confirm), writes argon2 hash to `settings.admin_password_hash`. Does not invalidate existing sessions.
- (Optional) **Sign out everywhere** — clears the `sessions` table; forces re-login on all devices.
