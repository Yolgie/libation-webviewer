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

fn build_state(cache_dir: &Path) -> AppState {
    AppState {
        db_path: fixture("sample.db"),
        db_path_rw: None,
        books_dir: fixture("."),
        cache_dir: cache_dir.to_path_buf(),
        admin_writes_allowed: true,
        enable_admin: false,
        auth: Arc::new(AuthBackend::new(None)),
    }
}

async fn request(state: AppState, uri: &str) -> axum::http::Response<Body> {
    let app = routes::router(state);
    app.oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap()
}

#[tokio::test]
async fn static_serves_stylesheet() {
    let tmp = tempfile::tempdir().unwrap();
    let resp = request(build_state(tmp.path()), "/static/style.css").await;
    assert_eq!(resp.status(), StatusCode::OK);
    let ct = resp
        .headers()
        .get("content-type")
        .unwrap()
        .to_str()
        .unwrap();
    assert!(ct.starts_with("text/css"), "unexpected content-type {ct}");
    let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let css = std::str::from_utf8(&body).unwrap();
    assert!(
        css.contains("font-family"),
        "css body did not look like our stylesheet"
    );
}

#[tokio::test]
async fn static_serves_htmx() {
    let tmp = tempfile::tempdir().unwrap();
    let resp = request(build_state(tmp.path()), "/static/htmx.min.js").await;
    assert_eq!(resp.status(), StatusCode::OK);
    let ct = resp
        .headers()
        .get("content-type")
        .unwrap()
        .to_str()
        .unwrap();
    assert!(
        ct.contains("javascript"),
        "unexpected content-type {ct} for htmx.min.js"
    );
    let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let js = std::str::from_utf8(&body).unwrap();
    assert!(
        js.contains("htmx"),
        "htmx.min.js body did not contain the 'htmx' marker"
    );
}

#[tokio::test]
async fn static_serves_placeholder_svg() {
    let tmp = tempfile::tempdir().unwrap();
    let resp = request(build_state(tmp.path()), "/static/placeholder.svg").await;
    assert_eq!(resp.status(), StatusCode::OK);
    let ct = resp
        .headers()
        .get("content-type")
        .unwrap()
        .to_str()
        .unwrap();
    assert!(ct.starts_with("image/svg"), "unexpected content-type {ct}");
}

#[tokio::test]
async fn static_unknown_returns_404() {
    let tmp = tempfile::tempdir().unwrap();
    let resp = request(build_state(tmp.path()), "/static/does-not-exist.css").await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn static_sets_cache_control() {
    let tmp = tempfile::tempdir().unwrap();
    let resp = request(build_state(tmp.path()), "/static/style.css").await;
    let cc = resp
        .headers()
        .get("cache-control")
        .unwrap()
        .to_str()
        .unwrap();
    assert!(
        cc.contains("max-age"),
        "cache-control missing max-age: {cc}"
    );
}
