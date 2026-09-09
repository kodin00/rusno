# 04 — Deployment Pipeline

How a deploy goes from trigger to finished state. Covers the state machine, scheduling rules, health checks, rollback, and rusno's startup behavior.

## State machine

Each deploy row moves through these states, persisted to `deploys.status`:

```
queued ─▶ pulling ─▶ building ─▶ starting ─▶ healthy ─▶ succeeded
   │         │           │            │           │
   │         ▼           ▼            ▼           ▼
   └─────────┴───────────┴────────────┴──────────▶ failed
```

| State | Meaning | What rusno is doing |
|---|---|---|
| `queued` | Deploy requested, waiting for a worker slot | In the global queue; visible in UI as amber |
| `pulling` | Fetching latest from git | `git fetch` + `git checkout <branch>` + `git pull --ff-only` on the project folder |
| `building` | Running compose build/pull | `docker compose -f <path> <command>` started (e.g. `up -d --build`) |
| `starting` | compose up returned 0, waiting for containers to be ready | Polling `docker compose ps` |
| `healthy` | All containers `running` (or `healthy` if healthchecks defined) | Transition state, brief |
| `succeeded` | Final success | Deploy row closed, `finished_at` set |
| `failed` | Final failure | `error` column populated, `log_tail` has the relevant output |

### Transitions

- Forward only; no backward transitions except a new deploy row starting fresh.
- A `failed` deploy never auto-retries. The user triggers a new deploy (manual or via webhook).
- `building` → `starting` happens when `docker compose up` exits 0.
- `starting` → `healthy` happens when all project containers report `running`/`healthy`.
- `starting` → `failed` happens on health timeout (see below).
- Any unexpected error (git fails, compose exits non-zero) → `failed` with the captured stderr in `error` + `log_tail`.

### Commit tracking

- After `pulling` succeeds, rusno records `git rev-parse HEAD` into `deploys.commit_sha` and the commit subject line into `commit_msg`.
- This is what populates the rollback picker — only commits with a succeeded deploy row are eligible.

## Scheduling

### Per-project worker

Each project has a dedicated single-worker channel (tokio `mpsc`, capacity 1). This guarantees:

- **At most one deploy running per project** — no two builds clobber the same folder.
- **Pending coalesce**: if a deploy is running and ≥1 new triggers arrive, only the **latest** queued trigger survives; earlier queued ones are dropped. This handles "push twice fast" without piling up.
- **Never cancel a running deploy** — a build in progress runs to completion even if a newer push arrives. The newer push becomes the single pending entry.

### Global concurrency cap

- A global semaphore (`tokio::sync::Semaphore`, permits = `settings.deploy_concurrency`, default 2) gates how many projects can deploy simultaneously across the whole host.
- A project worker acquires a permit before transitioning `queued → pulling`; releases it on terminal state (`succeeded`/`failed`).
- Projects above the cap sit in `queued` until a permit frees.
- Configurable in Settings; raising it on a beefy host allows more parallel builds, lowering it protects a small host's CPU/RAM.

### Trigger sources

All five trigger types feed the same per-project worker:

| `trigger` value | Origin |
|---|---|
| `webhook_github` | GitHub push event via `POST /hook/<slug>` (HMAC-verified) |
| `webhook_plain` | Manual HTTP POST to the same endpoint (shared-secret-verified) |
| `manual` | "Deploy now" button in the UI |
| `rollback` | Rollback action from a prior deploy row |
| `restart` | rusno startup, for projects with `auto_start=1` |

## Deploy steps (in order)

1. **Acquire global permit** (or wait in `queued`).
2. **Git sync** (`pulling`):
   - If `folder_path` doesn't exist: `git clone <source_url> <folder_path>`.
   - If it exists: `git fetch origin && git checkout <branch> && git pull --ff-only`.
   - On a non-fast-forward or dirty tree: rusno stashes local changes (`git stash`), retries pull; if still failing, `failed` with the git error. (Local edits to tracked files are discouraged — the compose/`.env` editors write to specific paths, but if a repo has the compose tracked and you edited it in-browser, rusno's editor saves are meant to coexist with git; see "compose and git" below.)
3. **Record commit** — `git rev-parse HEAD` → `deploys.commit_sha`, subject → `commit_msg`.
4. **Compose up** (`building`):
   - Command: `docker compose -f <folder_path>/<compose_path> <compose_command>`.
   - `compose_command` is space-split (no shell), validated at registration to start with `up`. Example: `up -d --build --remove-orphans`.
   - stdout/stderr captured line-by-line to `<folder_path>/logs/<deploy_id>.log` and mirrored (tail) to `deploys.log_tail`.
5. **Health check** (`starting` → `healthy`):
   - Poll `docker compose ps` every 2s.
   - Success: every service container is `running`, and if any has a healthcheck, it's `healthy`.
   - Timeout: `settings.default_health_timeout_secs` (default 60s) or the project's `health_timeout_secs` override.
   - On timeout: `failed`, and `docker compose logs --tail=200` is appended to the deploy log + `log_tail`.
6. **Terminal** (`succeeded`/`failed`): set `finished_at`, release the global permit, update the UI via the next poll.

### Compose and git

The compose file and `.env` live **inside** the cloned repo working tree. If they're git-tracked, your in-browser edits will show as local modifications; `git pull --ff-only` may then fail. rusno handles this by stashing before pull. The recommended pattern (documented for users) is: if you want your compose/`.env` edits to survive cleanly across pulls, either commit them to the repo or add them to `.gitignore`. rusno doesn't auto-commit anything.

## Rollback

- Picker shows previously-deployed commits: `SELECT DISTINCT commit_sha, commit_msg, started_at FROM deploys WHERE project_id=? AND status='succeeded' ORDER BY started_at DESC LIMIT 25`.
- Selecting one triggers a new deploy row with `trigger='rollback'`, `is_rollback=1`.
- Steps are the same as a normal deploy, except step 2 becomes `git fetch origin && git checkout <commit_sha>` (detached HEAD) — no pull, no branch checkout.
- The deploy log clearly marks it as a rollback with the target SHA.
- Rollback doesn't change the configured branch — the next normal deploy (webhook/manual) will `git checkout <branch> && git pull` again, moving forward from `main`.

## rusno startup (auto-start)

On rusno start:

1. Load config, run migrations, load master key.
2. Start the HTTP server.
3. For every project where `auto_start=1`: enqueue a deploy with `trigger='restart'`.
   - These go through the same scheduler + global cap, so on a host with 10 auto-start projects and cap=2, they come up 2 at a time.
   - The restart deploy skips git pull if the folder already exists and HEAD matches the last succeeded `commit_sha` (optimization: avoid re-pulling if nothing changed). If HEAD differs, it does a normal pull.
   - Compose up still runs (this is the "override the default docker compose restart policy" behavior — rusno brings stacks up itself rather than relying on per-container `restart: always`).
4. Projects with `auto_start=0` are left untouched — whatever state their containers were in (stopped/running) persists.

This means a host reboot + `systemctl start rusno` (or docker container restart) brings up every auto-start stack without depending on the compose files' own restart policies.

## Log capture

- Each deploy writes to `<folder_path>/logs/<deploy_id>.log`.
- Capture: rusno spawns git/docker as child processes with `stdout`+`stderr` piped; each line is both written to the file and appended to an in-memory ring buffer (last 200 lines).
- At terminal state, the ring buffer is flushed to `deploys.log_tail`.
- The deployments UI reads `log_tail` for the inline view; the "full log" link serves the raw file via an auth-gated route.
- In-flight deploys: the log view polls an SSE-like endpoint (`GET /deployments/<id>/log?from=<offset>`) every 5s for incremental lines.

## Failure handling

- Git failures (clone, fetch, pull): `failed` with git's stderr in `error`.
- Compose non-zero exit: `failed` with the exit code + last 200 lines in `log_tail`.
- Health timeout: `failed` with "health check timed out after Ns" + `docker compose logs --tail=200`.
- rusno process crash mid-deploy: on restart, rusno scans for `deploys` in non-terminal states (`queued`/`pulling`/`building`/`starting`), marks them `failed` with `error="rusno restarted mid-deploy"`, and does not auto-retry (operator must trigger again). This prevents zombie deploys after a crash.
