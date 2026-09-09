# 02 — Data Model

SQLite schema sketch, project folder layout, and secrets handling. This is the persistence contract the implementation follows.

## Database overview

- Single SQLite file at `~/.rusno/rusno.db`.
- Managed by `sqlx` with migrations under `migrations/` (run via `sqlx migrate`).
- WAL mode enabled for concurrent reads during deploys.
- All timestamps stored as RFC 3339 strings in UTC (TEXT column) — portable, human-readable.

## Schema sketch

### `settings`

Key/value table for runtime-editable global settings (everything that's *not* bootstrap config in `config.toml`).

```sql
CREATE TABLE settings (
  key   TEXT PRIMARY KEY,
  value TEXT NOT NULL
);
```

Known keys (code validates on write):
| key | value | default |
|---|---|---|
| `admin_password_hash` | argon2id hash | (set on first run) |
| `session_secret` | 256-bit random, used to sign session cookies | (generated on first run) |
| `rusno_url` | external URL/domain, e.g. `https://rusno.example.com` | empty |
| `projects_root` | absolute path, overrides `config.toml` | `~/rusno/projects` |
| `ssh_mode` | `rusno-managed` \| `host-existing` | `rusno-managed` |
| `github_token_encrypted` | AES-256-GCM ciphertext (nonce+ct in one blob) | NULL |
| `deploy_concurrency` | integer, global semaphore cap | `2` |
| `default_health_timeout_secs` | integer | `60` |

### `projects`

```sql
CREATE TABLE projects (
  id              INTEGER PRIMARY KEY AUTOINCREMENT,
  slug             TEXT NOT NULL UNIQUE,             -- URL-safe, used in /hook/<slug>
  display_name    TEXT NOT NULL,                     -- human name, default = repo name
  folder_name     TEXT NOT NULL,                     -- on-disk folder, default = repo name
  folder_path     TEXT NOT NULL,                     -- absolute path, resolved at registration
  source_type     TEXT NOT NULL CHECK (source_type IN ('git_ssh','git_https')),
  source_url      TEXT NOT NULL,                     -- git URL, clone target
  branch          TEXT NOT NULL DEFAULT 'main',      -- target deploy branch
  compose_path    TEXT NOT NULL,                     -- relative to folder_path, e.g. "docker/docker-compose.yml"
  compose_command TEXT NOT NULL DEFAULT 'up -d --build --remove-orphans',
  auto_start      INTEGER NOT NULL DEFAULT 1,        -- 0/1, re-up on rusno start
  webhook_secret  TEXT NOT NULL,                     -- random per-project, plaintext (used for HMAC/verification)
  webhook_enabled INTEGER NOT NULL DEFAULT 1,        -- 0/1, disable webhook + http deployment
  branch_filter   INTEGER NOT NULL DEFAULT 1,        -- 0/1, only deploy on push to configured branch
  health_timeout_secs INTEGER NOT NULL DEFAULT 60,
  created_at      TEXT NOT NULL,
  updated_at      TEXT NOT NULL
);
```

Notes:
- `webhook_secret` is stored as plaintext — it's a shared secret that must be compared in constant time on the webhook path anyway, and encryption buys nothing here (rusno needs the raw value to HMAC). It's distinct from the encrypted GitHub token, which *is* sensitive.
- `slug` is derived from `folder_name` (lowercase, `[a-z0-9-]`), uniqueness-enforced. If it collides, rusno appends `-2`, `-3`, etc.
- `folder_path` is `<projects_root>/<folder_name>`, frozen at registration. Changing `projects_root` in Settings affects only future projects.

### `deploys`

```sql
CREATE TABLE deploys (
  id           INTEGER PRIMARY KEY AUTOINCREMENT,
  project_id   INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
  commit_sha   TEXT,                                    -- NULL until pull succeeds
  commit_msg   TEXT,
  trigger      TEXT NOT NULL CHECK (trigger IN ('webhook_github','webhook_plain','manual','rollback','restart')),
  status       TEXT NOT NULL CHECK (status IN ('queued','pulling','building','starting','healthy','succeeded','failed')),
  log_tail     TEXT,                                    -- last ~200 lines, for quick UI display
  log_path     TEXT,                                    -- absolute path to full log file
  started_at   TEXT NOT NULL,
  finished_at  TEXT,
  is_rollback  INTEGER NOT NULL DEFAULT 0,
  error        TEXT
);

CREATE INDEX idx_deploys_project_started ON deploys(project_id, started_at DESC);
CREATE INDEX idx_deploys_latest ON deploys(started_at DESC);
```

- `log_tail` is a truncated mirror so the deployments list can show context without file I/O.
- `log_path` points to `<folder_path>/logs/<id>.log` — full stdout/stderr from git + docker compose.
- The deployments tab queries `ORDER BY started_at DESC LIMIT 25 OFFSET ?` for pagination.

### `sessions`

```sql
CREATE TABLE sessions (
  id           TEXT PRIMARY KEY,                       -- random session id
  created_at   TEXT NOT NULL,
  expires_at   TEXT NOT NULL,
  csrf_token   TEXT NOT NULL                           -- per-session CSRF for non-GET HTMX requests
);

CREATE INDEX idx_sessions_expires ON sessions(expires_at);
```

- Cookie holds `id` signed with `session_secret`. Server validates signature + expiry + DB row.
- Sliding 7-day expiry: each successful request bumps `expires_at`.
- CSRF: every state-changing HTMX request must send `X-CSRF-Token` matching the session's `csrf_token`; the token is rendered into the page as a meta tag.

### `ssh_keys` (display cache, not authoritative)

```sql
CREATE TABLE ssh_keys (
  fingerprint TEXT PRIMARY KEY,
  path         TEXT NOT NULL,                          -- e.g. ~/.ssh/id_ed25519
  type         TEXT NOT NULL,                          -- ed25519, rsa, ecdsa
  mode         TEXT NOT NULL CHECK (mode IN ('rusno-managed','host-existing')),
  has_pub      INTEGER NOT NULL DEFAULT 1,
  created_at   TEXT NOT NULL
);
```

- Populated by scanning `~/.ssh/` (host-existing) plus the rusno-managed key at `~/.rusno/ssh/`.
- Re-scanned on Settings page load; not a source of truth.
- The rusno-managed row's `path` = `~/.rusno/ssh/rusno_ed25519`.

## Project folder layout (on disk)

```
<projects_root>/                          # default ~/rusno/projects/
└── <folder_name>/                        # default = repo name, overridable
    ├── .git/                              # cloned on first deploy
    ├── <compose_path>                     # e.g. docker/docker-compose.yml — edited in-place
    ├── .env                               # authored via editor or copied from .env.example
    ├── .env.example                       # from the repo, if present
    └── logs/
        └── <deploy_id>.log                # full deploy log
```

- rusno clones into `folder_path` (not a bare repo — a working tree on the configured branch).
- The compose path is resolved as `folder_path / compose_path`. If the file doesn't exist after clone, the editor opens empty and saving creates it at that path (including parent dirs).
- `.env` path: rusno defaults to `folder_path/.env`. If the compose file sets `env_file:` to a different relative path, rusno detects this and edits *that* path instead. This keeps the editor honest about which file the compose actually loads.

## Secrets handling

### Master key

- `~/.rusno/keys/master.key` — 32 random bytes, generated on first run, file mode `0600`, owned by the rusno user.
- Loaded into memory once at startup; never written elsewhere; not logged.
- **Backup is the operator's job** — documented in install guide. Loss = all encrypted secrets unrecoverable.

### AES-256-GCM per secret

- Each secret value is encrypted with a fresh 12-byte nonce.
- Storage format: `base64(nonce || ciphertext || tag)`.
- Applied to: `github_token` in `settings`.
- `webhook_secret` is **not** encrypted (see rationale in `projects` notes above) — it's a verifier, not a credential rusno uses against an external service.

### SSH keys

Two modes (Settings toggle):

- **`rusno-managed`**: rusno generates an ed25519 keypair at `~/.rusno/ssh/rusno_ed25519(.pub)`. Clones use `GIT_SSH_COMMAND="ssh -i ~/.rusno/ssh/rusno_ed25519 -o IdentitiesOnly=yes -o StrictHostKeyChecking=accept-new"`. Pubkey is displayed + copyable in Settings.
- **`host-existing`**: rusno uses the host's `~/.ssh/id_*` keys via `ssh-agent`. Settings scans `~/.ssh/`, lists keys with fingerprints, and shows which is in the agent. No key generation; rusno just runs plain `git clone` (agent handles auth).

Mode switch is a runtime setting; rusno doesn't move or delete keys when switching.

## Migration strategy

- `sqlx migrate add` creates versioned SQL files under `migrations/`.
- rusno runs pending migrations on startup (fail-fast if a migration errors).
- No auto-rollback of schema; migrations are forward-only. A bad migration is a bug to fix with a new forward migration.
