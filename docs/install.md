# Installing rusno

Two install variants: (a) single binary + systemd, (b) dockerized rusno with host socket bind-mounted.

## Prerequisites (both variants)

| Dependency | Required? |
|---|---|
| Docker Engine + `docker compose` plugin | **yes** |
| `git` CLI | **yes** |
| `ssh` | only for SSH-URL clones |
| `ssh-agent` | only for host-existing SSH mode with passphrase keys |

## Variant (a): Binary + systemd (bare-metal, recommended)

```bash
# 1. Get the binary (build locally or download a release)
cargo build --release
sudo cp target/release/rusno /usr/local/bin/

# 2. Install the systemd service
sudo cp deploy/rusno.service /etc/systemd/system/
sudo sed -i "s/REPLACE_USER/$(whoami)/" /etc/systemd/system/rusno.service
sudo systemctl daemon-reload
sudo systemctl enable --now rusno

# 3. First-run setup
# Visit http://localhost:6967 — you'll be redirected to the setup wizard.
# For headless installs:
rusno init --admin-password 'your-password' --projects-root ~/rusno/projects
```

Or use the bundled installer:

```bash
cargo build --release
./install.sh
```

## Variant (b): Dockerized rusno

```bash
docker build -t rusno .

docker run -d \
  -p 6967:6967 \
  -v /var/run/docker.sock:/var/run/docker.sock \
  -v ~/.rusno:/root/.rusno \
  -v ~/rusno/projects:/root/rusno/projects \
  -v ~/.ssh:/root/.ssh:ro \
  --name rusno \
  rusno:latest
```

Bind-mounts:

| Host path | Container path | Mode | Why |
|---|---|---|---|
| `/var/run/docker.sock` | same | rw | rusno manages host Docker |
| `~/.rusno` | `/root/.rusno` | rw | DB, keys, config |
| projects root | `/root/rusno/projects` | rw | git clones live here |
| `~/.ssh` | `/root/.ssh` | ro | SSH keys for git clone (host-existing mode) |

When ssh mode = `rusno-managed`, the key lives at `~/.rusno/ssh/` (rw-mounted) and `GIT_SSH_COMMAND` points there. The `~/.ssh` ro-mount is only consulted in host-existing mode.

## ⚠️ Back up your master key

`~/.rusno/keys/master.key` is generated on first run, never leaves the host, and is never backed up by rusno. **Losing it means all encrypted secrets (e.g. the GitHub token) become unrecoverable.** Back it up separately and securely.

## What gets created

```
~/.rusno/
├── config.toml        # bootstrap config (port, projects root, ssh mode)
├── rusno.db           # SQLite database
├── keys/
│   └── master.key     # 256-bit master key (0600) — BACK THIS UP
└── ssh/
    └── rusno_ed25519  # generated when ssh mode = rusno-managed
```

## Behind a reverse proxy

rusno binds `0.0.0.0:6967` by default. Put nginx/Caddy in front for TLS:

```
# Caddyfile
rusno.example.com {
    reverse_proxy localhost:6967
}
```

Set the external URL in **Settings → rusno URL** so webhook URLs display correctly.
