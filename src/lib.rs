//! The crate ships as the `libation-webviewer` binary. The lib surface
//! exists so the integration tests under `tests/` can drive the router,
//! auth backend, DB layer, etc. directly; consumers outside this repo
//! should not depend on it. Internal-only modules (`html`, `query`,
//! `static_assets`) are scoped `pub(crate)`.

pub mod auth;
pub mod cover;
pub mod db;
pub mod error;
pub mod fs;
pub(crate) mod html;
pub(crate) mod query;
pub mod routes;
pub mod state;
pub(crate) mod static_assets;
pub mod view;
