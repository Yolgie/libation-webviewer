pub mod admin;
pub mod book;
pub mod health;
pub mod library;

use axum::Router;

pub fn router() -> Router {
    Router::new()
        .merge(health::routes())
        .merge(library::routes())
        .merge(book::routes())
        .merge(admin::routes())
}
