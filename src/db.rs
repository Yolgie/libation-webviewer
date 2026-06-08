//! rusqlite queries against Libation's SQLite DB.
//!
//! Read paths run through `LibraryPool` — an r2d2 pool of read-only
//! connections — and execute under `tokio::task::spawn_blocking` so
//! rusqlite's synchronous calls never stall an async runtime thread.
//! The single write path (the admin requeue) opens a separate RW
//! connection on demand, runs one `UPDATE`, and drops it; pooling RW
//! would serialise badly with Libation's own writer. A schema-drift
//! guard compares the latest `__EFMigrationsHistory` head against
//! `KNOWN_GOOD_MIGRATIONS` and fails admin writes closed when the
//! head is unknown — flipped on by `ALLOW_UNKNOWN_SCHEMA=1`. The list
//! query (`LIST_BOOKS_SQL`) assembles the library view in one pass;
//! per-book detail (`get_book_by_asin`) loads contributors, series,
//! and supplements around the core row.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::{params, Connection, OpenFlags, OptionalExtension, Row};

use crate::html::decode_entities;
use crate::view::{BookDetail, BookView, SeriesEntry};

/// Migration ids the viewer has been validated against. Add new heads
/// here whenever Libation ships a schema migration the viewer has been
/// re-tested against. `ALLOW_UNKNOWN_SCHEMA=1` lets an operator override
/// the resulting gate at runtime.
const KNOWN_GOOD_MIGRATIONS: &[&str] = &[
    // Latest seen as of June 2026, from the sample DB.
    "20260427201829_ReAddCategoryName2",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchemaStatus {
    Known,
    Unknown,
}

/// Unified error for the DB layer — covers pool-acquisition failures,
/// rusqlite errors, and blocking-task panics. Slice 4c (`AppError`)
/// will convert this into an HTTP response.
#[derive(thiserror::Error, Debug)]
pub enum DbError {
    #[error("connection pool error: {0}")]
    Pool(#[from] r2d2::Error),
    #[error(transparent)]
    Db(#[from] rusqlite::Error),
    #[error("db worker task failed: {0}")]
    Join(String),
}

const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

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

const SUPPLEMENTS_SQL: &str = r#"
SELECT Url FROM Supplement WHERE BookId = ?1 ORDER BY SupplementId
"#;

/// Pool of read-only SQLite connections. Clone-cheap (the pool itself
/// is wrapped in an `Arc` by r2d2). Each `ro` call grabs a pooled
/// connection and runs the closure inside `spawn_blocking` so the
/// synchronous rusqlite work never blocks the tokio runtime.
#[derive(Debug, Clone)]
pub struct LibraryPool {
    inner: Arc<PoolInner>,
}

#[derive(Debug)]
struct PoolInner {
    ro: r2d2::Pool<SqliteConnectionManager>,
    db_path: PathBuf,
}

impl LibraryPool {
    /// Build a read-only pool against Libation's SQLite DB. The DB is
    /// opened with `mode=ro` and a 5s busy timeout so we never wedge
    /// Libation's own writer.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, DbError> {
        let db_path = path.as_ref().to_path_buf();
        let path_str = db_path.to_string_lossy().into_owned();
        let uri = format!("file:{}?mode=ro", path_str);
        let manager = SqliteConnectionManager::file(uri)
            .with_flags(OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI)
            .with_init(|c| c.busy_timeout(BUSY_TIMEOUT));
        // Small pool: the viewer's read traffic is dominated by
        // filesystem scans, not DB throughput; four connections is
        // plenty for parallel HTMX swaps.
        let ro = r2d2::Pool::builder()
            .max_size(4)
            .build(manager)
            .map_err(DbError::Pool)?;
        Ok(Self {
            inner: Arc::new(PoolInner { ro, db_path }),
        })
    }

    /// The DB path the pool was opened against. Used at startup for
    /// the schema check (which is synchronous and happens before the
    /// runtime is up to speed) and surfaced for log lines.
    pub fn db_path(&self) -> &Path {
        &self.inner.db_path
    }

    /// Run a synchronous closure against a pooled read-only connection
    /// inside `spawn_blocking`. The closure returns a `rusqlite::Result`;
    /// any error (pool, db, join) is collapsed into `DbError`.
    pub async fn ro<R, F>(&self, f: F) -> Result<R, DbError>
    where
        F: FnOnce(&Connection) -> rusqlite::Result<R> + Send + 'static,
        R: Send + 'static,
    {
        let pool = self.inner.clone();
        tokio::task::spawn_blocking(move || -> Result<R, DbError> {
            let conn = pool.ro.get()?;
            Ok(f(&conn)?)
        })
        .await
        .map_err(|e| DbError::Join(e.to_string()))?
    }
}

/// Thin wrapper around an owned `Connection`. Kept for code paths that
/// want a one-shot handle (startup schema check, admin RW path,
/// unit/integration tests). All query methods delegate to the free
/// functions below so the pool and the wrapper share a single SQL
/// surface.
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
        conn.busy_timeout(BUSY_TIMEOUT)?;
        Ok(Self { conn })
    }

    /// Open Libation's SQLite DB read-write. Used only inside the admin
    /// requeue path. Held briefly: each `requeue_book` call opens,
    /// commits, and drops the handle.
    pub fn open_rw(path: impl AsRef<Path>) -> rusqlite::Result<Self> {
        let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
        conn.busy_timeout(BUSY_TIMEOUT)?;
        Ok(Self { conn })
    }

    pub fn ping(&self) -> rusqlite::Result<()> {
        ping(&self.conn)
    }

    pub fn book_id_for_asin(&self, asin: &str) -> rusqlite::Result<Option<i64>> {
        book_id_for_asin(&self.conn, asin)
    }

    pub fn requeue_book(&mut self, book_id: i64) -> rusqlite::Result<usize> {
        requeue_book(&mut self.conn, book_id)
    }

    pub fn latest_migration(&self) -> Option<String> {
        latest_migration(&self.conn)
    }

    pub fn check_schema(&self) -> (SchemaStatus, Option<String>) {
        check_schema(&self.conn)
    }

    pub fn list_books(&self) -> rusqlite::Result<Vec<BookView>> {
        list_books(&self.conn)
    }

    pub fn get_book_by_asin(&self, asin: &str) -> rusqlite::Result<Option<BookDetail>> {
        get_book_by_asin(&self.conn, asin)
    }
}

// ---- Free query functions (the single SQL surface). --------------------

/// Issue the cheapest possible round-trip to confirm the DB is
/// readable. Used by `/healthz`.
pub fn ping(conn: &Connection) -> rusqlite::Result<()> {
    conn.query_row("SELECT 1", [], |r| r.get::<_, i64>(0))
        .map(|_| ())
}

/// Look up an ASIN's BookId without loading the full BookDetail.
pub fn book_id_for_asin(conn: &Connection, asin: &str) -> rusqlite::Result<Option<i64>> {
    conn.query_row(
        "SELECT BookId FROM Books WHERE AudibleProductId = ?1",
        params![asin],
        |r| r.get::<_, i64>(0),
    )
    .optional()
}

/// The admin write. Sets `BookStatus = 0` so Libation re-downloads
/// the book on its next scan. Returns the number of rows changed
/// (0 = no UserDefinedItem row for that BookId).
pub fn requeue_book(conn: &mut Connection, book_id: i64) -> rusqlite::Result<usize> {
    let tx = conn.transaction()?;
    let changed = tx.execute(
        "UPDATE UserDefinedItem SET BookStatus = 0 WHERE BookId = ?1",
        params![book_id],
    )?;
    tx.commit()?;
    Ok(changed)
}

/// The latest `MigrationId` in `__EFMigrationsHistory`, or `None`
/// if the table is missing or empty.
pub fn latest_migration(conn: &Connection) -> Option<String> {
    conn.query_row(
        "SELECT MigrationId FROM __EFMigrationsHistory ORDER BY MigrationId DESC LIMIT 1",
        [],
        |r| r.get::<_, String>(0),
    )
    .ok()
}

/// Report whether the DB's schema head is one the viewer has been
/// tested against. Returns the head id alongside the verdict so the
/// caller can log it.
pub fn check_schema(conn: &Connection) -> (SchemaStatus, Option<String>) {
    let head = latest_migration(conn);
    let status = match &head {
        Some(id) if KNOWN_GOOD_MIGRATIONS.contains(&id.as_str()) => SchemaStatus::Known,
        _ => SchemaStatus::Unknown,
    };
    (status, head)
}

/// List every undeleted book with its authors and narrators.
pub fn list_books(conn: &Connection) -> rusqlite::Result<Vec<BookView>> {
    let mut stmt = conn.prepare(LIST_BOOKS_SQL)?;
    let books = stmt
        .query_map([], map_book_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(books)
}

/// Look up the full detail record for a single ASIN.
pub fn get_book_by_asin(conn: &Connection, asin: &str) -> rusqlite::Result<Option<BookDetail>> {
    let mut stmt = conn.prepare(GET_BOOK_SQL)?;
    let detail = stmt.query_row(params![asin], map_detail_row).optional()?;
    let Some(mut detail) = detail else {
        return Ok(None);
    };

    // Authors + narrators + publishers in one pass over the contributors table.
    let mut authors = Vec::new();
    let mut narrators = Vec::new();
    let mut publishers = Vec::new();
    let mut contrib_stmt = conn.prepare(CONTRIBUTORS_SQL)?;
    let mut rows = contrib_stmt.query(params![detail.view.book_id])?;
    while let Some(row) = rows.next()? {
        let role: i64 = row.get(0)?;
        let name: String = row.get(1)?;
        let name = decode_entities(&name);
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
    let mut series_stmt = conn.prepare(SERIES_SQL)?;
    let mut rows = series_stmt.query(params![detail.view.book_id])?;
    while let Some(row) = rows.next()? {
        let name: Option<String> = row.get(0)?;
        let order: Option<String> = row.get(1)?;
        if let Some(name) = name {
            series.push(SeriesEntry {
                name: decode_entities(&name),
                order,
            });
        }
    }
    detail.series = series;

    // Supplement URLs (zero or more). We surface them as outbound
    // links and never fetch the bytes ourselves.
    let mut supplements = Vec::new();
    let mut supp_stmt = conn.prepare(SUPPLEMENTS_SQL)?;
    let mut rows = supp_stmt.query(params![detail.view.book_id])?;
    while let Some(row) = rows.next()? {
        let url: String = row.get(0)?;
        if !url.is_empty() {
            supplements.push(url);
        }
    }
    detail.supplements = supplements;

    Ok(Some(detail))
}

fn map_book_row(row: &Row<'_>) -> rusqlite::Result<BookView> {
    // Subtitle is TEXT NOT NULL in the schema; empty string = no subtitle.
    let subtitle: String = row.get(3)?;
    let title: String = row.get(2)?;
    let authors_concat: Option<String> = row.get(13)?;
    let narrators_concat: Option<String> = row.get(14)?;
    Ok(BookView {
        book_id: row.get(0)?,
        asin: row.get(1)?,
        title: decode_entities(&title),
        subtitle: if subtitle.is_empty() {
            None
        } else {
            Some(decode_entities(&subtitle))
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
        authors: split_and_decode(authors_concat),
        narrators: split_and_decode(narrators_concat),
    })
}

fn map_detail_row(row: &Row<'_>) -> rusqlite::Result<BookDetail> {
    let subtitle: String = row.get(3)?;
    let title: String = row.get(2)?;
    let description: String = row.get(8)?;
    let picture_large: Option<String> = row.get(9)?;
    Ok(BookDetail {
        view: BookView {
            book_id: row.get(0)?,
            asin: row.get(1)?,
            title: decode_entities(&title),
            subtitle: if subtitle.is_empty() {
                None
            } else {
                Some(decode_entities(&subtitle))
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
        // Description is full HTML markup; downstream callers run
        // `html::paragraphs` on it so the template gets plain-text
        // paragraphs and askama's auto-escape does the rest.
        description,
        publishers: Vec::new(),
        series: Vec::new(),
        picture_large_id: picture_large.filter(|s| !s.is_empty()),
        supplements: Vec::new(),
    })
}

fn split_and_decode(opt: Option<String>) -> Vec<String> {
    match opt {
        None => Vec::new(),
        Some(s) if s.is_empty() => Vec::new(),
        Some(s) => s.split("|||").map(decode_entities).collect(),
    }
}
