use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use libation_webviewer::{auth::AuthBackend, routes, state::AppState};
use tower::ServiceExt;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

/// Copy sample.db to a temp file and return both the tempdir guard and
/// the writable path. Drop the guard to clean up.
fn writable_db_copy() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sample.db");
    std::fs::copy(fixture("sample.db"), &path).unwrap();
    (dir, path)
}

#[allow(clippy::too_many_arguments)]
fn build_state(
    cache_dir: &Path,
    db_path: PathBuf,
    db_path_rw: Option<PathBuf>,
    enable_admin: bool,
    admin_writes_allowed: bool,
    password: Option<String>,
    auth: Option<Arc<AuthBackend>>,
) -> AppState {
    AppState {
        db_path,
        db_path_rw,
        books_dir: fixture("."),
        cache_dir: cache_dir.to_path_buf(),
        scan: Arc::new(HashMap::new()),
        admin_writes_allowed,
        enable_admin,
        auth: auth.unwrap_or_else(|| Arc::new(AuthBackend::new(password))),
    }
}

async fn run(state: AppState, req: Request<Body>) -> axum::http::Response<Body> {
    let app = routes::router(state);
    app.oneshot(req).await.unwrap()
}

fn post(uri: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(uri)
        .body(Body::empty())
        .unwrap()
}

#[tokio::test]
async fn admin_routes_404_when_admin_disabled() {
    let tmp = tempfile::tempdir().unwrap();
    let state = build_state(
        tmp.path(),
        fixture("sample.db"),
        None,
        false, // enable_admin=false
        true,
        None,
        None,
    );
    let req = Request::builder()
        .uri("/admin/login")
        .body(Body::empty())
        .unwrap();
    let resp = run(state, req).await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn login_form_renders_when_enabled() {
    let tmp = tempfile::tempdir().unwrap();
    let state = build_state(
        tmp.path(),
        fixture("sample.db"),
        None,
        true,
        true,
        Some("p".into()),
        None,
    );
    let req = Request::builder()
        .uri("/admin/login")
        .body(Body::empty())
        .unwrap();
    let resp = run(state, req).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let html = std::str::from_utf8(&body).unwrap();
    assert!(html.contains("Admin login"));
    assert!(html.contains(r#"name="password""#));
}

#[tokio::test]
async fn login_submit_with_right_password_redirects_and_sets_cookie() {
    let tmp = tempfile::tempdir().unwrap();
    let state = build_state(
        tmp.path(),
        fixture("sample.db"),
        None,
        true,
        true,
        Some("hunter2".into()),
        None,
    );
    let req = Request::builder()
        .method("POST")
        .uri("/admin/login")
        .header("content-type", "application/x-www-form-urlencoded")
        .body(Body::from("password=hunter2"))
        .unwrap();
    let resp = run(state, req).await;
    assert_eq!(resp.status(), StatusCode::SEE_OTHER);
    let cookie = resp.headers().get("set-cookie").unwrap().to_str().unwrap();
    assert!(cookie.starts_with("lwv_session="));
    assert!(cookie.contains("HttpOnly"));
    assert!(cookie.contains("SameSite=Strict"));
}

#[tokio::test]
async fn login_submit_with_wrong_password_returns_401() {
    let tmp = tempfile::tempdir().unwrap();
    let state = build_state(
        tmp.path(),
        fixture("sample.db"),
        None,
        true,
        true,
        Some("hunter2".into()),
        None,
    );
    let req = Request::builder()
        .method("POST")
        .uri("/admin/login")
        .header("content-type", "application/x-www-form-urlencoded")
        .body(Body::from("password=wrong"))
        .unwrap();
    let resp = run(state, req).await;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn logout_clears_cookie() {
    let tmp = tempfile::tempdir().unwrap();
    let state = build_state(
        tmp.path(),
        fixture("sample.db"),
        None,
        true,
        true,
        None,
        None,
    );
    let resp = run(state, post("/admin/logout")).await;
    assert_eq!(resp.status(), StatusCode::SEE_OTHER);
    let cookie = resp.headers().get("set-cookie").unwrap().to_str().unwrap();
    assert!(cookie.contains("Max-Age=0"));
}

#[tokio::test]
async fn requeue_without_auth_returns_401_when_password_required() {
    let tmp = tempfile::tempdir().unwrap();
    let state = build_state(
        tmp.path(),
        fixture("sample.db"),
        Some(fixture("sample.db")),
        true,
        true,
        Some("p".into()),
        None,
    );
    let resp = run(state, post("/books/B08G9RZBTT/requeue")).await;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn requeue_when_writes_not_allowed_returns_503() {
    let (_dir, db) = writable_db_copy();
    let tmp = tempfile::tempdir().unwrap();
    let state = build_state(
        tmp.path(),
        db.clone(),
        Some(db),
        true,
        false, // admin_writes_allowed=false
        None,
        None,
    );
    let resp = run(state, post("/books/B08G9RZBTT/requeue")).await;
    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn requeue_when_rw_path_unset_returns_503() {
    let tmp = tempfile::tempdir().unwrap();
    let state = build_state(
        tmp.path(),
        fixture("sample.db"),
        None, // db_path_rw=None
        true,
        true,
        None,
        None,
    );
    let resp = run(state, post("/books/B08G9RZBTT/requeue")).await;
    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn requeue_unknown_asin_returns_404() {
    let (_dir, db) = writable_db_copy();
    let tmp = tempfile::tempdir().unwrap();
    let state = build_state(tmp.path(), db.clone(), Some(db), true, true, None, None);
    let resp = run(state, post("/books/NEVERHEARD/requeue")).await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn requeue_anonymous_mode_writes_book_status_to_zero() {
    let (_dir, db) = writable_db_copy();
    let tmp = tempfile::tempdir().unwrap();
    let state = build_state(
        tmp.path(),
        db.clone(),
        Some(db.clone()),
        true,
        true,
        None,
        None,
    );
    let resp = run(state, post("/books/B08G9RZBTT/requeue")).await;
    assert_eq!(resp.status(), StatusCode::OK);

    let conn = rusqlite::Connection::open(&db).unwrap();
    let bs: i64 = conn
        .query_row(
            "SELECT BookStatus FROM UserDefinedItem u \
             JOIN Books b ON u.BookId = b.BookId \
             WHERE b.AudibleProductId = ?1",
            rusqlite::params!["B08G9RZBTT"],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(bs, 0, "BookStatus should be 0 after requeue");
}

#[tokio::test]
async fn requeue_with_valid_cookie_writes_through() {
    let (_dir, db) = writable_db_copy();
    let tmp = tempfile::tempdir().unwrap();
    let auth = Arc::new(AuthBackend::new(Some("p".into())));
    let token = auth.issue_token();
    let state = build_state(
        tmp.path(),
        db.clone(),
        Some(db.clone()),
        true,
        true,
        None,
        Some(auth),
    );
    let req = Request::builder()
        .method("POST")
        .uri("/books/B08G9RZBTT/requeue")
        .header("cookie", format!("lwv_session={}", token))
        .body(Body::empty())
        .unwrap();
    let resp = run(state, req).await;
    assert_eq!(resp.status(), StatusCode::OK);

    let conn = rusqlite::Connection::open(&db).unwrap();
    let bs: i64 = conn
        .query_row(
            "SELECT BookStatus FROM UserDefinedItem u \
             JOIN Books b ON u.BookId = b.BookId \
             WHERE b.AudibleProductId = ?1",
            rusqlite::params!["B08G9RZBTT"],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(bs, 0);
}

#[tokio::test]
async fn library_shows_login_link_when_password_required_and_not_logged_in() {
    let tmp = tempfile::tempdir().unwrap();
    let state = build_state(
        tmp.path(),
        fixture("sample.db"),
        None,
        true,
        true,
        Some("p".into()),
        None,
    );
    let req = Request::builder().uri("/").body(Body::empty()).unwrap();
    let resp = run(state, req).await;
    let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let html = std::str::from_utf8(&body).unwrap();
    assert!(
        html.contains(r#"href="/admin/login""#),
        "library page is missing the admin login link"
    );
}

#[tokio::test]
async fn library_shows_logout_when_logged_in() {
    let tmp = tempfile::tempdir().unwrap();
    let auth = Arc::new(AuthBackend::new(Some("p".into())));
    let token = auth.issue_token();
    let state = build_state(
        tmp.path(),
        fixture("sample.db"),
        None,
        true,
        true,
        None,
        Some(auth),
    );
    let req = Request::builder()
        .uri("/")
        .header("cookie", format!("lwv_session={}", token))
        .body(Body::empty())
        .unwrap();
    let resp = run(state, req).await;
    let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let html = std::str::from_utf8(&body).unwrap();
    assert!(html.contains("Admin logout"));
}

#[tokio::test]
async fn library_shows_schema_warning_when_writes_disabled() {
    let tmp = tempfile::tempdir().unwrap();
    let state = build_state(
        tmp.path(),
        fixture("sample.db"),
        None,
        true,
        false, // admin_writes_allowed=false
        None,
        None,
    );
    let req = Request::builder().uri("/").body(Body::empty()).unwrap();
    let resp = run(state, req).await;
    let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let html = std::str::from_utf8(&body).unwrap();
    assert!(html.contains("schema-warn"));
    assert!(html.contains("ALLOW_UNKNOWN_SCHEMA"));
}

#[tokio::test]
async fn library_hides_admin_nav_when_admin_disabled() {
    let tmp = tempfile::tempdir().unwrap();
    let state = build_state(
        tmp.path(),
        fixture("sample.db"),
        None,
        false, // admin off
        true,
        Some("p".into()),
        None,
    );
    let req = Request::builder().uri("/").body(Body::empty()).unwrap();
    let resp = run(state, req).await;
    let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let html = std::str::from_utf8(&body).unwrap();
    assert!(!html.contains(r#"href="/admin/login""#));
    assert!(!html.contains("Admin logout"));
}

#[tokio::test]
async fn detail_shows_requeue_button_when_admin_can_write() {
    let tmp = tempfile::tempdir().unwrap();
    let state = build_state(
        tmp.path(),
        fixture("sample.db"),
        Some(fixture("sample.db")),
        true,
        true,
        None, // anonymous mode
        None,
    );
    let req = Request::builder()
        .uri("/books/B08G9RZBTT")
        .body(Body::empty())
        .unwrap();
    let resp = run(state, req).await;
    let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let html = std::str::from_utf8(&body).unwrap();
    assert!(html.contains("Re-queue for download"));
    assert!(html.contains(r#"hx-post="/books/B08G9RZBTT/requeue""#));
}

#[tokio::test]
async fn detail_hides_requeue_button_when_admin_disabled() {
    let tmp = tempfile::tempdir().unwrap();
    let state = build_state(
        tmp.path(),
        fixture("sample.db"),
        None,
        false,
        true,
        None,
        None,
    );
    let req = Request::builder()
        .uri("/books/B08G9RZBTT")
        .body(Body::empty())
        .unwrap();
    let resp = run(state, req).await;
    let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let html = std::str::from_utf8(&body).unwrap();
    assert!(!html.contains("Re-queue for download"));
}

#[tokio::test]
async fn detail_hides_requeue_button_when_not_logged_in() {
    let tmp = tempfile::tempdir().unwrap();
    let state = build_state(
        tmp.path(),
        fixture("sample.db"),
        Some(fixture("sample.db")),
        true,
        true,
        Some("p".into()), // password required, no cookie supplied
        None,
    );
    let req = Request::builder()
        .uri("/books/B08G9RZBTT")
        .body(Body::empty())
        .unwrap();
    let resp = run(state, req).await;
    let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let html = std::str::from_utf8(&body).unwrap();
    assert!(!html.contains("Re-queue for download"));
}

#[tokio::test]
async fn requeue_only_touches_book_status_column() {
    let (_dir, db) = writable_db_copy();
    let tmp = tempfile::tempdir().unwrap();

    // Snapshot the row before the write.
    let conn = rusqlite::Connection::open(&db).unwrap();
    #[allow(clippy::type_complexity)]
    let (book_id, _bs_before, finished_before, tags_before, last_dl_before): (
        i64,
        i64,
        i64,
        String,
        Option<String>,
    ) = conn
        .query_row(
            "SELECT u.BookId, u.BookStatus, u.IsFinished, u.Tags, u.LastDownloaded \
             FROM UserDefinedItem u JOIN Books b ON u.BookId = b.BookId \
             WHERE b.AudibleProductId = ?1",
            rusqlite::params!["B08G9RZBTT"],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .unwrap();
    drop(conn);

    let state = build_state(
        tmp.path(),
        db.clone(),
        Some(db.clone()),
        true,
        true,
        None,
        None,
    );
    let resp = run(state, post("/books/B08G9RZBTT/requeue")).await;
    assert_eq!(resp.status(), StatusCode::OK);

    let conn = rusqlite::Connection::open(&db).unwrap();
    let (bs_after, finished_after, tags_after, last_dl_after): (i64, i64, String, Option<String>) = conn
        .query_row(
            "SELECT BookStatus, IsFinished, Tags, LastDownloaded FROM UserDefinedItem WHERE BookId = ?1",
            rusqlite::params![book_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .unwrap();
    assert_eq!(bs_after, 0);
    assert_eq!(
        finished_after, finished_before,
        "IsFinished should not have changed"
    );
    assert_eq!(tags_after, tags_before, "Tags should not have changed");
    assert_eq!(
        last_dl_after, last_dl_before,
        "LastDownloaded should not have changed"
    );
}
