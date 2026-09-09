# rusno — Deployment Manager

**rusno** is a Rust-based, self-hosted deployment manager. It runs `docker compose` stacks on your host, exposes a web dashboard to register and manage projects, receives GitHub webhooks (or plain HTTP POSTs) for automatic deployment, and gives you per-project control over compose files, env files, branches, rollbacks, and host hygiene.

> Status: **design spec**. The documents in this folder describe the intended v1 build. Nothing is implemented yet — this folder *is* the source of truth that the implementation will follow.

## Quick facts

| | |
|---|---|
| Language | Rust (stable) |
| Web framework | `axum` |
| Frontend | Server-rendered HTML (maud) + HTMX + Alpine.js, CodeMirror 6 (CDN) for in-browser code editing |
| Persistence | SQLite via `sqlx` (migrations via `sqlx migrate`) |
| Container access | `bollard` (Docker Engine API) + shelling out to `docker compose` for deploy |
| Host telemetry | `sysinfo` (CPU / memory / storage) |
| Dev toolchain | `mise` (pinned via `.mise.toml`) |
| Auth | Single admin password (argon2) + signed cookie session (7-day sliding) |
| Default port | `6967` (configurable in Settings or `rusno serve --port`) |
| Install variants | (a) single static binary + systemd, (b) dockerized rusno with host socket bind-mounted |

## Documents

Read in order for the full picture; each is self-contained for reference.

1. [Architecture](./01-architecture.md) — process model, data dirs, install variants, dependency list.
2. [Data Model](./02-data-model.md) — SQLite schema sketch, project layout, secrets handling.
3. [Features](./03-features.md) — dashboard, projects, deployments, settings — the full feature behavior.
4. [Deployment Pipeline](./04-deployment-pipeline.md) — state machine, scheduling, health checks, rollback.
5. [Webhooks](./05-webhooks.md) — endpoint design, GitHub HMAC vs plain, trigger filtering.
6. [Tech Stack & Tooling](./06-tech-stack.md) — crates, mise setup, build, release.
7. [Implementation Phases](./07-phases.md) — phased build plan, this is where the work happens.

## Scope summary (from the grilling sessions)

- **rusno itself**: single Rust binary; ships as (a) systemd service for bare-metal and (b) docker image that bind-mounts the host docker socket, `~/.rusno`, projects root, and `~/.ssh`.
- **Projects**: registered via git SSH/HTTPS URL (GHCR image mode was dropped — you author the compose). Per-project: branch (default `main`, auto-detect with master fallback), compose path (e.g. `docker/docker-compose.yml`), compose-up command (default `up -d --build --remove-orphans`), auto-start-on-restart toggle.
- **Project folders**: root is `~/rusno/projects/` by default, configurable on Settings. Per-project folder name defaults to repo name, overridable; display name separate, same default.
- **Editors**: in-browser editing of the compose file and `.env` at their repo paths; copy `.env.example` → `.env` action.
- **Webhooks**: one endpoint per project (`POST /hook/<slug>`), auto-detects GitHub HMAC vs plain shared-secret; deploys only when push is to the configured branch (toggleable).
- **Deploy history**: paginated, latest-first; rollback to any previously-deployed commit.
- **Telemetry**: dashboard shows rusno-managed container count (labeled `rusno.project=<slug>`); CPU/memory/storage bars are host-wide via `sysinfo`.
- **Docker cleanup**: two-tier — safe prune (unused images + build cache) and nuclear prune (adds volumes), both confirmed.
- **SSH keys**: Settings toggle — rusno-managed (generates `rusno_ed25519`) or host-existing (uses `~/.ssh/id_*` + ssh-agent). Pubkey copyable.
- **GitHub token**: optional, encrypted at rest; unlocks HTTPS-private clones, webhook auto-register, repo autocomplete. Not required for any core flow.
- **Secrets at rest**: master key file (`~/.rusno/keys/master.key`, 256-bit, 0600) + AES-256-GCM per secret.
- **First run**: web setup wizard (set admin password, confirm projects root, optional GitHub token) + `rusno init` CLI for headless.
- **Concurrency**: per-project single worker with pending-coalesce (never cancel running); global semaphore default 2 concurrent deploys, configurable.
- **Health check**: after `compose up` returns 0, poll `docker compose ps` to running/healthy, 60s default timeout (per-project configurable); on failure capture `docker compose logs --tail=200`.

## Designing principle

rusno is a **single-binary, self-hosted deployment manager**. It does not try to be Kubernetes, a CI runner, or a multi-host orchestrator. Every feature should reduce to: *I can manage my host's docker-compose stacks from a browser and auto-deploy them from git.*
