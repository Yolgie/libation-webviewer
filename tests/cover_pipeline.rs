use std::path::PathBuf;

use libation_webviewer::cover::{extract, get_or_build, resize_to_webp, src_hash, CoverError, CoverFormat};
use tempfile::TempDir;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

#[test]
fn extract_m4b_returns_jpeg() {
    let (bytes, fmt) = extract(&fixture("tiny.m4b")).expect("m4b extract");
    assert!(bytes.len() > 100, "expected non-trivial cover bytes ({} bytes)", bytes.len());
    assert_eq!(&bytes[..2], &[0xff, 0xd8], "expected JPEG SOI");
    assert!(matches!(fmt, CoverFormat::Jpeg));
}

#[test]
fn extract_mp3_returns_jpeg() {
    let (bytes, fmt) = extract(&fixture("tiny.mp3")).expect("mp3 extract");
    assert!(bytes.len() > 100);
    assert_eq!(&bytes[..2], &[0xff, 0xd8]);
    assert!(matches!(fmt, CoverFormat::Jpeg));
}

#[test]
fn extract_no_cover_returns_not_found() {
    let err = extract(&fixture("no_cover.m4b")).expect_err("expected NotFound");
    assert!(matches!(err, CoverError::NotFound), "got {:?}", err);
}

#[test]
fn extract_unsupported_extension_returns_parse_error() {
    let err = extract(std::path::Path::new("/tmp/whatever.ogg"))
        .expect_err("expected parse error");
    assert!(matches!(err, CoverError::Parse(_)), "got {:?}", err);
}

#[test]
fn resize_produces_valid_webp() {
    let (jpeg, _) = extract(&fixture("tiny.m4b")).unwrap();
    let webp = resize_to_webp(&jpeg, 32).expect("resize");
    assert!(!webp.is_empty());
    // WebP RIFF header: "RIFF" .. "WEBP"
    assert_eq!(&webp[..4], b"RIFF");
    assert_eq!(&webp[8..12], b"WEBP");
}

#[tokio::test]
async fn get_or_build_thumb_writes_then_reads_cache() {
    let cache = TempDir::new().unwrap();
    let source = fixture("tiny.m4b");

    let (bytes1, mime1) = get_or_build(cache.path(), "B08G9RZBTT", &source, true)
        .await
        .expect("first build");
    assert_eq!(mime1, "image/webp");

    let hash = src_hash(&source).unwrap();
    let expected = cache
        .path()
        .join("covers")
        .join("B08G9RZBTT")
        .join(format!("thumb-{}.webp", hash));
    assert!(expected.exists(), "thumb cache file not written: {:?}", expected);

    // Second call must return identical bytes (from cache).
    let (bytes2, mime2) = get_or_build(cache.path(), "B08G9RZBTT", &source, true)
        .await
        .expect("second build");
    assert_eq!(bytes1, bytes2);
    assert_eq!(mime1, mime2);
}

#[tokio::test]
async fn get_or_build_full_preserves_jpeg_mime() {
    let cache = TempDir::new().unwrap();
    let (bytes, mime) = get_or_build(cache.path(), "ABCDEFGHIJ", &fixture("tiny.mp3"), false)
        .await
        .expect("full");
    assert_eq!(mime, "image/jpeg");
    assert_eq!(&bytes[..2], &[0xff, 0xd8]);
}

#[tokio::test]
async fn get_or_build_no_cover_surfaces_not_found() {
    let cache = TempDir::new().unwrap();
    let err = get_or_build(cache.path(), "NOCOVER001", &fixture("no_cover.m4b"), true)
        .await
        .expect_err("expected NotFound");
    assert!(matches!(err, CoverError::NotFound));
}
