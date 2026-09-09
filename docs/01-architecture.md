# 01 — Architecture

rusno is a single Rust binary that runs an HTTP server (axum) on the host, talks to the local Docker Engine (bollard + `docker compose` shelling-out), reads host telemetry (sysinfo), and persists state to SQLite.

## Process model

```
┌──────────────────────────────────────────────────────────────────┐
│  rusno binary (axum server, port 6967 default)                   │
│                                                                  │
│  ┌─────────────┐   ┌──────────────┐   ┌────────────────────────┐ │
│  │ HTTP routes │   │ Deploy       │   │ Host access            │ │
│  │ (axum)      │──▶│ scheduler   │──▶│ - bollard (Docker API) │ │
│  │ + HTMX UI  │   │ (tokio mpsc) │   │ - sysinfo (telemetry)  │ │
│  └─────────────┘   └──────────────┘   │ - git CLI (clone/pull) │ │
│         │                  │           │ - ssh (key-based)      │ │
│         ▼                  ▼           └────────────────────────┘ │
│  ┌─────────────┐   ┌──────────────┐                              │
│  │ SQLite      │   │ Project      │                              │
│  │ (sqlx)      │   │ workers (N)  │                              │
│  └─────────────┘   └──────────────┘                              │
└──────────────────────────────────────────────────────────────────┘
```

- **Single binary**, one process. The web server, deploy scheduler, and project workers all live in one tokio runtime.
- **No separate worker daemon.** Deploy jobs run as async tasks spawned by the scheduler.
- **HTTP server** binds `0.0.0.0:6967` by default (configurable). Behind a reverse proxy (nginx/Caddy) for TLS in production.

## Data directories

rusno separates its own files from project files.

### rusno home: `~/.rusno/`

Owned by the user running rusno. Layout:

```
~/.rusno/
├── config.toml              # port, projects_root, ssh mode, github token presence
├── rusno.db                  # SQLite database (projects, deploys, settings)
├── keys/
│   └── master.key           # 256-bit random master key (0600), for secret encryption
└── ssh/
│   ├── rusno_ed25519        # generated when ssh mode = "rusno-managed"
│   └── rusno_ed25519.pub    # copyable pubkey
```

- `config.toml` is the bootstrap config (port, projects root, ssh mode). Runtime-editable settings (admin password hash, github token, etc.) live in SQLite, not the file — the file is only read on startup.
- `master.key` is generated on first run, never leaves the host, never backed up by rusno. **Back it up separately** (documented in install guide) — losing it means all encrypted secrets become unrecoverable.

### Projects root: configurable, default `~/rusno/projects/`

```
~/rusno/projects/                       # configurable in Settings
├── my-app/                             # folder name (default = repo name, overridable)
│   ├── .git/                           # cloned repo
│   ├── docker/docker-compose.yml       # edited in-place at the path you configured
│   ├── .env                             # authored/copied from .env.example
│   └── logs/
│       └── <deploy-id>.log             # full deploy log per deploy
└── another-project/
    └── ...
```

- Per-project path = `<projects_root>/<folder_name>/`.
- The compose path you set (e.g. `docker/docker-compose.yml`) is resolved relative to the project folder.
- `.env` lives at the project folder root by default; if the compose file references a different env path, rusno edits that path instead.
- Logs are truncated-tail in SQLite + full file on disk.

## Install variants

### (a) Binary + systemd (bare-metal, recommended for dedicated hosts)

```bash
# install
sudo cp rusno /usr/local/bin/
sudo cp deploy/rusno.service /etc/systemd/system/
sudo systemctl enable --now rusno

# runs as the user specified in the unit file (default: the installing user)
```

- `rusno.service` runs `rusno serve` under a `User=` directive.
- rusno's own logs → journald (stdout/stderr captured by systemd).
- First-run setup wizard runs on first HTTP visit; for headless installs, `rusno init --admin-password ... --projects-root ...`.

### (b) Dockerized rusno

```bash
docker run -d \
  -p 6967:6967 \
  -v /var/run/docker.sock:/var/run/docker.sock \
  -v ~/.rusno:/root/.rusno \
  -v ~/rusno/projects:/root/rusno/projects \
  -v ~/.ssh:/root/.ssh:ro \
  --name rusno \
  ghcr.io/you/rusno:latest
```

Bind-mounts:
| Host path | Container path | Mode | Why |
|---|---|---|---|
| `/var/run/docker.sock` | same | rw | rusno manages host Docker |
| `~/.rusno` | `/root/.rusno` | rw | DB, keys, config |
| projects root | `/root/rusno/projects` | rw | git clones live here |
| `~/.ssh` | `/root/.ssh` | ro | SSH keys for git clone (rusno-managed key is in `~/.rusno/ssh`, so this is for host-existing mode) |

> **Note on SSH mode in dockerized rusno**: when ssh mode = `rusno-managed`, the key lives at `~/.rusno/ssh/` (rw-mounted) and `GIT_SSH_COMMAND` points there. The `~/.ssh` ro-mount is only consulted when ssh mode = `host-existing`. The `ro` flag can be promoted to `rw` if rusno needs to write `known_hosts`.

## Runtime dependencies on the host

| Dependency | Used for | Required? |
|---|---|---|
| Docker Engine + `docker compose` plugin | project deploys, cleanup, container listing | **yes** |
| `git` CLI | clone, pull, checkout for rollback | **yes** |
| `ssh` | private-repo clones over SSH | only if using SSH URLs |
| `ssh-agent` | host-existing SSH mode | only if host-existing mode + passphrase-protected keys |

`mise` is **dev-only** — it pins the Rust toolchain and dev CLIs during development. The released binary has no mise dependency at runtime.

## Config bootstrap (`~/.rusno/config.toml`)

Minimal on-disk config; everything else is in SQLite.

```toml
# generated by `rusno init` or first-run wizard
port = 6967
projects_root = "~/rusno/projects"
ssh_mode = "rusno-managed"   # or "host-existing"
data_dir = "~/.rusno"
```

- CLI flags override config: `rusno serve --port 7000`.
- Settings page writes runtime values to SQLite (`settings` table), not this file.
- `projects_root` change in Settings moves **future** projects only; existing project folders stay where they are (rusno records the absolute path per project at registration time).

## rusno's own logging

- **rusno process logs**: stdout/stderr → journald (systemd) or container stdout (docker). Structured logging via `tracing` + `tracing-subscriber`.
- **Per-deploy logs**: full output captured to `<project>/logs/<deploy-id>.log`; last 200 lines mirrored into SQLite `deploys.log_tail` for quick UI display without opening the file.
