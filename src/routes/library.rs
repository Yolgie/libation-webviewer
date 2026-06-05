use std::path::Path;

use axum::{extract::State, response::Html, routing::get, Router};
use tracing::error;

use crate::db;
use crate::state::AppState;
use crate::view::BookView;

pub fn routes() -> Router<AppState> {
    Router::new().route("/", get(library_list))
}

async fn library_list(State(state): State<AppState>) -> Html<String> {
    match load_books(&state.db_path) {
        Ok(books) => Html(render_library(&books)),
        Err(err) => {
            error!(?err, "DB unavailable; serving degraded library page");
            Html(render_degraded(&err))
        }
    }
}

fn load_books(path: &Path) -> rusqlite::Result<Vec<BookView>> {
    db::Library::open_ro(path)?.list_books()
}

fn render_library(books: &[BookView]) -> String {
    let mut rows = String::new();
    for b in books {
        let status = match b.book_status {
            0 => "Not downloaded".to_string(),
            1 => "Downloaded".to_string(),
            2 => "Error".to_string(),
            3 => "Partial".to_string(),
            n => format!("Unknown({})", n),
        };
        let subtitle_html = b
            .subtitle
            .as_deref()
            .map(|s| format!(r#"<br><span class="muted">{}</span>"#, html_escape(s)))
            .unwrap_or_default();
        rows.push_str(&format!(
            "<tr><td><img src=\"/books/{asin}/thumb\" loading=\"lazy\" width=\"60\" height=\"60\" alt=\"\" class=\"thumb\"></td><td>{title}{subtitle}</td><td>{author}</td><td>{narrator}</td><td>{minutes} min</td><td>{status}</td></tr>",
            asin = html_escape(&b.asin),
            title = html_escape(&b.title),
            subtitle = subtitle_html,
            author = html_escape(&b.authors.join(", ")),
            narrator = html_escape(&b.narrators.join(", ")),
            minutes = b.length_minutes,
            status = status,
        ));
    }
    format!(
        r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<title>libation-webviewer</title>
<style>
body {{ font-family: system-ui, sans-serif; max-width: 1200px; margin: 2em auto; padding: 0 1em; color: #222; }}
h1 {{ margin-bottom: .2em; }}
.muted {{ color: #777; font-size: 0.9em; font-weight: normal; }}
table {{ border-collapse: collapse; width: 100%; margin-top: 1em; }}
th, td {{ padding: .4em .8em; border-bottom: 1px solid #eee; text-align: left; vertical-align: top; }}
th {{ background: #f5f5f5; font-weight: 600; }}
img.thumb {{ display: block; background: #f0f0f0; border-radius: 4px; }}
</style>
</head>
<body>
<h1>Library <span class="muted">{n} books</span></h1>
<table>
<thead><tr><th></th><th>Title</th><th>Author</th><th>Narrator</th><th>Length</th><th>Status</th></tr></thead>
<tbody>
{rows}
</tbody>
</table>
</body>
</html>"#,
        n = books.len(),
        rows = rows,
    )
}

fn render_degraded(err: &rusqlite::Error) -> String {
    format!(
        r#"<!doctype html>
<html lang="en">
<head><meta charset="utf-8"><title>libation-webviewer</title></head>
<body style="font-family: system-ui, sans-serif; max-width: 700px; margin: 4em auto; padding: 0 1em; color: #222;">
<h1>Library data unavailable</h1>
<p>The viewer is running, but Libation's database could not be read.</p>
<pre>{}</pre>
</body>
</html>"#,
        html_escape(&err.to_string())
    )
}

fn html_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            other => out.push(other),
        }
    }
    out
}
