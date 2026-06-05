//! Compile-time-embedded static assets (htmx, css, placeholder cover).
//!
//! `rust-embed` walks `assets/` at build time and bakes the files into
//! the binary. The `/static/*` handler in `routes::static_route` looks
//! them up by relative path.

use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "assets/"]
pub struct StaticAssets;
