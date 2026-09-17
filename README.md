# rusno

**rusno** is a self-hosted deployment manager written in Rust. It runs `docker compose` stacks on your host, exposes a web dashboard to register and manage projects, receives GitHub webhooks (or plain HTTP POSTs) for automatic deployment, and gives you per-project control over compose files, env files, branches, rollbacks, and host hygiene.

## Install

```bash
curl -fsSL https://raw.githubusercontent.com/kodin00/rusno/master/install.sh | sudo sh
```

Already have the binary? `sudo rusno service install` wires up the systemd service (auto-starts on boot). To upgrade an existing install: `sudo rusno update`.

This downloads the latest build from the [`latest` release](https://github.com/kodin00/rusno/releases/latest), installs the `rusno` binary to `/usr/local/bin`, wires up a systemd service, and runs first-run init. Then visit `http://localhost:6967` — you'll be redirected to the setup wizard. See [docs/install.md](./docs/install.md) for manual and Docker variants.

## What it does

- Register projects by git URL; rusno clones and runs `docker compose up` on each push.
- Per-project: branch, compose path, compose-up command, auto-start-on-restart toggle.
- In-browser editing of compose files and `.env` (CodeMirror 6).
- GitHub webhooks with HMAC verification, or plain shared-secret HTTP POSTs.
- Deploy history with per-commit rollback.
- Host telemetry (CPU/memory/storage), Docker cleanup (safe + nuclear prune).
- SSH key management (rusno-managed ed25519 or host-existing via ssh-agent).
- Optional GitHub token for private HTTPS clones, webhook auto-register, repo autocomplete.
- Interactive terminal dashboard: run `rusno` (no subcommand) for a live TUI of projects, running containers, host telemetry, and recent deployments.
- Self-update: `rusno update` checks GitHub for a newer release, confirms, and swaps the binary in place (restarts the systemd service if installed).

## Tech stack

Rust + axum + maud (server-rendered HTML) + HTMX + Alpine.js + CodeMirror 6 (CDN). SQLite via sqlx. Docker via bollard + compose shell-out. Host telemetry via sysinfo.

## Documentation

Full design spec lives in [`docs/`](./docs/). Read in order:

1. [Architecture](./docs/01-architecture.md)
2. [Data Model](./docs/02-data-model.md)
3. [Features](./docs/03-features.md)
4. [Deployment Pipeline](./docs/04-deployment-pipeline.md)
5. [Webhooks](./docs/05-webhooks.md)
6. [Tech Stack & Tooling](./docs/06-tech-stack.md)
7. [Implementation Phases](./docs/07-phases.md)

## Designing principle

rusno is a single-binary, self-hosted deployment manager. It does not try to be Kubernetes, a CI runner, or a multi-host orchestrator. Every feature should reduce to: *I can manage my host's docker-compose stacks from a browser and auto-deploy them from git.*
