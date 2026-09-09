-- rusno initial schema
-- All timestamps stored as RFC 3339 strings in UTC (TEXT columns).

-- Key/value table for runtime-editable global settings.
CREATE TABLE settings (
  key   TEXT PRIMARY KEY,
  value TEXT NOT NULL
);

-- Registered projects.
CREATE TABLE projects (
  id                  INTEGER PRIMARY KEY AUTOINCREMENT,
  slug                TEXT NOT NULL UNIQUE,
  display_name        TEXT NOT NULL,
  folder_name         TEXT NOT NULL,
  folder_path         TEXT NOT NULL,
  source_type         TEXT NOT NULL CHECK (source_type IN ('git_ssh','git_https')),
  source_url          TEXT NOT NULL,
  branch              TEXT NOT NULL DEFAULT 'main',
  compose_path        TEXT NOT NULL DEFAULT 'docker/docker-compose.yml',
  compose_command     TEXT NOT NULL DEFAULT 'up -d --build --remove-orphans',
  auto_start          INTEGER NOT NULL DEFAULT 1,
  webhook_secret      TEXT NOT NULL,
  webhook_enabled     INTEGER NOT NULL DEFAULT 1,
  branch_filter       INTEGER NOT NULL DEFAULT 1,
  health_timeout_secs INTEGER NOT NULL DEFAULT 60,
  created_at          TEXT NOT NULL,
  updated_at          TEXT NOT NULL
);

-- Deploy records.
CREATE TABLE deploys (
  id           INTEGER PRIMARY KEY AUTOINCREMENT,
  project_id   INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
  commit_sha   TEXT,
  commit_msg   TEXT,
  trigger      TEXT NOT NULL CHECK (trigger IN ('webhook_github','webhook_plain','manual','rollback','restart')),
  status       TEXT NOT NULL CHECK (status IN ('queued','pulling','building','starting','healthy','succeeded','failed')),
  log_tail     TEXT,
  log_path     TEXT,
  started_at   TEXT NOT NULL,
  finished_at  TEXT,
  is_rollback  INTEGER NOT NULL DEFAULT 0,
  error        TEXT
);

CREATE INDEX idx_deploys_project_started ON deploys(project_id, started_at DESC);
CREATE INDEX idx_deploys_latest ON deploys(started_at DESC);

-- Session table for signed-cookie auth.
CREATE TABLE sessions (
  id           TEXT PRIMARY KEY,
  created_at   TEXT NOT NULL,
  expires_at   TEXT NOT NULL,
  csrf_token   TEXT NOT NULL
);

CREATE INDEX idx_sessions_expires ON sessions(expires_at);

-- SSH key display cache (not authoritative — rescanned on Settings load).
CREATE TABLE ssh_keys (
  fingerprint TEXT PRIMARY KEY,
  path        TEXT NOT NULL,
  type        TEXT NOT NULL,
  mode        TEXT NOT NULL CHECK (mode IN ('rusno-managed','host-existing')),
  has_pub     INTEGER NOT NULL DEFAULT 1,
  created_at  TEXT NOT NULL
);
