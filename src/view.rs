use serde::Serialize;

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
