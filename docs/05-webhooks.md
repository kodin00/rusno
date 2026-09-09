# 05 — Webhooks

rusno accepts deployment triggers over HTTP. Each project gets one endpoint that auto-detects GitHub webhook format vs. plain shared-secret POST.

## Endpoint

```
POST /hook/<project-slug>
```

- `<project-slug>` is the project's URL-safe slug (derived from folder name, `[a-z0-9-]`).
- If the project has `webhook_enabled=0`, the endpoint returns **404** (the project exists but webhooks are disabled).
- If the project doesn't exist, also **404** — rusno does not reveal which slugs are registered.

## Auto-detection

rusno inspects the request headers to decide verification mode:

| Header present | Mode | Verification |
|---|---|---|
| `X-Hub-Signature-256` | GitHub HMAC | HMAC-SHA256 of the body using `webhook_secret`, constant-time compare |
| `X-Rusno-Secret` (or `X-Webhook-Secret`) | Plain | Constant-time compare of the header value against `webhook_secret` |
| Neither | Reject | **401 Unauthorized** |

Notes:
- GitHub mode also requires `X-GitHub-Event: push` (any other event → `200 OK` with body `{"status":"ignored","reason":"event not push"}` — this prevents GitHub from retrying).
- Plain mode: the secret is in a header, not the body, so the body can carry arbitrary metadata that rusno ignores (a plain "deploy now" POST with no body is fine).
- The same `webhook_secret` works for both modes — it's one secret per project, shown in the project detail page, copy-only.

## Verification detail

### GitHub HMAC

```
signature = "sha256=" + HMAC-SHA256(body, webhook_secret)
```

- rusno reads the full request body into memory (cap 10 MB — GitHub payloads are small JSON).
- Computes `HMAC-SHA256(body, webhook_secret)`, hex-encodes, prepends `sha256=`, compares with the header value using `subtle::ConstantTimeEq`.
- On mismatch: **401 Unauthorized** with `{"status":"error","reason":"bad signature"}`.
- On match: proceeds to branch filtering.

### Plain shared-secret

- Header `X-Rusno-Secret` (preferred) or `X-Webhook-Secret` (alias) must equal `webhook_secret`, compared in constant time.
- On mismatch: **401 Unauthorized**.
- On match: proceeds directly to deploy trigger (no branch filter — plain mode always deploys).

## Branch filter (GitHub mode)

When a GitHub push event verifies, rusno checks whether the push is to the project's configured branch:

- `branch_filter=1` (default): parse the JSON body for `ref`. It comes as `refs/heads/<branch>`. If `<branch>` != the project's `branch` setting → **200 OK** with `{"status":"ignored","reason":"branch mismatch"}` (no deploy). This is the "only merges/pushes to the selected branch" behavior.
- `branch_filter=0`: any push triggers a deploy, regardless of branch.

Plain mode skips branch filtering entirely — you POST to deploy, and rusno deploys the configured branch.

## Trigger flow

After verification + filtering passes:

1. rusno enqueues a deploy with `trigger='webhook_github'` or `trigger='webhook_plain'` on the project's worker channel.
2. The HTTP response returns **200 OK** immediately with `{"status":"queued","deploy_id":<id>}` — the actual deploy runs asynchronously (git pull + compose up may take minutes).
3. The deploy progress is visible in the Deployments tab (and the project detail's history section).

This non-blocking response is important: GitHub retries webhooks that don't return 2xx within 10s. rusno must always return fast.

## Auto-register (optional, GitHub token only)

When a GitHub token is stored and the user registers a project from a `git@github.com:owner/repo.git` or `https://github.com/owner/repo.git` URL, rusno offers to auto-register the webhook on GitHub's side:

- Calls GitHub API `POST /repos/{owner}/{repo}/hooks` with:
  - `config.url = <rusno_url>/hook/<slug>`
  - `config.content_type = "json"`
  - `config.secret = <webhook_secret>`
  - `events = ["push"]`
- If `rusno_url` is unset, rusno warns and skips auto-register (the user must set the URL in Settings first).
- This is a checkbox on the register form: "Auto-register webhook on GitHub" (only visible when token is set + URL is a GitHub repo).
- On failure (404 repo, 403 no permission): the project is still created; rusno shows the webhook URL + secret for manual setup.

## Webhook URL display

In the project detail page, the webhook section shows:

```
URL:    https://rusno.example.com/hook/my-app     [copy]
Secret: 7fKx9b...                                [copy]
```

- URL is constructed from `settings.rusno_url` + `/hook/` + slug. If `rusno_url` is empty, it shows `/hook/<slug>` (relative) with a warning: "Set rusno URL in Settings for a complete webhook URL."
- Secret is the raw `webhook_secret`, copy-only (no edit field — changing it would break registered webhooks; if rotation is needed later, it's a v1.1 feature).
- Both are masked by default with a "reveal" toggle (so screen-sharers don't leak the secret).

## Security notes

- The webhook endpoint is **unauthenticated** by design (GitHub can't log in) — verification is purely signature/secret-based.
- Rate limiting: per-slug, max 10 triggers/minute; excess triggers return **429** without enqueueing. This prevents a misconfigured GitHub loop from hammering the queue.
- Body size capped at 10 MB; oversized → **413**.
- rusno logs webhook deliveries (slug, mode, branch if applicable, deploy_id or ignored reason) but never logs the secret.
