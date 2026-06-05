use serde::Serialize;

/// Compact summary used in the library list.
#[derive(Debug, Clone, Serialize)]
pub struct BookView {
    pub book_id: i64,
    pub asin: String,
    pub title: String,
    pub subtitle: Option<String>,
    pub length_minutes: i64,
    pub locale: String,
    pub language: Option<String>,
    pub date_published: Option<String>,
    pub book_status: i32,
    pub is_finished: bool,
    pub date_added: Option<String>,
    pub is_audible_plus: bool,
    pub absent_from_last_scan: bool,
    pub authors: Vec<String>,
    pub narrators: Vec<String>,
}

/// Full per-book record assembled for the detail page.
#[derive(Debug, Clone, Serialize)]
pub struct BookDetail {
    pub view: BookView,
    pub description: String,
    pub publishers: Vec<String>,
    pub series: Vec<SeriesEntry>,
    pub picture_large_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SeriesEntry {
    pub name: String,
    pub order: Option<String>,
}
