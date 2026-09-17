//! Self-update: check GitHub for a newer rusno release and swap the binary in place.
//!
//! The release mechanism is a rolling GitHub release tagged `latest` on
//! `kodin00/rusno` (see `install.sh` and `.github/workflows/ci.yml`). The
//! release publishes three assets under `.../releases/latest/download/`:
//!
//! - `rusno-linux-x86_64` — the compiled binary
//! - `SHA256SUMS`         — `<sha256>  rusno-linux-x86_64`
//! - `VERSION`            — plain-text version string, e.g. `0.2.0`
//!
//! `check_for_update` fetches `VERSION`, compares it to `CARGO_PKG_VERSION`
//! using semver, and returns the newer version when one exists. `run_update`
//! drives the full flow: confirm with the user (when interactive), download
//! the binary + checksum, verify the SHA256, atomically swap the binary in
//! place, and best-effort restart the systemd service.
//!
//! Network/parse errors degrade to "no update" rather than propagating as
//! `Err` — a self-update check should never break the CLI. Only a real bug
//! (download corrupted, checksum mismatch, cannot write the binary) surfaces
//! as `Err`.

use std::io::Write;
use std::path::Path;

use anyhow::{Context, Result};
use semver::Version;
use sha2::{Digest, Sha256};

/// Base URL for all rolling-`latest` release assets.
const RELEASE_BASE: &str = "https://github.com/kodin00/rusno/releases/latest/download";

/// Info about the latest published release.
#[derive(Debug, Clone)]
pub struct ReleaseInfo {
    /// Latest version string, e.g. "0.2.0" (no leading 'v').
    pub version: String,
}

/// Query the latest release. Returns `Ok(None)` when already up-to-date or the
/// version can't be determined (network/parse errors degrade to None).
pub async fn check_for_update() -> Result<Option<ReleaseInfo>> {
    let current = match parse_version(env!("CARGO_PKG_VERSION")) {
        Some(v) => v,
        // Should never happen: CARGO_PKG_VERSION is always valid semver.
        None => return Ok(None),
    };

    // Fetch the VERSION asset. Network failures are non-fatal: a flaky
    // connection or a missing asset should not surface as a CLI error.
    let latest_text = match reqwest::get(format!("{RELEASE_BASE}/VERSION")).await {
        Ok(resp) => match resp.text().await {
            Ok(t) => t,
            Err(e) => {
                tracing::debug!(error = %e, "reading VERSION asset");
                return Ok(None);
            }
        },
        Err(e) => {
            tracing::debug!(error = %e, "fetching VERSION asset");
            return Ok(None);
        }
    };

    let latest_text = latest_text.trim();
    let latest = match parse_version(latest_text) {
        Some(v) => v,
        None => {
            tracing::warn!(raw = %latest_text, "could not parse latest VERSION as semver");
            return Ok(None);
        }
    };

    if latest > current {
        Ok(Some(ReleaseInfo {
            version: latest.to_string(),
        }))
    } else {
        Ok(None)
    }
}

/// Check for a newer release; if one exists and (interactive && user confirms,
/// or !interactive) download, verify, swap the binary, and best-effort restart
/// the systemd service. Returns Ok(()) if already up-to-date or the update
/// completed. Returns Err on download/verify/swap failure.
pub async fn run_update(interactive: bool) -> Result<()> {
    let current = env!("CARGO_PKG_VERSION");

    let Some(info) = check_for_update().await? else {
        println!("rusno is up to date (v{current}).");
        return Ok(());
    };
    let latest = info.version.clone();

    if interactive {
        // Prompt without a trailing newline so the cursor sits on the same
        // line as the answer — needs an explicit stdout flush.
        print!("rusno {current} → {latest} is available. Update now? [y/N] ");
        let _ = std::io::stdout().flush();
        let mut line = String::new();
        if std::io::stdin().read_line(&mut line).is_err() {
            println!("Skipped (could not read stdin).");
            return Ok(());
        }
        // Proceed only on an answer that starts with 'y' (case-insensitive).
        // Anything else — empty line, 'n', "no" — aborts.
        if !line.trim().to_ascii_lowercase().starts_with('y') {
            println!("Skipped.");
            return Ok(());
        }
    }

    // Only the linux-x86_64 asset is published today; anything else has to
    // build from source.
    let (os, arch) = (std::env::consts::OS, std::env::consts::ARCH);
    if !matches!((os, arch), ("linux", "x86_64")) {
        anyhow::bail!(
            "self-update currently ships a linux-x86_64 binary; on {os}-{arch} \
             build from source: cargo install ... or see install.sh"
        );
    }
    let asset = format!("rusno-{os}-{arch}");

    // Download the new binary.
    let bytes = reqwest::get(format!("{RELEASE_BASE}/{asset}"))
        .await
        .with_context(|| format!("downloading {asset}"))?
        .bytes()
        .await
        .with_context(|| format!("reading {asset} response body"))?;

    // Download SHA256SUMS and pull out the expected hash for our asset.
    let sums = reqwest::get(format!("{RELEASE_BASE}/SHA256SUMS"))
        .await
        .context("downloading SHA256SUMS")?
        .text()
        .await
        .context("reading SHA256SUMS")?;
    let expected = match find_sha_for_asset(&sums, &asset) {
        Some(sha) => sha,
        None => anyhow::bail!("no SHA256SUMS entry for {asset}"),
    };

    // Verify the checksum before touching the running binary.
    let actual = hex::encode(Sha256::digest(&bytes));
    if actual != expected {
        anyhow::bail!("checksum mismatch for {asset}");
    }
    tracing::info!(asset = %asset, sha = %expected, "checksum verified");

    // Swap the binary in place.
    let exe = std::env::current_exe().context("resolving rusno binary path")?;
    replace_binary(&exe, &bytes)?;

    // Best-effort systemd restart.
    if try_restart_systemd() {
        println!("service restarted");
    } else {
        println!("binary updated — restart rusno to apply");
    }

    println!("rusno updated to v{latest}.");
    Ok(())
}

/// Parse a version string (optionally with a leading 'v') into a `semver::Version`.
fn parse_version(s: &str) -> Option<Version> {
    let s = s.trim().trim_start_matches('v');
    Version::parse(s).ok()
}

/// Find the SHA256 hex for `asset` in a SHA256SUMS body.
///
/// Each line is `<sha>  <asset>` (two spaces, per `sha256sum` output). We split
/// on whitespace and match the second field against `asset`.
fn find_sha_for_asset(sums: &str, asset: &str) -> Option<String> {
    for line in sums.lines() {
        let mut parts = line.split_whitespace();
        let sha = parts.next()?;
        let name = parts.next()?;
        if name == asset {
            return Some(sha.to_string());
        }
    }
    None
}

/// Write `bytes` to a sibling temp file with mode 0755, then atomically rename
/// it over `dest`. On Linux this succeeds even when `dest` is the currently
/// running binary.
///
/// A permission error on either the temp file creation or the rename surfaces
/// as a friendly "try: sudo rusno update" hint rather than a raw errno.
fn replace_binary(dest: &Path, bytes: &[u8]) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    // Sibling temp file: current_exe + ".new".
    let tmp = dest.with_extension("new");

    let result: Result<()> = (|| {
        let mut f = std::fs::File::create(&tmp)
            .with_context(|| format!("creating temp file {}", tmp.display()))?;
        f.write_all(bytes)
            .with_context(|| format!("writing {}", tmp.display()))?;
        // Mark the temp file executable before the rename so there is no
        // window where the in-place binary is non-executable.
        f.set_permissions(std::fs::Permissions::from_mode(0o755))
            .with_context(|| format!("chmod {}", tmp.display()))?;
        // Drop the handle before the rename so the fd is released on all
        // platforms (rename over an open file is fine on Linux but not
        // necessarily elsewhere).
        drop(f);
        std::fs::rename(&tmp, dest)
            .with_context(|| format!("renaming {} -> {}", tmp.display(), dest.display()))?;
        Ok(())
    })();

    // Don't leave a half-written temp file littering the install dir.
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }

    result.map_err(|e| {
        // Detect "destination not writable" (e.g. /usr/local/bin owned by
        // root) anywhere in the error chain and surface a clear hint.
        let perm_denied = e.chain().any(|c| {
            c.downcast_ref::<std::io::Error>()
                .map(|io| io.kind() == std::io::ErrorKind::PermissionDenied)
                .unwrap_or(false)
        });
        if perm_denied {
            anyhow::anyhow!("cannot replace {} (try: sudo rusno update)", dest.display())
        } else {
            e
        }
    })
}

/// Best-effort `systemctl restart rusno` when systemd appears to manage rusno.
///
/// Returns `true` only when systemctl is present, `rusno.service` is
/// installed, and the restart command succeeded. All errors are swallowed: a
/// failed restart must not fail the update.
fn try_restart_systemd() -> bool {
    // `command -v systemctl` equivalent: can we run it at all?
    let has_systemctl = std::process::Command::new("systemctl")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !has_systemctl {
        return false;
    }

    // Only restart if rusno is actually managed by systemd — otherwise we'd
    // be restarting a unit that does not exist.
    let unit = Path::new("/etc/systemd/system/rusno.service");
    if !unit.exists() {
        return false;
    }

    let status = std::process::Command::new("systemctl")
        .args(["restart", "rusno"])
        .status();
    matches!(status, Ok(s) if s.success())
}
