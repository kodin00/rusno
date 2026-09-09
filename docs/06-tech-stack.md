# 06 — Tech Stack & Tooling

The crates, the mise setup, the build, and the release. mise is **dev-only** — it pins the Rust toolchain and dev CLIs so every contributor gets the same versions. The released binary has no runtime mise dependency.

## Crate selection

### Core web + async
| Crate | Version hint | Purpose |
|---|---|---|
| `tokio` | 1.x, features `full` | async runtime |
| `axum` | 0.7+ | HTTP server, routing, middleware |
| `tower` | 0.5+ | middleware (auth, logging) |
| `tower-http` | 0.6+ | static files, compression, CORS, trace |

### Templating & UI
| Crate | Purpose |
|---|---|
| `maud` | type-safe HTML templates in Rust macros — all server-rendered UI |
| HTMX (CDN) | client-side partial swaps, no JS build |
| Alpine.js (CDN) | tiny interactive bits (modals, toggles) |
| CodeMirror 6 (CDN) | in-browser compose/`.env` editor |

> No `node`, no bundler. The three JS deps are loaded from CDN in a single `<script>`/`<link>` block in the base template. Offline use is a v1.1 concern (vendoring).

### Persistence
| Crate | Purpose |
|---|---|
| `sqlx` | SQLite driver, compile-time checked queries, migrations — features `runtime-tokio`, `sqlite`, `chrono` |
| `chrono` | timestamp types (stored as RFC 3339 TEXT) |

### Docker & system
| Crate | Purpose |
|---|---|
| `bollard` | Docker Engine API (list containers, `system df`, prune) |
| `sysinfo` | CPU / memory / disk telemetry |
| `duct` or `tokio::process::Command` | shelling out to `git`, `docker compose`, `ssh-keygen` |

### Crypto & auth
| Crate | Purpose |
|---|---|
| `argon2` | admin password hashing |
| `aes-gcm` | AES-256-GCM for GitHub token encryption |
| `subtle` | constant-time comparisons (webhook secret + HMAC) |
| `ring` or `hmac` + `sha2` | HMAC-SHA256 for GitHub webhook verification |
| `rand` | key + secret generation |
| `cookie` | signed session cookies |

### Config & logging
| Crate | Purpose |
|---|---|
| `config` | read `~/.rusno/config.toml` + env overrides |
| `tracing` + `tracing-subscriber` | structured logs → stdout/journald |
| `anyhow` | error propagation in bin layer |
| `thiserror` | typed errors in lib layer |

### GitHub API (optional, when token present)
| Crate | Purpose |
|---|---|
| `reqwest` | GitHub REST calls (webhook auto-register, repo autocomplete) |
| `serde` + `serde_json` | request/response bodies + webhook payload parsing |

## mise setup (`.mise.toml`)

Lives at repo root. Pinned so `mise install` gives a reproducible dev env.

```toml
[tools]
rust = "1.81.0"
"cargo:cargo-watch" = "latest"
"cargo:sqlx-cli" = "latest"
"cargo:cargo-edit" = "latest"
"cargo:cargo-audit" = "latest"

[env]
DATABASE_URL = "sqlite://~/.rusno/rusno.db"
RUST_LOG = "rusno=debug,info"
```

- Rust version pinned to a specific stable (update deliberately).
- Dev CLIs: `cargo-watch` (auto-rebuild on save), `sqlx-cli` (migrations + `cargo sqlx prepare`), `cargo-edit` (`cargo add`), `cargo-audit` (advisory scanning).
- `DATABASE_URL` env so `sqlx` query macros compile-check against the real schema during build (sqlx offline mode for CI later).

New contributor flow:
```bash
git clone <rusno>
cd rusno
mise install        # installs pinned Rust + dev CLIs
cargo sqlx migrate run   # sets up dev DB
cargo run           # starts rusno on :6967
```

## Project layout (cargo workspace)

```
rusno/
├── .mise.toml
├── Cargo.toml
├── README.md
├── docs/                       # this folder
├── migrations/                 # sqlx migrations
│   └── 0001_initial.sql
└── src/
    ├── main.rs                 # CLI (clap), starts server
    ├── config.rs               # config.toml + settings table
    ├── db.rs                   # pool, migrations
    ├── models/                 # sqlx row types
    ├── crypto.rs               # master key, AES-GCM, argon2
    ├── auth.rs                 # session, CSRF
    ├── deploy/
    │   ├── mod.rs              # scheduler, state machine
    │   ├── worker.rs           # per-project worker
    │   └── git.rs              # clone/pull/checkout
    ├── docker.rs               # bollard wrapper, compose shell-out
    ├── webhooks.rs             # /hook/<slug> handler
    ├── routes/
    │   ├── dashboard.rs
    │   ├── projects.rs
    │   ├── deployments.rs
    │   ├── settings.rs
    │   └── auth.rs
    └── templates/              # maud modules
        ├── layout.rs           # base HTML + nav
        ├── dashboard.rs
        ├── projects.rs
        ├── deployments.rs
        └── settings.rs
```

`main.rs` uses `clap` for `serve`, `init`, `migrate` subcommands.

## Build

```bash
# dev
cargo watch -x run

# release (static-ish, dynamic libc)
cargo build --release
# binary at target/release/rusno

# musl static (for portable binary; cross-rs or musl target)
cargo build --release --target x86_64-unknown-linux-musl
```

- CI: GitHub Actions matrix builds `x86_64-unknown-linux-gnu` + `aarch64-unknown-linux-gnu` + `aarch64-apple-darwin` (dev/secondary target).
- sqlx offline mode: `cargo sqlx prepare -- --bin rusno` commits `.sqlx/` so CI builds without a DB.
- Release artifacts: the binary + a sample `rusno.service` systemd unit + an install script.

## Release variants

| Artifact | Contents |
|---|---|
| `rusno-x86_64-unknown-linux-gnu` | binary for most Linux hosts |
| `rusno-aarch64-unknown-linux-gnu` | binary for ARM Linux (Raspberry Pi, Ampere) |
| `rusno-aarch64-apple-darwin` | binary for macOS dev/test (not primary target) |
| `rusno.service` | systemd unit template |
| `install.sh` | downloads binary, installs service, runs `rusno init` prompt |
| Docker image `ghcr.io/you/rusno:latest` | multi-arch, `linux/amd64` + `linux/arm64` |

## Testing strategy

- Unit: pure logic (state machine transitions, slug derivation, AES round-trip, branch-filter parsing).
- Integration: a `docker-compose.yml` in `tests/fixtures/` that rusno actually deploys against a local Docker socket in CI (guarded behind a `#[cfg(feature = "integration")]` flag, skipped if `DOCKER_HOST` unset).
- Webhook: a test that posts a signed GitHub payload and asserts a deploy is enqueued.

## Linting

- `cargo clippy -- -D warnings` in CI.
- `cargo fmt --check` in CI.
- `cargo audit` for advisories (via `cargo-audit` in mise).
