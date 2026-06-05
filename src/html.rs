//! Tiny helpers for taming the HTML that Libation stores inside text
//! fields. Libation's book descriptions arrive with embedded `<p>`,
//! `<b>`, `<i>` tags plus HTML-entity-encoded characters (`&amp;`,
//! `&#39;`). We strip the markup and decode the common entities so the
//! template can render plain prose paragraphs and askama's default
//! escaper does the right thing.
//!
//! We deliberately avoid pulling in a full HTML sanitiser (e.g.
//! `ammonia`) — the source is a single trusted writer (Libation), the
//! markup vocabulary is tiny, and we never want to render raw HTML
//! verbatim.

use std::sync::OnceLock;

use regex::Regex;

static TAG_RE: OnceLock<Regex> = OnceLock::new();
static PARAGRAPH_RE: OnceLock<Regex> = OnceLock::new();
static WHITESPACE_RE: OnceLock<Regex> = OnceLock::new();
static NUMERIC_ENTITY_RE: OnceLock<Regex> = OnceLock::new();

/// Strip every HTML tag from `s` (best-effort regex sweep; the source
/// is well-formed enough to make this safe in practice).
pub fn strip_tags(s: &str) -> String {
    TAG_RE
        .get_or_init(|| Regex::new(r"<[^>]+>").unwrap())
        .replace_all(s, "")
        .to_string()
}

/// Decode the named entities Libation actually emits plus numeric
/// references (`&#39;`, `&#x27;`). Anything we don't recognise is left
/// alone — that's safer than guessing.
pub fn decode_entities(s: &str) -> String {
    let mut out = s.to_string();
    for (entity, replacement) in [
        ("&amp;", "&"),
        ("&lt;", "<"),
        ("&gt;", ">"),
        ("&quot;", "\""),
        ("&apos;", "'"),
        ("&nbsp;", " "),
    ] {
        if out.contains(entity) {
            out = out.replace(entity, replacement);
        }
    }
    let numeric = NUMERIC_ENTITY_RE.get_or_init(|| Regex::new(r"&#(x?[0-9a-fA-F]+);").unwrap());
    if numeric.is_match(&out) {
        out = numeric
            .replace_all(&out, |caps: &regex::Captures<'_>| {
                let raw = &caps[1];
                let code =
                    if let Some(hex) = raw.strip_prefix('x').or_else(|| raw.strip_prefix('X')) {
                        u32::from_str_radix(hex, 16).ok()
                    } else {
                        raw.parse::<u32>().ok()
                    };
                match code.and_then(char::from_u32) {
                    Some(c) => c.to_string(),
                    None => caps[0].to_string(),
                }
            })
            .to_string();
    }
    out
}

/// Split a Libation description (post-strip, post-decode) into paragraphs.
/// Libation separates paragraphs with `<p>...</p>` blocks; once the tags
/// are gone, we split on runs of whitespace that include a newline, or
/// fall back to the whole thing as a single paragraph.
pub fn paragraphs(raw_html: &str) -> Vec<String> {
    let pre_split = PARAGRAPH_RE
        .get_or_init(|| Regex::new(r"</p>\s*<p[^>]*>").unwrap())
        .split(raw_html);
    let collapse_ws = WHITESPACE_RE.get_or_init(|| Regex::new(r"\s+").unwrap());
    pre_split
        .map(|chunk| decode_entities(&strip_tags(chunk)))
        .map(|s| collapse_ws.replace_all(&s, " ").trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_tags_removes_simple_tags() {
        assert_eq!(strip_tags("<p>hello <b>world</b></p>"), "hello world");
    }

    #[test]
    fn decode_entities_replaces_amp_lt_gt_quot_apos() {
        assert_eq!(
            decode_entities("Spells, Swords, &amp; Stealth &lt;3"),
            "Spells, Swords, & Stealth <3"
        );
        assert_eq!(decode_entities("don&#39;t"), "don't");
        assert_eq!(decode_entities("a&#x27;b"), "a'b");
    }

    #[test]
    fn paragraphs_split_on_p_boundaries() {
        let html = "<p>First paragraph.</p> <p>Second one.</p>";
        assert_eq!(paragraphs(html), vec!["First paragraph.", "Second one."]);
    }

    #[test]
    fn paragraphs_handle_inline_markup_inside() {
        let html = "<p>Hello <b>bold</b> and <i>italics</i>.</p><p>And &amp; more.</p>";
        assert_eq!(
            paragraphs(html),
            vec!["Hello bold and italics.", "And & more."]
        );
    }

    #[test]
    fn paragraphs_handle_no_tags() {
        assert_eq!(paragraphs("just text"), vec!["just text"]);
    }

    #[test]
    fn paragraphs_drop_empty_chunks() {
        assert_eq!(paragraphs("<p></p><p>real</p>"), vec!["real"]);
    }
}
