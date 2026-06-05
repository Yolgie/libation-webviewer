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
    assert!(
        !phm.narrators.is_empty(),
        "expected at least one narrator for PHM"
    );
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
