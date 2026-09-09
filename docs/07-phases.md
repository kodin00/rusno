# 07 — Implementation Phases

The build is phased so each phase ends with something runnable. Each phase references the relevant design docs. A phase is "done" when its acceptance criteria pass — not when the code compiles.

## Phase 0 — Scaffolding

**Goal**: a repo that builds and runs a hello-world axum server, with mise pinned.

Tasks:
- `cargo init`, set up the workspace layout from [06-tech-stack.md](./06-tech-stack.md).
- Write `.mise.toml` (pinned Rust + dev CLIs).
- Add base dependencies: `tokio`, `axum`, `tower`, `tower-http`, `maud`, `tracing`, `tracing-subscriber`, `clap`, `anyhow`, `config`.
- `src/main.rs`: `clap` subcommand `serve --port 6967` that starts axum with a single `GET /` returning "rusno".
- Base HTML layout template (`templates/layout.rs`) with nav stubs for the four tabs.
- `.gitignore`, `README.md` (repo-level, points to `docs/`).
- CI: GitHub Actions that runs `mise install` + `cargo build` + `cargo clippy -D warnings` + `cargo fmt --check`.

**Acceptance**:
- `mise install && cargo run -- serve` starts a server on `:6967` returning the base layout.
- CI is green.

---

## Phase 1 — Persistence & config

**Goal**: rusno can read config, run migrations, and persist settings.

Docs: [01-architecture.md](./01-architecture.md), [02-data-model.md](./02-data-model.md).

Tasks:
- Add `sqlx` (sqlite), `chrono`.
- `migrations/0001_initial.sql` with the full schema from doc 02.
- `src/db.rs`: pool, `run_migrations()` on startup.
- `src/config.rs`: read `~/.rusno/config.toml`; if missing, create with defaults (`port=6967`, `projects_root=~/rusno/projects`, `ssh_mode=rusno-managed`, `data_dir=~/.rusno`).
- `~/.rusno/` dir + `keys/` + `ssh/` created on startup if missing.
- `rusno init` subcommand: writes config + creates dirs (headless setup path).
- `settings` table defaults: `deploy_concurrency=2`, `default_health_timeout_secs=60`.

**Acceptance**:
- `rusno init` creates `~/.rusno/config.toml` + dirs.
- `rusno serve` runs migrations on startup and starts cleanly with a populated DB.

---

## Phase 2 — Auth & first-run wizard

**Goal**: admin password set, sessions work, login/logout.

Docs: [02-data-model.md](./02-data-model.md) (sessions table), [03-features.md](./03-features.md) (Settings → Admin account).

Tasks:
- Add `argon2`, `cookie`, `rand`, `subtle`.
- `src/crypto.rs`: argon2 hash/verify for admin password.
- First-run wizard: `GET /setup` (if `admin_password_hash` is unset) → form (password + confirm) → writes hash + generates `session_secret`. Redirects to `/login`.
- `POST /login`: verify password, create session row, set signed cookie, redirect to `/`.
- `POST /logout`: delete session, clear cookie.
- Auth middleware: every non-`/hook/*` and non-`/setup` and non-`/login` route requires a valid session.
- CSRF: `X-CSRF-Token` meta tag rendered in base layout; middleware validates on POST/PUT/DELETE.

**Acceptance**:
- Fresh install → visit `/` → redirected to `/setup`.
- Set password → redirected to `/login` → log in → land on dashboard (empty).
- Restart server → cookie still valid (7-day sliding).

---

## Phase 3 — Dashboard telemetry & container listing

**Goal**: the Dashboard tab shows real data.

Docs: [03-features.md](./03-features.md) (Dashboard tab), [01-architecture.md](./01-architecture.md).

Tasks:
- Add `sysinfo`, `bollard`.
- `src/docker.rs`: bollard client, `list_rusno_containers()` (filter by label `rusno.project`, state `running`), `system_df()`, `prune_safe()`, `prune_nuclear()`.
- Dashboard route `GET /`: project count, container count, CPU%, memory used/total, storage used/total (disk holding `projects_root`).
- Three telemetry progress bars (CPU/memory/storage) with HTMX 5s poll (`hx-trigger="every 5s"` on the bars' fragment).
- Last 5 deploys table (empty for now — will populate in Phase 6).

**Acceptance**:
- Dashboard shows non-zero CPU/memory (real sysinfo data) and 0 projects / 0 containers.
- Telemetry bars update every 5s without full page reload.

---

## Phase 4 — Projects: registration & listing

**Goal**: can register a project, see it in the list, but deploy is a stub.

Docs: [03-features.md](./03-features.md) (Projects tab → Register flow, Project list), [02-data-model.md](./02-data-model.md) (projects table).

Tasks:
- `src/models/project.rs`: sqlx row type, CRUD.
- `GET /projects`: list table.
- `GET /projects/new`: the registration form with field order from doc 03 (source URL at top, name + folder name with repo-name placeholder, branch with `main` default, compose path, compose command, auto-start toggle, health timeout).
- `GET /projects/new/autodetect?url=...`: HTMX endpoint that, given the source URL, returns JSON-ish HTML fragment with repo name + auto-detected default branch (via `git ls-remote --symref` or GitHub API if token). Called on URL-field blur.
- `POST /projects`: validate, resolve `folder_path`, generate `webhook_secret` (32 random bytes, base64url), derive `slug` from `folder_name` (with collision suffix), insert row. First deploy is **enqueued but not yet implemented** — placeholder returns "deploy pending (Phase 6)".
- Project detail page `GET /projects/<slug>`: header + webhook section (URL + secret, copy-only, masked with reveal) + action buttons (stubs) + editors (stubs).

**Acceptance**:
- Register a public repo → it appears in the project list.
- Project detail shows the webhook URL (relative, since `rusno_url` may be unset) and copyable secret.
- Folder path is recorded; the actual clone happens in Phase 6.

---

## Phase 5 — Compose & `.env` editors

**Goal**: edit the compose file and `.env` in-browser.

Docs: [03-features.md](./03-features.md) (Project detail → editors).

Tasks:
- CodeMirror 6 loaded from CDN in base layout (only on pages that need it).
- `GET /projects/<slug>/compose`: returns the compose file contents as a fragment with a CodeMirror textarea.
- `POST /projects/<slug>/compose`: atomic save (write `.tmp` → `fsync` → rename) to `<folder_path>/<compose_path>`, creating parent dirs if needed.
- `GET /projects/<slug>/env` + `POST` for `.env` at the compose's `env_file` path (default `<folder_path>/.env`; rusno parses the compose to detect `env_file:`).
- "Copy from `.env.example`" button: `GET /projects/<slug>/env/example` loads `.env.example` contents into the editor buffer (unsaved) via HTMX swap.
- "Reset to repo version" for compose: `git checkout -- <compose_path>` (requires the repo to be cloned — available after Phase 6; until then, disabled with tooltip).

**Acceptance**:
- On a project whose folder exists (manually clone for testing), open the compose editor, edit, save → file on disk reflects changes.
- Create a new compose at a path that didn't exist → saving creates it.
- Copy `.env.example` → edit → save → `.env` written.

---

## Phase 6 — Deploy pipeline (core)

**Goal**: registering a project actually clones + runs compose; deploy history visible.

Docs: [04-deployment-pipeline.md](./04-deployment-pipeline.md), [01-architecture.md](./01-architecture.md).

Tasks:
- `src/deploy/`: scheduler (global semaphore via `tokio::sync::Semaphore`), per-project worker (`mpsc` capacity 1, coalesce-pending logic), state machine.
- `src/deploy/git.rs`: `clone`, `fetch + checkout + pull --ff-only`, `checkout <sha>` (rollback), `rev-parse HEAD`, `ls-remote --symref` for autodetect.
- `src/deploy/worker.rs`: the 6-step deploy (acquire permit → git sync → record commit → compose up → health poll → terminal). Log capture to `<folder_path>/logs/<id>.log` + ring buffer → `log_tail`.
- `src/crypto.rs`: add master key load + AES-256-GCM (needed now if we store anything sensitive — mainly for the GitHub token in Phase 9, but the module lands here).
- Wire `POST /projects` to enqueue a first deploy after insert.
- Deployments tab `GET /deployments`: paginated table, 25/page, `trigger` column, status pill, in-flight poll.
- Deploy detail page `GET /deployments/<id>`: header + `log_tail` inline + full-log link.
- rusno startup: enqueue `trigger='restart'` for `auto_start=1` projects.
- Crash recovery: on startup, mark non-terminal deploys as `failed` with `error="rusno restarted mid-deploy"`.

**Acceptance**:
- Register a public repo with a real compose → rusno clones, runs `docker compose up`, deploy reaches `succeeded`, containers appear in the dashboard count.
- Deployments tab shows the deploy with its commit + status.
- Restart rusno → auto-start projects re-deploy (or skip if HEAD unchanged).
- A second push while a deploy is running → coalesced (only latest queued survives).

---

## Phase 7 — Project actions & rollback

**Goal**: stop, restart, rollback, remove all work.

Docs: [03-features.md](./03-features.md) (Project actions), [04-deployment-pipeline.md](./04-deployment-pipeline.md) (Rollback).

Tasks:
- `POST /projects/<slug>/deploy`: manual deploy now (trigger=`manual`).
- `POST /projects/<slug>/stop`: `docker compose -f <path> stop`.
- `POST /projects/<slug>/restart`: `docker compose -f <path> restart`.
- `POST /projects/<slug>/rollback`: shows picker (HTMX partial) of prior succeeded deploys' commits; selecting one enqueues `trigger='rollback'`.
- `POST /projects/<slug>/remove`: confirm modal → `docker compose down -v --remove-orphans` → delete `folder_path` → delete row.
- Webhook toggle inline on the list + detail.

**Acceptance**:
- Stop → containers stop (dashboard count drops).
- Restart → containers come back.
- Rollback to a prior commit → deploy row marked `is_rollback=1`, `git checkout <sha>` visible in logs.
- Remove → folder gone, project gone, webhook 404s.

---

## Phase 8 — Webhooks

**Goal**: external triggers work.

Docs: [05-webhooks.md](./05-webhooks.md).

Tasks:
- Add `hmac` + `sha2` (or `ring`).
- `POST /hook/<slug>`: auto-detect GitHub HMAC vs plain secret, verify in constant time, branch filter (parse `ref`), enqueue deploy, return 200 fast.
- Rate limit: 10/min per slug → 429.
- Body cap 10 MB → 413.
- Log deliveries (slug, mode, branch, deploy_id or ignored reason) — never the secret.
- GitHub non-`push` events → 200 `ignored`.

**Acceptance**:
- `curl -X POST -H "X-Rusno-Secret: <secret>" http://localhost:6967/hook/<slug>` → deploy enqueued.
- GitHub push to the configured branch → deploy enqueued; push to another branch → 200 ignored.
- Bad signature → 401.

---

## Phase 9 — Settings page (full)

**Goal**: all Settings sections work.

Docs: [03-features.md](./03-features.md) (Settings page).

Tasks:
- `GET/POST /settings`: rusno URL, projects root, deploy concurrency, health timeout default.
- SSH section: mode toggle, `rusno-managed` key gen (`ssh-keygen -t ed25519 -f ~/.rusno/ssh/rusno_ed25519 -N ""`), pubkey display + copy, `host-existing` scan of `~/.ssh/` + `ssh-add -l`.
- GitHub token: masked field, save (AES-256-GCM encrypt) / remove.
- Docker maintenance: `docker system df` summary, safe prune button (`image prune -a -f` + `builder prune -f`), nuclear prune (`system prune -a --volumes -f`) with typed confirm.
- Change password (argon2), sign out everywhere (clear sessions).

**Acceptance**:
- Set rusno URL → project detail webhook URL becomes absolute.
- Generate rusno-managed key → pubkey appears, copyable.
- Paste GitHub token → saved, masked, removable; enables private HTTPS clone on next project register.
- Safe prune runs and reclaims space; nuclear prune requires typed confirm.

---

## Phase 10 — Install variants & release

**Goal**: shippable.

Docs: [01-architecture.md](./01-architecture.md) (Install variants), [06-tech-stack.md](./06-tech-stack.md) (Release variants).

Tasks:
- `deploy/rusno.service`: systemd unit template with `User=` + `ExecStart=/usr/local/bin/rusno serve`.
- `install.sh`: download binary, install, install service, prompt for `rusno init`.
- `Dockerfile`: multi-stage (`rust:1.81` build → `debian:slim` or `alpine` runtime), `ENTRYPOINT ["rusno", "serve"]`. Multi-arch via `docker buildx`.
- GitHub Actions release workflow: on tag, build matrix + publish binary + push image to GHCR.
- Docs: `docs/install.md` covering both variants + the `~/.rusno/keys/master.key` backup warning.

**Acceptance**:
- `install.sh` on a fresh Ubuntu VM → rusno running as a service, first-visit wizard works.
- `docker run` with the documented bind-mounts → rusno running, can deploy a project that uses the host Docker socket.
- Tag pushed → artifacts published.

---

## Phase 11 — Polish & hardening (post-v1)

Not blocking v1; tracked for shortly after:

- Vendored CodeMirror/HTMX for offline use.
- Webhook secret rotation (per-project regenerate + show new URL/secret).
- SSE-based live log streaming (replace 5s polling for in-flight deploys).
- Multi-arch macOS binary as a first-class release.
- Audit log (who deployed what, when, settings changes).
- Project tags/groups for hosts with many projects.
- Backup/restore: `rusno backup` exports DB + config + master key into a tarball; `rusno restore` reverses it.
