//! Library list query parameters: search text, sort key, status filter.
//! Applied in-memory to the assembled BookView list, which is small
//! enough (64 books in the sample) that there's no need to push the
//! work back into SQL.

use serde::Deserialize;

use crate::view::BookView;

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct LibraryQuery {
    /// Free-text search across title, subtitle, authors, narrators.
    pub q: String,
    /// Sort key. Defaults to `title` (case-insensitive). Recognised:
    /// `title`, `author`, `length`, `date_added`.
    pub sort: String,
    /// Status filter. Recognised: `all` (default), `downloaded`,
    /// `not_downloaded`.
    pub status: String,
}

impl LibraryQuery {
    /// Whether the query carries any filters or non-default sort.
    /// Useful for "no filters → show everything" decisions in the view.
    pub fn is_active(&self) -> bool {
        !self.q.is_empty()
            || (!self.sort.is_empty() && self.sort != "title")
            || (!self.status.is_empty() && self.status != "all")
    }

    /// Render the query as a `?q=…&sort=…&status=…` suffix that the
    /// `HX-Push-URL` header points the browser bar at.
    pub fn url_querystring(&self) -> String {
        let mut parts = Vec::new();
        if !self.q.is_empty() {
            parts.push(format!("q={}", urlencode(&self.q)));
        }
        if !self.sort.is_empty() && self.sort != "title" {
            parts.push(format!("sort={}", urlencode(&self.sort)));
        }
        if !self.status.is_empty() && self.status != "all" {
            parts.push(format!("status={}", urlencode(&self.status)));
        }
        if parts.is_empty() {
            "/".to_string()
        } else {
            format!("/?{}", parts.join("&"))
        }
    }
}

fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.as_bytes() {
        let c = *b as char;
        if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '~') {
            out.push(c);
        } else {
            out.push_str(&format!("%{:02X}", b));
        }
    }
    out
}

/// Filter and sort `books` according to `q`. The input is consumed; the
/// returned vector is the post-filter, post-sort result.
pub fn apply(mut books: Vec<BookView>, q: &LibraryQuery) -> Vec<BookView> {
    // Status filter.
    match q.status.as_str() {
        "downloaded" => books.retain(|b| b.book_status == 1),
        "not_downloaded" => books.retain(|b| b.book_status != 1),
        _ => {} // "all" or unset
    }

    // Free-text search (ASCII case-insensitive substring against
    // title / subtitle / authors / narrators). Accented characters
    // are matched literally on both sides — fine for English-language
    // metadata, which is the bulk of Libation catalogues.
    let needle = q.q.trim().to_ascii_lowercase();
    if !needle.is_empty() {
        books.retain(|b| matches_text(b, &needle));
    }

    // Sort. The case-insensitive key sorts use `sort_by_cached_key` so
    // each book's lowercased key is computed once per sort rather than
    // once per comparison.
    match q.sort.as_str() {
        "author" => books.sort_by_cached_key(|b| b.authors.first().map(|s| s.to_lowercase())),
        "length" => books.sort_by_key(|b| std::cmp::Reverse(b.length_minutes)),
        "date_added" => books.sort_by(|a, b| b.date_added.cmp(&a.date_added)),
        _ => books.sort_by_cached_key(|b| b.title.to_lowercase()),
    }

    books
}

fn matches_text(b: &BookView, needle: &str) -> bool {
    if contains_ignore_ascii_case(&b.title, needle) {
        return true;
    }
    if let Some(sub) = &b.subtitle {
        if contains_ignore_ascii_case(sub, needle) {
            return true;
        }
    }
    if b.authors
        .iter()
        .any(|s| contains_ignore_ascii_case(s, needle))
    {
        return true;
    }
    if b.narrators
        .iter()
        .any(|s| contains_ignore_ascii_case(s, needle))
    {
        return true;
    }
    false
}

/// Substring contains, ASCII-case-insensitive, zero-alloc. `needle`
/// is assumed lowercased by the caller; we ASCII-fold each window
/// of `haystack` in place via `eq_ignore_ascii_case`. Worst case is
/// O(haystack * needle), which is fine for title-length strings.
fn contains_ignore_ascii_case(haystack: &str, needle: &str) -> bool {
    let h = haystack.as_bytes();
    let n = needle.as_bytes();
    if n.is_empty() {
        return true;
    }
    if h.len() < n.len() {
        return false;
    }
    h.windows(n.len()).any(|w| w.eq_ignore_ascii_case(n))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::view::BookView;

    fn book(title: &str, author: &str, length: i64, status: i32) -> BookView {
        BookView {
            book_id: 0,
            asin: "x".into(),
            title: title.into(),
            subtitle: None,
            length_minutes: length,
            locale: "us".into(),
            language: None,
            date_published: None,
            book_status: status,
            is_finished: false,
            date_added: Some("2024-01-01".into()),
            is_audible_plus: false,
            absent_from_last_scan: false,
            authors: vec![author.into()],
            narrators: vec![],
        }
    }

    fn fixture() -> Vec<BookView> {
        vec![
            book("Banana Republic", "Author A", 100, 1),
            book("Apple Pie", "Author Z", 200, 0),
            book("Cherry Cake", "Author B", 300, 1),
        ]
    }

    #[test]
    fn default_sort_is_title_case_insensitive() {
        let q = LibraryQuery::default();
        let titles: Vec<_> = apply(fixture(), &q).into_iter().map(|b| b.title).collect();
        assert_eq!(titles, vec!["Apple Pie", "Banana Republic", "Cherry Cake"]);
    }

    #[test]
    fn sort_by_length_descending() {
        let q = LibraryQuery {
            sort: "length".into(),
            ..LibraryQuery::default()
        };
        let lens: Vec<_> = apply(fixture(), &q)
            .into_iter()
            .map(|b| b.length_minutes)
            .collect();
        assert_eq!(lens, vec![300, 200, 100]);
    }

    #[test]
    fn sort_by_author_case_insensitive() {
        let q = LibraryQuery {
            sort: "author".into(),
            ..LibraryQuery::default()
        };
        let authors: Vec<_> = apply(fixture(), &q)
            .into_iter()
            .flat_map(|b| b.authors)
            .collect();
        assert_eq!(authors, vec!["Author A", "Author B", "Author Z"]);
    }

    #[test]
    fn search_matches_title_substring() {
        let q = LibraryQuery {
            q: "cherry".into(),
            ..LibraryQuery::default()
        };
        let titles: Vec<_> = apply(fixture(), &q).into_iter().map(|b| b.title).collect();
        assert_eq!(titles, vec!["Cherry Cake"]);
    }

    #[test]
    fn search_matches_author_substring() {
        let q = LibraryQuery {
            q: "author z".into(),
            ..LibraryQuery::default()
        };
        let titles: Vec<_> = apply(fixture(), &q).into_iter().map(|b| b.title).collect();
        assert_eq!(titles, vec!["Apple Pie"]);
    }

    #[test]
    fn status_filter_downloaded() {
        let q = LibraryQuery {
            status: "downloaded".into(),
            ..LibraryQuery::default()
        };
        let titles: Vec<_> = apply(fixture(), &q).into_iter().map(|b| b.title).collect();
        assert_eq!(titles, vec!["Banana Republic", "Cherry Cake"]);
    }

    #[test]
    fn status_filter_not_downloaded() {
        let q = LibraryQuery {
            status: "not_downloaded".into(),
            ..LibraryQuery::default()
        };
        let titles: Vec<_> = apply(fixture(), &q).into_iter().map(|b| b.title).collect();
        assert_eq!(titles, vec!["Apple Pie"]);
    }

    #[test]
    fn is_active_detects_filters() {
        assert!(!LibraryQuery::default().is_active());
        assert!(LibraryQuery {
            q: "x".into(),
            ..LibraryQuery::default()
        }
        .is_active());
        assert!(LibraryQuery {
            status: "downloaded".into(),
            ..LibraryQuery::default()
        }
        .is_active());
        assert!(!LibraryQuery {
            status: "all".into(),
            ..LibraryQuery::default()
        }
        .is_active());
    }

    #[test]
    fn url_querystring_round_trip() {
        let q = LibraryQuery {
            q: "andy weir".into(),
            sort: "length".into(),
            status: "downloaded".into(),
        };
        let s = q.url_querystring();
        assert!(s.starts_with("/?"));
        assert!(s.contains("q=andy%20weir"));
        assert!(s.contains("sort=length"));
        assert!(s.contains("status=downloaded"));
    }

    #[test]
    fn contains_ignore_ascii_case_matches_and_misses() {
        // Hit: mixed-case match anywhere in the haystack.
        assert!(super::contains_ignore_ascii_case(
            "Project Hail Mary",
            "hail"
        ));
        assert!(super::contains_ignore_ascii_case(
            "Project Hail Mary",
            "MARY"
        ));
        assert!(super::contains_ignore_ascii_case("a", "a"));
        // Miss: shorter haystack, unrelated text.
        assert!(!super::contains_ignore_ascii_case(
            "Project",
            "Project Hail"
        ));
        assert!(!super::contains_ignore_ascii_case("foo", "bar"));
        // Empty needle is trivially contained (caller already guards
        // against this; verify the helper is consistent).
        assert!(super::contains_ignore_ascii_case("anything", ""));
    }

    #[test]
    fn url_querystring_omits_defaults() {
        let q = LibraryQuery {
            sort: "title".into(),
            status: "all".into(),
            ..LibraryQuery::default()
        };
        assert_eq!(q.url_querystring(), "/");
    }
}
