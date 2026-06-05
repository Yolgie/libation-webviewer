use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use libation_webviewer::auth::AuthBackend;
use libation_webviewer::fs::BookFiles;
use libation_webviewer::routes;
use libation_webviewer::state::AppState;
use tower::ServiceExt;

const TEST_ASIN: &str = "TESTBOOK01";

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

fn build_state(cache_dir: &Path) -> AppState {
    let m4b = fixture("tiny.m4b");
    let mut scan = HashMap::new();
    scan.insert(
        TEST_ASIN.to_string(),
        BookFiles {
            folder: m4b.parent().unwrap().to_path_buf(),
            audio_files: vec![m4b],
            metadata_json: None,
        },
    );
    AppState {
        db_path: fixture("sample.db"),
        db_path_rw: None,
        books_dir: fixture("."),
        cache_dir: cache_dir.to_path_buf(),
        scan: Arc::new(scan),
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
async fn healthz_returns_ok() {
    let tmp = tempfile::tempdir().unwrap();
    let resp = request(build_state(tmp.path()), "/healthz").await;
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn library_list_renders_with_thumbnail_links() {
    let tmp = tempfile::tempdir().unwrap();
    let resp = request(build_state(tmp.path()), "/").await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let html = std::str::from_utf8(&body).unwrap();
    assert!(html.contains("<h1>Library"));
    assert!(html.contains(r#"src="/books/B08G9RZBTT/thumb""#));
    assert!(html.contains(r#"href="/books/B08G9RZBTT""#));
}

#[tokio::test]
async fn detail_returns_200_with_book_metadata() {
    let tmp = tempfile::tempdir().unwrap();
    let resp = request(build_state(tmp.path()), "/books/B08G9RZBTT").await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let html = std::str::from_utf8(&body).unwrap();
    assert!(html.contains("Project Hail Mary"));
    assert!(html.contains("Andy Weir"));
    assert!(html.contains("Ray Porter"));
    // Cover img and back link should both be present.
    assert!(html.contains(r#"src="/books/B08G9RZBTT/cover""#));
    assert!(html.contains(r#"href="/""#));
}

#[tokio::test]
async fn detail_returns_404_for_unknown_asin() {
    let tmp = tempfile::tempdir().unwrap();
    let resp = request(build_state(tmp.path()), "/books/NEVERHEARD").await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn files_fragment_lists_audio_files() {
    let tmp = tempfile::tempdir().unwrap();
    let resp = request(
        build_state(tmp.path()),
        &format!("/books/{}/files", TEST_ASIN),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let html = std::str::from_utf8(&body).unwrap();
    assert!(html.contains(&format!("/books/{}/download/0", TEST_ASIN)));
    assert!(html.contains("tiny.m4b"));
}

#[tokio::test]
async fn download_streams_audio_with_attachment_disposition() {
    let tmp = tempfile::tempdir().unwrap();
    let resp = request(
        build_state(tmp.path()),
        &format!("/books/{}/download/0", TEST_ASIN),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let headers = resp.headers().clone();
    assert_eq!(headers.get("content-type").unwrap(), "audio/mp4");
    let cd = headers
        .get("content-disposition")
        .unwrap()
        .to_str()
        .unwrap();
    assert!(
        cd.contains("attachment"),
        "expected attachment disposition: {}",
        cd
    );
    assert!(cd.contains("tiny.m4b"));

    let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let on_disk = std::fs::read(fixture("tiny.m4b")).unwrap();
    assert_eq!(body.as_ref(), on_disk.as_slice());
}

#[tokio::test]
async fn download_out_of_range_returns_404() {
    let tmp = tempfile::tempdir().unwrap();
    let resp = request(
        build_state(tmp.path()),
        &format!("/books/{}/download/99", TEST_ASIN),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn download_unknown_asin_returns_404() {
    let tmp = tempfile::tempdir().unwrap();
    let resp = request(build_state(tmp.path()), "/books/UNKNOWN0001/download/0").await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn library_list_does_not_double_encode_ampersands() {
    // sample.db subtitles like "Spells, Swords, &amp; Stealth ..." used to
    // render as "&amp;amp;" because the source HTML entities were escaped
    // a second time. After html::decode_entities in the db layer this
    // should reduce to a single &amp; in the rendered HTML.
    let tmp = tempfile::tempdir().unwrap();
    let resp = request(build_state(tmp.path()), "/").await;
    let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let html = std::str::from_utf8(&body).unwrap();
    assert!(
        !html.contains("&amp;amp;"),
        "library HTML still contains double-encoded ampersand"
    );
}

#[tokio::test]
async fn detail_page_strips_html_from_description() {
    // PHM's description in sample.db is raw HTML with <p>/<b>/<i> tags
    // and entity-encoded text. After html::paragraphs the template should
    // render plain prose paragraphs - no escaped tag markers visible to
    // the reader.
    let tmp = tempfile::tempdir().unwrap();
    let resp = request(build_state(tmp.path()), "/books/B08G9RZBTT").await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let html = std::str::from_utf8(&body).unwrap();
    assert!(
        !html.contains("&lt;p&gt;") && !html.contains("&lt;b&gt;") && !html.contains("&lt;i&gt;"),
        "description HTML still leaks escaped tags from the source"
    );
    // The actual text should be present:
    assert!(html.contains("THE #1"));
    assert!(html.contains("NEW YORK TIMES"));
}

#[tokio::test]
async fn library_search_filters_to_matches() {
    let tmp = tempfile::tempdir().unwrap();
    let resp = request(build_state(tmp.path()), "/?q=Project+Hail+Mary").await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let html = std::str::from_utf8(&body).unwrap();
    assert!(html.contains("Project Hail Mary"));
    // Other unrelated books should NOT be in the filtered list. Use Dune
    // (also in the sample DB but unrelated to PHM) as the negative case.
    assert!(
        !html.contains("Frank Herbert"),
        "search filter leaked an unrelated book into the result set"
    );
}

#[tokio::test]
async fn library_status_filter_not_downloaded_is_empty_on_sample() {
    // Every book in sample.db has BookStatus=1 (downloaded).
    let tmp = tempfile::tempdir().unwrap();
    let resp = request(build_state(tmp.path()), "/?status=not_downloaded").await;
    let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let html = std::str::from_utf8(&body).unwrap();
    assert!(
        html.contains("No books match"),
        "expected empty-state message"
    );
    // PHM specifically should be filtered out.
    assert!(!html.contains("Project Hail Mary"));
}

#[tokio::test]
async fn library_sort_length_descending() {
    let tmp = tempfile::tempdir().unwrap();
    let resp = request(build_state(tmp.path()), "/?sort=length").await;
    let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let html = std::str::from_utf8(&body).unwrap();
    // The longest book in sample.db is "Super Powereds: Year 4" (3637 min).
    // It should appear before shorter books.
    let phm_pos = html.find("Project Hail Mary");
    let yr4_pos = html.find("Super Powereds: Year 4");
    assert!(yr4_pos.is_some() && phm_pos.is_some());
    assert!(
        yr4_pos.unwrap() < phm_pos.unwrap(),
        "expected longer book (Year 4 = 3637 min) before shorter (PHM = 970 min)"
    );
}

#[tokio::test]
async fn partial_library_returns_rows_only_no_layout() {
    let tmp = tempfile::tempdir().unwrap();
    let resp = request(build_state(tmp.path()), "/partial/library").await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let html = std::str::from_utf8(&body).unwrap();
    // Just rows; no doctype, no <html>, no <head>.
    assert!(!html.contains("<!doctype"));
    assert!(!html.contains("<html"));
    assert!(html.contains(r#"<tbody id="library-rows">"#));
    // Real books should be present.
    assert!(html.contains("Project Hail Mary"));
}

#[tokio::test]
async fn partial_library_sets_hx_push_url_for_active_filters() {
    let tmp = tempfile::tempdir().unwrap();
    let resp = request(
        build_state(tmp.path()),
        "/partial/library?q=Project&sort=length",
    )
    .await;
    let hx = resp
        .headers()
        .get("HX-Push-Url")
        .expect("HX-Push-Url header missing")
        .to_str()
        .unwrap();
    assert!(hx.starts_with("/?"), "unexpected push URL: {hx}");
    assert!(hx.contains("q=Project"));
    assert!(hx.contains("sort=length"));
}

#[tokio::test]
async fn library_renders_filter_form() {
    let tmp = tempfile::tempdir().unwrap();
    let resp = request(build_state(tmp.path()), "/").await;
    let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let html = std::str::from_utf8(&body).unwrap();
    assert!(html.contains(r#"hx-get="/partial/library""#));
    assert!(html.contains(r#"name="q""#));
    assert!(html.contains(r#"name="sort""#));
    assert!(html.contains(r#"name="status""#));
}

#[tokio::test]
async fn library_preserves_query_in_filter_form() {
    let tmp = tempfile::tempdir().unwrap();
    let resp = request(
        build_state(tmp.path()),
        "/?q=Andy+Weir&sort=length&status=downloaded",
    )
    .await;
    let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let html = std::str::from_utf8(&body).unwrap();
    assert!(html.contains(r#"value="Andy Weir""#));
    // The selected attribute should land on the chosen options.
    assert!(
        html.contains(r#"<option value="length"     selected>Length</option>"#)
            || html.contains(r#"<option value="length" selected>Length</option>"#)
    );
}

#[tokio::test]
async fn pages_include_htmx_and_stylesheet() {
    let tmp = tempfile::tempdir().unwrap();
    let resp = request(build_state(tmp.path()), "/").await;
    let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let html = std::str::from_utf8(&body).unwrap();
    assert!(html.contains(r#"href="/static/style.css""#));
    assert!(html.contains(r#"src="/static/htmx.min.js""#));
}
