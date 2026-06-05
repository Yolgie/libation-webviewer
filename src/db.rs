//! rusqlite queries against Libation's SQLite DB.
//!
//! Opens the DB read-only with a busy timeout so we never wedge
//! Libation's own writer. The schema-drift guard, write path, and
//! per-book detail query land in later slices.

use std::path::Path;
use std::time::Duration;

use rusqlite::{params, Connection, OpenFlags, Row};

use crate::view::{BookDetail, BookView, SeriesEntry};

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

const GET_BOOK_SQL: &str = r#"
SELECT
  b.BookId,
  b.AudibleProductId,
  b.Title,
  b.Subtitle,
  b.LengthInMinutes,
  b.Locale,
  b.Language,
  b.DatePublished,
  b.Description,
  b.PictureLarge,
  COALESCE(udi.BookStatus, 0),
  COALESCE(udi.IsFinished, 0),
  lb.DateAdded,
  COALESCE(lb.IsAudiblePlus, 0),
  COALESCE(lb.AbsentFromLastScan, 0)
FROM Books b
LEFT JOIN LibraryBooks lb ON b.BookId = lb.BookId
LEFT JOIN UserDefinedItem udi ON b.BookId = udi.BookId
WHERE b.AudibleProductId = ?1
  AND COALESCE(lb.IsDeleted, 0) = 0
"#;

const CONTRIBUTORS_SQL: &str = r#"
SELECT bc.Role, c.Name
FROM BookContributor bc
JOIN Contributors c ON bc.ContributorId = c.ContributorId
WHERE bc.BookId = ?1
ORDER BY bc.Role, bc."Order"
"#;

const SERIES_SQL: &str = r#"
SELECT s.Name, sb."Order"
FROM SeriesBook sb
JOIN Series s ON s.SeriesId = sb.SeriesId
WHERE sb.BookId = ?1
ORDER BY sb."Order"
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

    /// Look up the full detail record for a single ASIN.
    pub fn get_book_by_asin(&self, asin: &str) -> rusqlite::Result<Option<BookDetail>> {
        let mut stmt = self.conn.prepare(GET_BOOK_SQL)?;
        let detail = stmt.query_row(params![asin], map_detail_row).optional()?;
        let Some(mut detail) = detail else {
            return Ok(None);
        };

        // Authors + narrators + publishers in one pass over the contributors table.
        let mut authors = Vec::new();
        let mut narrators = Vec::new();
        let mut publishers = Vec::new();
        let mut contrib_stmt = self.conn.prepare(CONTRIBUTORS_SQL)?;
        let mut rows = contrib_stmt.query(params![detail.view.book_id])?;
        while let Some(row) = rows.next()? {
            let role: i64 = row.get(0)?;
            let name: String = row.get(1)?;
            match role {
                1 => authors.push(name),
                2 => narrators.push(name),
                3 => publishers.push(name),
                _ => {} // Unknown roles are tolerated, just ignored on the detail page.
            }
        }
        detail.view.authors = authors;
        detail.view.narrators = narrators;
        detail.publishers = publishers;

        // Series entries (zero or more).
        let mut series = Vec::new();
        let mut series_stmt = self.conn.prepare(SERIES_SQL)?;
        let mut rows = series_stmt.query(params![detail.view.book_id])?;
        while let Some(row) = rows.next()? {
            let name: Option<String> = row.get(0)?;
            let order: Option<String> = row.get(1)?;
            if let Some(name) = name {
                series.push(SeriesEntry { name, order });
            }
        }
        detail.series = series;

        Ok(Some(detail))
    }
}

// Bring rusqlite's OptionalExtension into scope for `.optional()`.
use rusqlite::OptionalExtension;

fn map_book_row(row: &Row<'_>) -> rusqlite::Result<BookView> {
    // Subtitle is TEXT NOT NULL in the schema; empty string = no subtitle.
    let subtitle: String = row.get(3)?;
    let authors_concat: Option<String> = row.get(13)?;
    let narrators_concat: Option<String> = row.get(14)?;
    Ok(BookView {
        book_id: row.get(0)?,
        asin: row.get(1)?,
        title: row.get(2)?,
        subtitle: if subtitle.is_empty() {
            None
        } else {
            Some(subtitle)
        },
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

fn map_detail_row(row: &Row<'_>) -> rusqlite::Result<BookDetail> {
    let subtitle: String = row.get(3)?;
    let description: String = row.get(8)?;
    let picture_large: Option<String> = row.get(9)?;
    Ok(BookDetail {
        view: BookView {
            book_id: row.get(0)?,
            asin: row.get(1)?,
            title: row.get(2)?,
            subtitle: if subtitle.is_empty() {
                None
            } else {
                Some(subtitle)
            },
            length_minutes: row.get(4)?,
            locale: row.get(5)?,
            language: row.get(6)?,
            date_published: row.get(7)?,
            book_status: row.get(10)?,
            is_finished: row.get::<_, i64>(11)? != 0,
            date_added: row.get(12)?,
            is_audible_plus: row.get::<_, i64>(13)? != 0,
            absent_from_last_scan: row.get::<_, i64>(14)? != 0,
            authors: Vec::new(), // populated by get_book_by_asin
            narrators: Vec::new(),
        },
        description,
        publishers: Vec::new(),
        series: Vec::new(),
        picture_large_id: picture_large.filter(|s| !s.is_empty()),
    })
}

fn split_or_empty(opt: Option<String>) -> Vec<String> {
    match opt {
        None => Vec::new(),
        Some(s) if s.is_empty() => Vec::new(),
        Some(s) => s.split("|||").map(String::from).collect(),
    }
}
