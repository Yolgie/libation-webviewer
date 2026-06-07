//! Book-folder scanner and ASIN parser.
//!
//! The folder name must contain `[ASIN]` — that's how Libation lays out
//! the books directory. Folders without the token are skipped (and
//! logged), which is the documented "no [ASIN] -> ignore" graceful path.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use regex::Regex;
use tracing::{info, warn};

#[derive(Debug, Clone)]
pub struct BookFiles {
    pub folder: PathBuf,
    /// Audio files in the folder, alpha-sorted (`.m4b`, `.m4a`, `.mp4`, `.mp3`).
    pub audio_files: Vec<PathBuf>,
    /// `<title> [<ASIN>].metadata.json`, if present.
    pub metadata_json: Option<PathBuf>,
}

static ASIN_RE: OnceLock<Regex> = OnceLock::new();

fn asin_regex() -> &'static Regex {
    ASIN_RE.get_or_init(|| Regex::new(r"\[([A-Z0-9]{10})\]").unwrap())
}

/// Extract the ASIN token from a folder name like `Some Title [B08G9RZBTT]`.
pub fn parse_asin(name: &str) -> Option<String> {
    asin_regex().captures(name).map(|c| c[1].to_string())
}

/// Walk `root` looking for a sub-directory whose name carries the given
/// `[ASIN]` token. Returns the matching folder path, or `None`. An
/// unreadable root logs a warning and returns `None`.
pub fn find_book_folder(root: &Path, asin: &str) -> Option<PathBuf> {
    let read = match std::fs::read_dir(root) {
        Ok(r) => r,
        Err(err) => {
            warn!(path = %root.display(), %err, "books root unreadable; cannot resolve ASIN");
            return None;
        }
    };
    for entry in read.flatten() {
        let path = entry.path();
        match entry.file_type() {
            Ok(t) if t.is_dir() => {}
            _ => continue,
        }
        // Skip (don't abort) on names we can't parse as UTF-8 — a single
        // unrelated non-UTF-8 sibling shouldn't make every later ASIN
        // lookup fail depending on `readdir` order. Matches `scan_books`.
        let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        if parse_asin(name).as_deref() == Some(asin) {
            return Some(path);
        }
    }
    None
}

/// Live per-request scan: locate `asin`'s folder under `root` and read
/// its current contents. Returns `None` if no folder with that ASIN
/// exists yet (e.g., Libation hasn't downloaded it). This is cheap —
/// two `readdir` calls — and is the source of truth for per-book
/// routes so that downloads which appear post-startup are visible
/// without restarting the container.
pub fn scan_one(root: &Path, asin: &str) -> Option<BookFiles> {
    let folder = find_book_folder(root, asin)?;
    let (audio_files, metadata_json) = scan_book_dir(&folder);
    Some(BookFiles {
        folder,
        audio_files,
        metadata_json,
    })
}

/// Walk `root` and return a map of ASIN -> BookFiles. Missing/unreadable
/// roots yield an empty map with a warning logged; they never panic.
pub fn scan_books(root: impl AsRef<Path>) -> HashMap<String, BookFiles> {
    let root = root.as_ref();
    let mut out = HashMap::new();
    let read = match std::fs::read_dir(root) {
        Ok(r) => r,
        Err(err) => {
            warn!(path = %root.display(), %err, "books root unreadable; scan returns empty");
            return out;
        }
    };
    for entry in read.flatten() {
        let path = entry.path();
        match entry.file_type() {
            Ok(t) if t.is_dir() => {}
            _ => continue,
        }
        let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        let Some(asin) = parse_asin(name) else {
            info!(folder = %name, "folder skipped: no [ASIN] token");
            continue;
        };
        let (audio_files, metadata_json) = scan_book_dir(&path);
        out.insert(
            asin,
            BookFiles {
                folder: path,
                audio_files,
                metadata_json,
            },
        );
    }
    out
}

fn scan_book_dir(dir: &Path) -> (Vec<PathBuf>, Option<PathBuf>) {
    let mut audio = Vec::new();
    let mut meta = None;
    let Ok(read) = std::fs::read_dir(dir) else {
        return (audio, meta);
    };
    for f in read.flatten() {
        let fp = f.path();
        let Some(ext) = fp.extension().and_then(|s| s.to_str()) else {
            continue;
        };
        match ext.to_ascii_lowercase().as_str() {
            "m4b" | "m4a" | "mp4" | "mp3" => audio.push(fp),
            "json" if fp.to_string_lossy().ends_with(".metadata.json") => {
                meta = Some(fp);
            }
            _ => {}
        }
    }
    audio.sort();
    (audio, meta)
}
