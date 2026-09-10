//! Cryptography for rusno: password hashing, AES-256-GCM symmetric
//! encryption, and random value generation.
//!
//! All routines use the crates already pinned in `Cargo.toml`:
//! `argon2` (0.5), `aes-gcm` (0.10), `rand` (0.8), `base64` (0.22),
//! `subtle` (2), `hmac` (0.12), `sha2` (0.10), and `anyhow` for errors.

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use anyhow::{anyhow, Result};
use argon2::password_hash::{
    rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString,
};
use argon2::Argon2;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use base64::Engine;
use hmac::{Hmac, Mac};
use rand::RngCore;
use sha2::Sha256;
use subtle::ConstantTimeEq;

/// AES-256-GCM nonce size, in bytes (the standard 96-bit value).
const NONCE_LEN: usize = 12;
/// AES-256 key size, in bytes.
const KEY_LEN: usize = 32;

/// HMAC-SHA256 type alias, used for GitHub webhook verification.
type HmacSha256 = Hmac<Sha256>;

/// A 32-byte key wrapping AES-256-GCM.
///
/// `MasterKey` is the single symmetric secret used to encrypt secrets stored
/// in the database (deploy tokens, webhook secrets, SSH keys, etc.). It is
/// generated once during `rusno init` and persisted to
/// `~/.rusno/keys/master.key` with mode 0600.
///
/// The struct is `Send + Sync` (a `[u8; 32]` is `Sync`) so it can live behind
/// an `Arc<MasterKey>` in the shared `AppState` and be used from any async
/// task.
#[derive(Clone)]
pub struct MasterKey([u8; KEY_LEN]);

impl MasterKey {
    /// Generate a brand-new random key using the system CSPRNG.
    pub fn generate() -> MasterKey {
        let mut key = [0u8; KEY_LEN];
        // `thread_rng()` is a process-local CSPRNG and is the documented way
        // to obtain randomness in application code.
        rand::thread_rng().fill_bytes(&mut key);
        MasterKey(key)
    }

    /// Construct a key from existing bytes.
    ///
    /// Accepts a 32-byte slice when possible. Shorter slices are zero-padded
    /// and longer slices are truncated to 32 bytes so callers that load a
    /// possibly-misshaped key from disk never panic; the common path (an
    /// exact 32-byte file written by `generate`) is fast and allocation-free.
    pub fn from_bytes(b: &[u8]) -> MasterKey {
        let mut key = [0u8; KEY_LEN];
        let n = b.len().min(KEY_LEN);
        key[..n].copy_from_slice(&b[..n]);
        MasterKey(key)
    }

    /// Borrow the raw 32 key bytes.
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// Encrypt `plaintext` with AES-256-GCM using a fresh random nonce.
    ///
    /// The on-the-wire storage format is the base64 (standard alphabet, with
    /// padding) encoding of `nonce || ciphertext || tag`. The GCM
    /// authentication tag is appended to the ciphertext by `aes-gcm`, so the
    /// decrypt side splits off the first 12 bytes as the nonce and treats the
    /// remainder as `ciphertext || tag`.
    pub fn encrypt(&self, plaintext: &[u8]) -> Result<String> {
        // Fresh 96-bit nonce per message: GCM is catastrophically broken if a
        // nonce is ever reused under the same key, so never use a counter
        // here — always pull from the CSPRNG.
        let mut nonce_bytes = [0u8; NONCE_LEN];
        rand::thread_rng().fill_bytes(&mut nonce_bytes);

        let key = Key::<Aes256Gcm>::from_slice(&self.0);
        let cipher = Aes256Gcm::new(key);
        let nonce = Nonce::from_slice(&nonce_bytes);

        // `encrypt` returns `ciphertext || tag` as a single Vec<u8>.
        let ciphertext = cipher
            .encrypt(nonce, plaintext)
            .map_err(|e| anyhow!("aes-gcm encryption failed: {e}"))?;

        // Pack `nonce || ciphertext || tag` and base64-encode the whole blob.
        let mut blob = Vec::with_capacity(NONCE_LEN + ciphertext.len());
        blob.extend_from_slice(&nonce_bytes);
        blob.extend_from_slice(&ciphertext);
        Ok(STANDARD.encode(&blob))
    }

    /// Decrypt a value produced by [`MasterKey::encrypt`].
    ///
    /// Splits the base64 blob into nonce (first 12 bytes) and
    /// `ciphertext || tag` (the rest), then asks AES-GCM to both decrypt and
    /// verify the tag. A tampered or truncated blob yields an error rather
    /// than plaintext.
    pub fn decrypt(&self, ciphertext_b64: &str) -> Result<Vec<u8>> {
        let blob = STANDARD
            .decode(ciphertext_b64)
            .map_err(|e| anyhow!("invalid base64 ciphertext: {e}"))?;

        if blob.len() < NONCE_LEN {
            return Err(anyhow!("ciphertext too short for nonce"));
        }
        let (nonce_bytes, ciphertext) = blob.split_at(NONCE_LEN);

        let key = Key::<Aes256Gcm>::from_slice(&self.0);
        let cipher = Aes256Gcm::new(key);
        let nonce = Nonce::from_slice(nonce_bytes);

        cipher
            .decrypt(nonce, ciphertext)
            .map_err(|e| anyhow!("aes-gcm decryption failed: {e}"))
    }
}

// `MasterKey` is `Send + Sync` by construction (`[u8; 32]` is `Sync`), but we
// assert it at compile time so future field changes cannot silently break the
// `Arc<MasterKey>` in `AppState`.
const _: () = {
    const fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<MasterKey>();
};

// ----------------------------------------------------------------------------
// Password hashing (argon2id)
// ----------------------------------------------------------------------------

/// Hash a password with Argon2id, returning the canonical PHC string.
///
/// The returned string encodes the algorithm, the memory/time/parallelism
/// parameters, and a freshly-generated salt, so it is fully self-describing
/// and can be verified later with [`verify_password`] without storing any
/// extra parameters alongside it.
pub fn hash_password(password: &str) -> Result<String> {
    // `Argon2::default()` selects Argon2id with the recommended OWASP-ish
    // parameters (19 MiB memory, 2 iterations, 1 lane). `OsRng` is the
    // crate-provided CSPRNG and is the right salt source.
    let salt = SaltString::generate(&mut OsRng);
    let hash = Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map_err(|e| anyhow!("argon2 hashing failed: {e}"))?;
    Ok(hash.to_string())
}

/// Verify a plaintext password against a stored PHC hash string.
///
/// Returns `Ok(true)` on a match and `Ok(false)` when the password does not
/// match (or the hash is malformed in a way argon2 reports as "no match").
/// Hard parse/algorithm failures bubble up as `Err` so callers can tell
/// "wrong password" apart from "corrupt stored value".
pub fn verify_password(password: &str, hash: &str) -> Result<bool> {
    let parsed =
        PasswordHash::new(hash).map_err(|e| anyhow!("invalid stored password hash: {e}"))?;
    match Argon2::default().verify_password(password.as_bytes(), &parsed) {
        Ok(()) => Ok(true),
        Err(argon2::password_hash::Error::Password) => Ok(false),
        Err(e) => Err(anyhow!("argon2 verification failed: {e}")),
    }
}

// ----------------------------------------------------------------------------
// Random generation
// ----------------------------------------------------------------------------

/// `n_bytes` random bytes, base64url-encoded without padding.
///
/// Used for session secrets, webhook secrets, and opaque tokens where a
/// compact, URL-safe, copy-pasteable string is wanted.
pub fn random_base64url(n_bytes: usize) -> String {
    let mut buf = vec![0u8; n_bytes];
    rand::thread_rng().fill_bytes(&mut buf);
    URL_SAFE_NO_PAD.encode(&buf)
}

// ----------------------------------------------------------------------------
// HMAC verification (GitHub webhooks)
// ----------------------------------------------------------------------------

/// Verify a GitHub-style `X-Hub-Signature-256` header.
///
/// GitHub computes `HMAC-SHA256(body, webhook_secret)`, hex-encodes it, and
/// sends `sha256=<hex>` as the header value. This function recomputes that
/// value from `body` and `secret`, then compares it to `signature` in
/// constant time to avoid timing-based signature oracle attacks.
///
/// A missing or differently-prefixed `signature` (e.g. a plain hex value
/// with no `sha256=` prefix) is treated as a mismatch and returns `false`,
/// never an error.
pub fn verify_hmac_sha256(secret: &[u8], body: &[u8], signature: &str) -> bool {
    let mut mac = match <HmacSha256 as Mac>::new_from_slice(secret) {
        // HMAC accepts any key length, so this never fails.
        Ok(m) => m,
        Err(_) => return false,
    };
    mac.update(body);
    let expected_mac = mac.finalize().into_bytes();
    let expected = format!("sha256={}", hex::encode(expected_mac));
    constant_time_eq(&expected, signature)
}

// ----------------------------------------------------------------------------
// Constant-time string compare
// ----------------------------------------------------------------------------

/// Compare two strings in constant time.
///
/// Lengths that differ short-circuit to `false` (the lengths themselves are
/// not treated as secret here — they are typically fixed-size API tokens or
/// recomputed HMAC values of equal length). When the lengths match, the byte
/// comparison uses `subtle::ConstantTimeEq` to avoid leaking the position of
/// the first differing byte through a timing side channel.
pub fn constant_time_eq(a: &str, b: &str) -> bool {
    let (a_bytes, b_bytes) = (a.as_bytes(), b.as_bytes());
    if a_bytes.len() != b_bytes.len() {
        return false;
    }
    a_bytes.ct_eq(b_bytes).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_master_key() {
        let key = MasterKey::generate();
        let plaintext = b"super secret deploy token";
        let ciphertext = key.encrypt(plaintext).unwrap();
        let recovered = key.decrypt(&ciphertext).unwrap();
        assert_eq!(recovered, plaintext);
        // Different key must not decrypt.
        let other = MasterKey::generate();
        assert!(other.decrypt(&ciphertext).is_err());
    }

    #[test]
    fn round_trip_password() {
        let hash = hash_password("correct horse battery staple").unwrap();
        assert!(verify_password("correct horse battery staple", &hash).unwrap());
        assert!(!verify_password("wrong password", &hash).unwrap());
    }

    #[test]
    fn hmac_round_trip() {
        let secret = b"webhook-secret";
        let body = b"{\"ref\":\"refs/heads/main\"}";
        let mut mac = <HmacSha256 as Mac>::new_from_slice(secret).unwrap();
        mac.update(body);
        let sig = format!("sha256={}", hex::encode(mac.finalize().into_bytes()));
        assert!(verify_hmac_sha256(secret, body, &sig));
        assert!(!verify_hmac_sha256(secret, b"tampered", &sig));
        assert!(!verify_hmac_sha256(b"wrong-secret", body, &sig));
    }

    #[test]
    fn constant_time_eq_matches_and_mismatches() {
        assert!(constant_time_eq("abc", "abc"));
        assert!(!constant_time_eq("abc", "abd"));
        assert!(!constant_time_eq("abc", "abcd"));
    }

    #[test]
    fn random_outputs_have_expected_length() {
        // 32 bytes -> 43 base64url chars (no padding).
        assert_eq!(random_base64url(32).len(), 43);
    }
}
