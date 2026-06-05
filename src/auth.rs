//! Admin auth: optional password gate plus in-memory session tokens.
//!
//! Deliberately simple (per PLAN.md):
//!   * `ADMIN_PASSWORD` env var holds the password in plain text.
//!   * Login compares it with `subtle::ConstantTimeEq`.
//!   * Successful login mints a random 256-bit token, stored in an
//!     in-memory `HashMap<token, expiry>`; the cookie is just a lookup
//!     key. No HMAC, no `SESSION_SECRET`, no persistence — restarting
//!     the process clears every session.
//!   * If `ADMIN_PASSWORD` is unset, the gate is effectively open
//!     (anonymous admin mode); main.rs already logged a WARN in that
//!     case.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

use axum::http::HeaderMap;
use subtle::ConstantTimeEq;
use uuid::Uuid;

const SESSION_TTL: Duration = Duration::from_secs(24 * 3600);
pub const COOKIE_NAME: &str = "lwv_session";

#[derive(Debug)]
pub struct AuthBackend {
    /// `None` = no password required (anonymous admin mode).
    password: Option<String>,
    sessions: Mutex<HashMap<String, SystemTime>>,
}

impl AuthBackend {
    pub fn new(password: Option<String>) -> Self {
        Self {
            password: password.filter(|s| !s.is_empty()),
            sessions: Mutex::new(HashMap::new()),
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

    /// Issue a fresh 256-bit session token (two uuid::simple-formatted
    /// chunks). Inserted into the in-memory map with a 24h expiry.
    pub fn issue_token(&self) -> String {
        let token = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
        let expiry = SystemTime::now() + SESSION_TTL;
        self.sessions
            .lock()
            .expect("session map poisoned")
            .insert(token.clone(), expiry);
        token
    }

    /// Verify a session token: returns `true` iff present in the map and
    /// not expired. Expired tokens are evicted on the way through.
    pub fn verify_token(&self, token: &str) -> bool {
        let mut sessions = self.sessions.lock().expect("session map poisoned");
        if let Some(expiry) = sessions.get(token).copied() {
            if SystemTime::now() < expiry {
                return true;
            }
            sessions.remove(token);
        }
        false
    }

    pub fn invalidate_token(&self, token: &str) {
        self.sessions
            .lock()
            .expect("session map poisoned")
            .remove(token);
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
    fn force_expire(&self, token: &str) {
        self.sessions.lock().unwrap().insert(
            token.to_string(),
            SystemTime::now() - Duration::from_secs(1),
        );
    }
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
        "{}={}; HttpOnly; SameSite=Strict; Path=/; Max-Age={}",
        COOKIE_NAME,
        token,
        SESSION_TTL.as_secs(),
    )
}

pub fn make_clear_cookie() -> String {
    format!(
        "{}=; HttpOnly; SameSite=Strict; Path=/; Max-Age=0",
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
        // Empty ADMIN_PASSWORD env var also means anonymous mode.
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
        assert!(token.len() >= 32);
        assert!(a.verify_token(&token));
        // A random other token must fail.
        assert!(!a.verify_token("not-a-real-token"));
    }

    #[test]
    fn invalidate_token_clears_the_session() {
        let a = AuthBackend::new(Some("p".into()));
        let token = a.issue_token();
        assert!(a.verify_token(&token));
        a.invalidate_token(&token);
        assert!(!a.verify_token(&token));
    }

    #[test]
    fn expired_token_is_rejected_and_evicted() {
        let a = AuthBackend::new(Some("p".into()));
        let token = a.issue_token();
        a.force_expire(&token);
        assert!(!a.verify_token(&token));
        // Second call should still return false (already evicted).
        assert!(!a.verify_token(&token));
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
        assert!(c.contains("SameSite=Strict"));
        assert!(c.contains("Path=/"));
        assert!(c.contains("Max-Age=86400"));
    }

    #[test]
    fn clear_cookie_has_zero_max_age() {
        let c = make_clear_cookie();
        assert!(c.contains("Max-Age=0"));
    }
}
