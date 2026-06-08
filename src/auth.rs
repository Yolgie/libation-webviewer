//! Admin auth: optional password gate plus HMAC-signed cookie sessions.
//!
//! Deliberately simple (per PLAN.md v1.1):
//!   * `ADMIN_PASSWORD` env var holds the password in plain text.
//!   * Login compares it with `subtle::ConstantTimeEq`.
//!   * Successful login mints a signed cookie of the form
//!     `<expiry_unix>.<hex(hmac_sha256(secret, expiry_unix))>`. The
//!     server keeps no per-session state: verify parses the cookie,
//!     re-derives the HMAC under its secret, and rejects on either
//!     signature mismatch or past expiry.
//!   * `SESSION_SECRET` env (64-char hex = 32 bytes) anchors the HMAC.
//!     If unset, the secret is generated at startup and sessions drop
//!     on restart — same effective behaviour as the pre-v1.1 in-memory
//!     map.
//!   * If `ADMIN_PASSWORD` is unset, the gate is effectively open
//!     (anonymous admin mode); main.rs already logged a WARN.
//!
//! Cookies are issued with `Secure; HttpOnly; SameSite=Strict; Path=/`.
//! The `Secure` flag assumes the deployment topology documented in
//! README (Caddy + Let's Encrypt terminating TLS in front of the
//! viewer container).

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::http::HeaderMap;
use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;
use subtle::ConstantTimeEq;
use uuid::Uuid;

const SESSION_TTL: Duration = Duration::from_secs(24 * 3600);
pub const COOKIE_NAME: &str = "lwv_session";

type HmacSha256 = Hmac<Sha256>;

#[derive(Debug)]
pub struct AuthBackend {
    /// `None` = no password required (anonymous admin mode).
    password: Option<String>,
    secret: [u8; 32],
}

impl AuthBackend {
    /// Construct with a freshly-generated 256-bit secret. Tokens minted
    /// by this instance won't validate after a restart unless the
    /// operator persists the secret out-of-band via `SESSION_SECRET`.
    pub fn new(password: Option<String>) -> Self {
        Self::with_secret(password, random_secret())
    }

    /// Construct with an explicit 256-bit secret (e.g. loaded from
    /// `SESSION_SECRET`). Tokens minted under the same secret survive
    /// process restarts.
    pub fn with_secret(password: Option<String>, secret: [u8; 32]) -> Self {
        Self {
            password: password.filter(|s| !s.is_empty()),
            secret,
        }
    }

    /// `true` if a password is set and login is therefore required.
    pub fn requires_password(&self) -> bool {
        self.password.is_some()
    }

    /// Constant-time check of the submitted password against the env-var.
    /// Returns `true` when no password is required (anonymous mode).
    pub fn verify_password(&self, submitted: &str) -> bool {
        let Some(expected) = &self.password else {
            return true;
        };
        submitted.as_bytes().ct_eq(expected.as_bytes()).into()
    }

    /// Issue a fresh signed cookie value with a 24h expiry.
    pub fn issue_token(&self) -> String {
        self.mint(SystemTime::now() + SESSION_TTL)
    }

    fn mint(&self, expiry: SystemTime) -> String {
        let expiry_unix = expiry
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let mac = self.sign(expiry_unix);
        format!("{}.{}", expiry_unix, hex::encode(mac))
    }

    fn sign(&self, expiry_unix: u64) -> [u8; 32] {
        // HMAC accepts any key length; only fails on allocation issues
        // that can't happen for a 32-byte key.
        let mut mac =
            HmacSha256::new_from_slice(&self.secret).expect("HMAC-SHA256 accepts any key length");
        mac.update(expiry_unix.to_string().as_bytes());
        mac.finalize().into_bytes().into()
    }

    /// Verify a session cookie: the HMAC must match under our secret
    /// AND the encoded expiry must be in the future. Stateless — no
    /// server-side lookup. Any malformed token returns `false` without
    /// panicking.
    pub fn verify_token(&self, token: &str) -> bool {
        let Some((expiry_str, sig_hex)) = token.split_once('.') else {
            return false;
        };
        let Ok(expiry_unix) = expiry_str.parse::<u64>() else {
            return false;
        };
        let Ok(sig) = hex::decode(sig_hex) else {
            return false;
        };
        let expected = self.sign(expiry_unix);
        if !bool::from(sig.as_slice().ct_eq(&expected)) {
            return false;
        }
        let now_unix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        expiry_unix > now_unix
    }

    /// Convenience for handlers: read the cookie out of the headers,
    /// verify the token. If no password is required, returns `true`
    /// unconditionally.
    pub fn is_authenticated(&self, headers: &HeaderMap) -> bool {
        if !self.requires_password() {
            return true;
        }
        match extract_session_token(headers) {
            Some(token) => self.verify_token(&token),
            None => false,
        }
    }

    #[cfg(test)]
    fn mint_with_expiry(&self, expiry: SystemTime) -> String {
        self.mint(expiry)
    }
}

/// Generate a fresh 256-bit secret. UUIDv4 derives its random bytes
/// from the platform CSPRNG (via `getrandom`), so concatenating two of
/// them yields >240 bits of entropy — plenty for an HMAC key, and no
/// extra dependency beyond what's already pulled in for token IDs.
fn random_secret() -> [u8; 32] {
    let mut out = [0u8; 32];
    out[..16].copy_from_slice(Uuid::new_v4().as_bytes());
    out[16..].copy_from_slice(Uuid::new_v4().as_bytes());
    out
}

/// Decode a 32-byte secret from a hex-encoded `SESSION_SECRET` env
/// var. Returns `None` on bad hex or wrong length so the caller can
/// log a clear error and fall back to a generated secret.
pub fn decode_secret(hex_str: &str) -> Option<[u8; 32]> {
    let bytes = hex::decode(hex_str.trim()).ok()?;
    bytes.try_into().ok()
}

/// Read `lwv_session=<token>` from the `Cookie` header, if present.
pub fn extract_session_token(headers: &HeaderMap) -> Option<String> {
    let cookie_header = headers.get("cookie")?.to_str().ok()?;
    let prefix = format!("{}=", COOKIE_NAME);
    for pair in cookie_header.split(';') {
        let pair = pair.trim();
        if let Some(value) = pair.strip_prefix(&prefix) {
            return Some(value.to_string());
        }
    }
    None
}

pub fn make_set_cookie(token: &str) -> String {
    format!(
        "{}={}; HttpOnly; Secure; SameSite=Strict; Path=/; Max-Age={}",
        COOKIE_NAME,
        token,
        SESSION_TTL.as_secs(),
    )
}

pub fn make_clear_cookie() -> String {
    format!(
        "{}=; HttpOnly; Secure; SameSite=Strict; Path=/; Max-Age=0",
        COOKIE_NAME
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn with_cookie(value: &str) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert("cookie", HeaderValue::from_str(value).unwrap());
        h
    }

    #[test]
    fn no_password_means_anonymous_mode() {
        let a = AuthBackend::new(None);
        assert!(!a.requires_password());
        assert!(a.verify_password("anything"));
        assert!(a.is_authenticated(&HeaderMap::new()));
    }

    #[test]
    fn empty_password_treated_as_no_password() {
        let a = AuthBackend::new(Some("".into()));
        assert!(!a.requires_password());
        assert!(a.verify_password("whatever"));
    }

    #[test]
    fn password_must_match_constant_time() {
        let a = AuthBackend::new(Some("hunter2".into()));
        assert!(a.requires_password());
        assert!(a.verify_password("hunter2"));
        assert!(!a.verify_password("hunter3"));
        assert!(!a.verify_password("hunter21"));
        assert!(!a.verify_password(""));
    }

    #[test]
    fn issue_then_verify_token() {
        let a = AuthBackend::new(Some("p".into()));
        let token = a.issue_token();
        // <expiry>.<hex sha256> = at least "<unix>." + 64 hex chars.
        assert!(token.contains('.'));
        assert!(a.verify_token(&token));
        assert!(!a.verify_token("not-a-real-token"));
    }

    #[test]
    fn expired_token_is_rejected() {
        let a = AuthBackend::new(Some("p".into()));
        let token = a.mint_with_expiry(SystemTime::now() - Duration::from_secs(1));
        assert!(!a.verify_token(&token));
    }

    #[test]
    fn tampered_hmac_is_rejected() {
        let a = AuthBackend::new(Some("p".into()));
        let token = a.issue_token();
        let mut bytes = token.into_bytes();
        // Flip the last hex nibble — still well-formed, signature invalid.
        let last = bytes.last_mut().unwrap();
        *last = if *last == b'0' { b'1' } else { b'0' };
        let tampered = String::from_utf8(bytes).unwrap();
        assert!(!a.verify_token(&tampered));
    }

    #[test]
    fn token_signed_by_a_different_secret_is_rejected() {
        let a = AuthBackend::with_secret(Some("p".into()), [1u8; 32]);
        let b = AuthBackend::with_secret(Some("p".into()), [2u8; 32]);
        let token = a.issue_token();
        assert!(!b.verify_token(&token));
    }

    #[test]
    fn malformed_token_shape_does_not_panic() {
        let a = AuthBackend::new(Some("p".into()));
        assert!(!a.verify_token(""));
        assert!(!a.verify_token("no-dot"));
        assert!(!a.verify_token(".only-dot"));
        assert!(!a.verify_token("only-dot."));
        assert!(!a.verify_token("not-a-number.deadbeef"));
        assert!(!a.verify_token("12345.not-hex!"));
        assert!(!a.verify_token("12345.aa.bb"));
    }

    #[test]
    fn token_survives_when_secret_is_pinned() {
        // The key UX win for SESSION_SECRET: tokens minted by one
        // backend instance verify against a fresh instance with the
        // same secret. (Whether the *process* survives is irrelevant.)
        let secret = [7u8; 32];
        let a = AuthBackend::with_secret(Some("p".into()), secret);
        let token = a.issue_token();
        let b = AuthBackend::with_secret(Some("p".into()), secret);
        assert!(b.verify_token(&token));
    }

    #[test]
    fn is_authenticated_reads_the_cookie() {
        let a = AuthBackend::new(Some("p".into()));
        let token = a.issue_token();
        let headers = with_cookie(&format!("lwv_session={}", token));
        assert!(a.is_authenticated(&headers));
    }

    #[test]
    fn is_authenticated_fails_without_cookie() {
        let a = AuthBackend::new(Some("p".into()));
        let _ = a.issue_token();
        assert!(!a.is_authenticated(&HeaderMap::new()));
    }

    #[test]
    fn extract_session_token_picks_the_right_pair() {
        let h = with_cookie("foo=bar; lwv_session=abc123; baz=qux");
        assert_eq!(extract_session_token(&h).as_deref(), Some("abc123"));
    }

    #[test]
    fn extract_session_token_none_when_absent() {
        let h = with_cookie("foo=bar; baz=qux");
        assert_eq!(extract_session_token(&h), None);
    }

    #[test]
    fn set_cookie_has_security_attributes() {
        let c = make_set_cookie("abc");
        assert!(c.contains("HttpOnly"));
        assert!(c.contains("Secure"));
        assert!(c.contains("SameSite=Strict"));
        assert!(c.contains("Path=/"));
        assert!(c.contains("Max-Age=86400"));
    }

    #[test]
    fn clear_cookie_has_zero_max_age_and_secure() {
        let c = make_clear_cookie();
        assert!(c.contains("Max-Age=0"));
        assert!(c.contains("Secure"));
    }

    #[test]
    fn decode_secret_roundtrips_a_well_formed_hex_string() {
        let hex_str = "0".repeat(64);
        assert_eq!(decode_secret(&hex_str), Some([0u8; 32]));
    }

    #[test]
    fn decode_secret_rejects_wrong_length() {
        // 30 bytes (60 hex chars) — close but no cigar.
        assert!(decode_secret(&"0".repeat(60)).is_none());
        assert!(decode_secret(&"0".repeat(70)).is_none());
    }

    #[test]
    fn decode_secret_rejects_bad_hex() {
        assert!(decode_secret(&"zz".repeat(32)).is_none());
    }

    #[test]
    fn random_secrets_differ_between_calls() {
        // 256 bits of entropy means collisions are astronomically
        // unlikely; we mostly want to assert the bytes aren't a constant.
        let a = random_secret();
        let b = random_secret();
        assert_ne!(a, b);
    }
}
