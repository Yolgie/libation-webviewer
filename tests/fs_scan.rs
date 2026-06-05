use std::fs;
use std::path::Path;

use libation_webviewer::fs::{parse_asin, scan_books};
use tempfile::TempDir;

fn touch(dir: &Path, name: &str) {
    fs::write(dir.join(name), b"").unwrap();
}

#[test]
fn parse_asin_extracts_alpha_token() {
    assert_eq!(
        parse_asin("Project Hail Mary [B08G9RZBTT]").as_deref(),
        Some("B08G9RZBTT")
    );
}

#[test]
fn parse_asin_extracts_digit_token() {
    // Real Audible ASINs can be 10 digits (legacy products).
    assert_eq!(
        parse_asin("Some Title [1648816533]").as_deref(),
        Some("1648816533")
    );
}

#[test]
fn parse_asin_rejects_invalid() {
    assert!(parse_asin("Some Title").is_none());
    assert!(parse_asin("Some Title [too-short]").is_none());
    assert!(parse_asin("Some Title [TOOLONG1234567]").is_none()); // > 10 chars
    assert!(parse_asin("Some Title [lowercase1]").is_none()); // lowercase
}

#[test]
fn scan_books_skips_folders_without_asin_token() {
    let tmp = TempDir::new().unwrap();
    let with_asin = tmp.path().join("Book A [B08G9RZBTT]");
    let no_asin = tmp.path().join("Some Folder");
    fs::create_dir(&with_asin).unwrap();
    fs::create_dir(&no_asin).unwrap();
    touch(&with_asin, "Book A [B08G9RZBTT].m4b");
    touch(&no_asin, "anything.m4b");

    let scan = scan_books(tmp.path());
    assert_eq!(
        scan.len(),
        1,
        "scan = {:?}",
        scan.keys().collect::<Vec<_>>()
    );
    assert!(scan.contains_key("B08G9RZBTT"));
}

#[test]
fn scan_books_finds_audio_and_metadata_companion() {
    let tmp = TempDir::new().unwrap();
    let book = tmp.path().join("Book A [B08G9RZBTT]");
    fs::create_dir(&book).unwrap();
    touch(&book, "Book A [B08G9RZBTT].m4b");
    touch(&book, "Book A [B08G9RZBTT].metadata.json");
    touch(&book, "irrelevant.txt");

    let scan = scan_books(tmp.path());
    let entry = &scan["B08G9RZBTT"];
    assert_eq!(entry.audio_files.len(), 1);
    assert!(entry.audio_files[0]
        .file_name()
        .unwrap()
        .to_str()
        .unwrap()
        .ends_with(".m4b"));
    assert!(entry.metadata_json.is_some());
}

#[test]
fn scan_books_handles_mp3_format() {
    let tmp = TempDir::new().unwrap();
    let book = tmp.path().join("MP3 Book [ABCDEFGHIJ]");
    fs::create_dir(&book).unwrap();
    touch(&book, "MP3 Book [ABCDEFGHIJ].mp3");

    let scan = scan_books(tmp.path());
    let entry = &scan["ABCDEFGHIJ"];
    assert_eq!(entry.audio_files.len(), 1);
}

#[test]
fn scan_books_sorts_split_chapters() {
    let tmp = TempDir::new().unwrap();
    let book = tmp.path().join("Split [ABCDEFGHIJ]");
    fs::create_dir(&book).unwrap();
    touch(&book, "Split [ABCDEFGHIJ] - Part 03.m4b");
    touch(&book, "Split [ABCDEFGHIJ] - Part 01.m4b");
    touch(&book, "Split [ABCDEFGHIJ] - Part 02.m4b");

    let scan = scan_books(tmp.path());
    let names: Vec<String> = scan["ABCDEFGHIJ"]
        .audio_files
        .iter()
        .map(|p| p.file_name().unwrap().to_str().unwrap().to_string())
        .collect();
    assert_eq!(
        names,
        vec![
            "Split [ABCDEFGHIJ] - Part 01.m4b",
            "Split [ABCDEFGHIJ] - Part 02.m4b",
            "Split [ABCDEFGHIJ] - Part 03.m4b",
        ]
    );
}

#[test]
fn scan_books_handles_missing_root_gracefully() {
    let scan = scan_books("/nonexistent/path/that/should/never/exist");
    assert!(scan.is_empty());
}
