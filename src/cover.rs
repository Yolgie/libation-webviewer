//! Embedded cover extraction, image resize, and on-disk cache.
//!
//! Dispatch by extension:
//!   * `.m4b` / `.m4a` / `.mp4` -> mp4ameta reads the `covr` atom.
//!   * `.mp3`                   -> id3 reads the first `APIC` frame.
//! The cache lives under `<cache_dir>/covers/<asin>/`. Filenames are
//! keyed on a short sha256 of (source mtime, source size) so the
//! cache invalidates automatically when Libation re-downloads a book.

use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::time::{Duration, UNIX_EPOCH};

use image::ImageFormat;
use sha2::{Digest, Sha256};
use tokio::fs;

#[derive(Debug, Clone, Copy)]
pub enum CoverFormat {
    Jpeg,
    Png,
}

impl CoverFormat {
    fn ext(self) -> &'static str {
        match self {
            CoverFormat::Jpeg => "jpg",
            CoverFormat::Png => "png",
        }
    }

    pub fn mime(self) -> &'static str {
        match self {
            CoverFormat::Jpeg => "image/jpeg",
            CoverFormat::Png => "image/png",
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum CoverError {
    #[error("no embedded cover")]
    NotFound,
    #[error("parse failed: {0}")]
    Parse(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("decode: {0}")]
    Decode(#[from] image::ImageError),
}

/// Extract embedded cover bytes from the audio file.
pub fn extract(path: &Path) -> Result<(Vec<u8>, CoverFormat), CoverError> {
    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "m4b" | "m4a" | "mp4" => extract_mp4(path),
        "mp3" => extract_mp3(path),
        other => Err(CoverError::Parse(format!("unsupported extension: {}", other))),
    }
}

fn extract_mp4(path: &Path) -> Result<(Vec<u8>, CoverFormat), CoverError> {
    let tag = mp4ameta::Tag::read_from_path(path).map_err(|e| CoverError::Parse(e.to_string()))?;
    let img = tag.artwork().ok_or(CoverError::NotFound)?;
    let fmt = match img.fmt {
        mp4ameta::ImgFmt::Jpeg => CoverFormat::Jpeg,
        mp4ameta::ImgFmt::Png => CoverFormat::Png,
        mp4ameta::ImgFmt::Bmp => {
            return Err(CoverError::Parse("BMP cover not supported".into()));
        }
    };
    Ok((img.data.to_vec(), fmt))
}

fn extract_mp3(path: &Path) -> Result<(Vec<u8>, CoverFormat), CoverError> {
    let tag = id3::Tag::read_from_path(path).map_err(|e| CoverError::Parse(e.to_string()))?;
    let pic = tag.pictures().next().ok_or(CoverError::NotFound)?;
    let fmt = if pic.mime_type.to_ascii_lowercase().contains("png") {
        CoverFormat::Png
    } else {
        CoverFormat::Jpeg
    };
    Ok((pic.data.clone(), fmt))
}

/// Resize image bytes to fit within `max_dim` x `max_dim` (preserving aspect
/// ratio) and re-encode as WebP.
pub fn resize_to_webp(bytes: &[u8], max_dim: u32) -> Result<Vec<u8>, CoverError> {
    let img = image::load_from_memory(bytes)?;
    let resized = img.resize(max_dim, max_dim, image::imageops::FilterType::Lanczos3);
    let mut out = Vec::new();
    resized.write_to(&mut Cursor::new(&mut out), ImageFormat::WebP)?;
    Ok(out)
}

/// Short stable cache key from the source file's (mtime, size).
pub fn src_hash(path: &Path) -> std::io::Result<String> {
    let meta = std::fs::metadata(path)?;
    let mtime = meta
        .modified()?
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO);
    let key = format!("{}:{}", mtime.as_secs(), meta.len());
    let digest = Sha256::digest(key.as_bytes());
    Ok(hex::encode(&digest[..8])) // 16 hex chars
}

/// Return cached or freshly-built cover bytes plus their content-type.
pub async fn get_or_build(
    cache_dir: &Path,
    asin: &str,
    source: &Path,
    want_thumb: bool,
) -> Result<(Vec<u8>, &'static str), CoverError> {
    let hash = src_hash(source)?;
    let dir = cache_dir.join("covers").join(asin);
    fs::create_dir_all(&dir).await?;

    if want_thumb {
        let cached = dir.join(format!("thumb-{}.webp", hash));
        if let Ok(bytes) = fs::read(&cached).await {
            return Ok((bytes, "image/webp"));
        }
        let (orig_bytes, _) = build_orig(&dir, &hash, source).await?;
        let thumb = resize_to_webp(&orig_bytes, 200)?;
        fs::write(&cached, &thumb).await?;
        return Ok((thumb, "image/webp"));
    }

    for fmt in [CoverFormat::Jpeg, CoverFormat::Png] {
        let cached = dir.join(format!("orig-{}.{}", hash, fmt.ext()));
        if let Ok(bytes) = fs::read(&cached).await {
            return Ok((bytes, fmt.mime()));
        }
    }
    let (bytes, fmt) = build_orig(&dir, &hash, source).await?;
    Ok((bytes, fmt.mime()))
}

async fn build_orig(
    dir: &Path,
    hash: &str,
    source: &Path,
) -> Result<(Vec<u8>, CoverFormat), CoverError> {
    let source_owned: PathBuf = source.to_path_buf();
    let (bytes, fmt) = tokio::task::spawn_blocking(move || extract(&source_owned))
        .await
        .map_err(|e| CoverError::Parse(format!("spawn_blocking: {}", e)))??;
    let cached = dir.join(format!("orig-{}.{}", hash, fmt.ext()));
    fs::write(&cached, &bytes).await?;
    Ok((bytes, fmt))
}
