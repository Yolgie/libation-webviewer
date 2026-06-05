use std::path::PathBuf;

use libation_webviewer::db::Library;

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("sample.db")
}

fn open() -> Library {
    Library::open_ro(fixture_path()).expect("open fixture sample.db")
}

#[test]
fn list_books_returns_all_undeleted_rows() {
    // sample.db has 64 books and every LibraryBooks row has IsDeleted = 0.
    let books = open().list_books().expect("list");
    assert_eq!(books.len(), 64);
}

#[test]
fn every_book_has_asin_and_title() {
    let books = open().list_books().expect("list");
    for b in &books {
        assert!(!b.asin.is_empty(), "missing ASIN on book {}", b.book_id);
        assert!(!b.title.is_empty(), "missing title on book {}", b.book_id);
    }
}

#[test]
fn project_hail_mary_metadata_is_complete() {
    let books = open().list_books().expect("list");
    let phm = books
        .iter()
        .find(|b| b.asin == "B08G9RZBTT")
        .expect("Project Hail Mary should be present in sample.db");
    assert_eq!(phm.title, "Project Hail Mary");
    assert_eq!(phm.length_minutes, 970);
    assert!(
        phm.authors.iter().any(|a| a == "Andy Weir"),
        "expected Andy Weir among authors, got {:?}",
        phm.authors,
    );
    assert!(!phm.narrators.is_empty(), "expected at least one narrator for PHM");
}

#[test]
fn books_are_sorted_by_title_case_insensitive() {
    let books = open().list_books().expect("list");
    for w in books.windows(2) {
        let a = w[0].title.to_lowercase();
        let b = w[1].title.to_lowercase();
        assert!(a <= b, "books not sorted by title: {:?} > {:?}", a, b);
    }
}

#[test]
fn get_book_by_asin_returns_full_detail_for_phm() {
    let detail = open()
        .get_book_by_asin("B08G9RZBTT")
        .expect("query ok")
        .expect("PHM should exist");
    assert_eq!(detail.view.title, "Project Hail Mary");
    assert_eq!(detail.view.length_minutes, 970);
    assert_eq!(detail.view.authors, vec!["Andy Weir"]);
    assert!(detail.view.narrators.iter().any(|n| n == "Ray Porter"));
    assert!(
        detail.publishers.iter().any(|p| p == "Audible Studios"),
        "expected Audible Studios in publishers, got {:?}",
        detail.publishers,
    );
    assert!(
        !detail.description.is_empty(),
        "expected non-empty description"
    );
    assert_eq!(detail.picture_large_id.as_deref(), Some("81Nzlrfud+L"));
}

#[test]
fn get_book_by_asin_returns_none_for_unknown() {
    let detail = open()
        .get_book_by_asin("NEVERHEARD")
        .expect("query ok");
    assert!(detail.is_none());
}

#[test]
fn get_book_by_asin_returns_series_when_present() {
    // "Undeath and Taxes (Dramatized Adaptation)" is book 2 in the Fred series.
    let detail = open()
        .get_book_by_asin("1648816533")
        .expect("query ok")
        .expect("book should exist");
    assert!(
        !detail.series.is_empty(),
        "expected series entries for this book"
    );
    assert!(
        detail
            .series
            .iter()
            .any(|s| s.name.contains("Fred, the Vampire Accountant")),
        "expected Fred series, got {:?}",
        detail.series,
    );
}
