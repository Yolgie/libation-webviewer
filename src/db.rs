//! rusqlite queries against Libation's SQLite DB.
//!
//! Opens the DB read-only with a busy timeout so we never wedge
//! Libation's own writer. The schema-drift guard, write path, and
//! per-book detail query land in later slices.

use std::path::Path;
use std::time::Duration;

use rusqlite::{Connection, OpenFlags, Row};

use crate::view::BookView;

const LIST_BOOKS_SQL: &str = r#"
SELECT
  b.BookId,
  b.AudibleProductId,
  b.Title,
  b.Subtitle,
  b.LengthInMinutes,
  b.Locale,
  b.Language,
  b.DatePublished,
  COALESCE(udi.BookStatus, 0),
  COALESCE(udi.IsFinished, 0),
  lb.DateAdded,
  COALESCE(lb.IsAudiblePlus, 0),
  COALESCE(lb.AbsentFromLastScan, 0),
  (SELECT GROUP_CONCAT(c.Name, '|||')
     FROM BookContributor bc
     JOIN Contributors c ON bc.ContributorId = c.ContributorId
     WHERE bc.BookId = b.BookId AND bc.Role = 1),
  (SELECT GROUP_CONCAT(c.Name, '|||')
     FROM BookContributor bc
     JOIN Contributors c ON bc.ContributorId = c.ContributorId
     WHERE bc.BookId = b.BookId AND bc.Role = 2)
FROM Books b
LEFT JOIN LibraryBooks lb ON b.BookId = lb.BookId
LEFT JOIN UserDefinedItem udi ON b.BookId = udi.BookId
WHERE COALESCE(lb.IsDeleted, 0) = 0
ORDER BY b.Title COLLATE NOCASE
"#;

pub struct Library {
    conn: Connection,
}

impl Library {
    /// Open Libation's SQLite DB read-only with a 5s busy timeout.
    pub fn open_ro(path: impl AsRef<Path>) -> rusqlite::Result<Self> {
        let path_str = path.as_ref().to_string_lossy();
        let uri = format!("file:{}?mode=ro", path_str);
        let conn = Connection::open_with_flags(
            &uri,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
        )?;
        conn.busy_timeout(Duration::from_secs(5))?;
        Ok(Self { conn })
    }

    /// List every undeleted book with its authors and narrators.
    pub fn list_books(&self) -> rusqlite::Result<Vec<BookView>> {
        let mut stmt = self.conn.prepare(LIST_BOOKS_SQL)?;
        let books = stmt
            .query_map([], map_book_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(books)
    }
}

fn map_book_row(row: &Row<'_>) -> rusqlite::Result<BookView> {
    // Subtitle is TEXT NOT NULL in the schema; empty string = no subtitle.
    let subtitle: String = row.get(3)?;
    let authors_concat: Option<String> = row.get(13)?;
    let narrators_concat: Option<String> = row.get(14)?;
    Ok(BookView {
        book_id: row.get(0)?,
        asin: row.get(1)?,
        title: row.get(2)?,
        subtitle: if subtitle.is_empty() { None } else { Some(subtitle) },
        length_minutes: row.get(4)?,
        locale: row.get(5)?,
        language: row.get(6)?,
        date_published: row.get(7)?,
        book_status: row.get(8)?,
        is_finished: row.get::<_, i64>(9)? != 0,
        date_added: row.get(10)?,
        is_audible_plus: row.get::<_, i64>(11)? != 0,
        absent_from_last_scan: row.get::<_, i64>(12)? != 0,
        authors: split_or_empty(authors_concat),
        narrators: split_or_empty(narrators_concat),
    })
}

fn split_or_empty(opt: Option<String>) -> Vec<String> {
    match opt {
        None => Vec::new(),
        Some(s) if s.is_empty() => Vec::new(),
        Some(s) => s.split("|||").map(String::from).collect(),
    }
}
