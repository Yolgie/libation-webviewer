use askama::Template;
use axum::{
    extract::{Form, Path, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
    Router,
};
use serde::Deserialize;
use tracing::{error, info, warn};

use crate::auth;
use crate::db;
use crate::state::AppState;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/admin/login", get(login_form).post(login_submit))
        .route("/admin/logout", post(logout))
        .route("/books/{asin}/requeue", post(requeue))
}

#[derive(Template)]
#[template(path = "admin_login.html")]
struct AdminLoginTemplate<'a> {
    error: Option<&'a str>,
    requires_password: bool,
}

async fn login_form(State(state): State<AppState>) -> Response {
    render(AdminLoginTemplate {
        error: None,
        requires_password: state.auth.requires_password(),
    })
}

#[derive(Debug, Deserialize)]
pub struct LoginForm {
    password: String,
}

async fn login_submit(State(state): State<AppState>, Form(form): Form<LoginForm>) -> Response {
    if !state.auth.verify_password(&form.password) {
        warn!("admin login failed: bad password");
        return (
            StatusCode::UNAUTHORIZED,
            render(AdminLoginTemplate {
                error: Some("Wrong password."),
                requires_password: state.auth.requires_password(),
            }),
        )
            .into_response();
    }
    let token = state.auth.issue_token();
    info!("admin login successful");

    let mut headers = HeaderMap::new();
    if let Ok(v) = HeaderValue::from_str(&auth::make_set_cookie(&token)) {
        headers.insert(header::SET_COOKIE, v);
    }
    // 303 See Other so the browser issues a fresh GET / and the cookie
    // is sent on that follow-up request.
    headers.insert(header::LOCATION, HeaderValue::from_static("/"));
    (StatusCode::SEE_OTHER, headers).into_response()
}

async fn logout() -> Response {
    // Sessions are stateless (signed cookies), so logout is just
    // clearing the cookie at the browser. Nothing to invalidate
    // server-side.
    let mut h = HeaderMap::new();
    if let Ok(v) = HeaderValue::from_str(&auth::make_clear_cookie()) {
        h.insert(header::SET_COOKIE, v);
    }
    h.insert(header::LOCATION, HeaderValue::from_static("/"));
    (StatusCode::SEE_OTHER, h).into_response()
}

#[derive(Template)]
#[template(path = "requeue_result.html")]
struct RequeueResultTemplate<'a> {
    asin: &'a str,
    /// `None` on success, `Some(reason)` for the various failure paths.
    error: Option<&'a str>,
}

async fn requeue(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(asin): Path<String>,
) -> Response {
    if !state.auth.is_authenticated(&headers) {
        return error_response(StatusCode::UNAUTHORIZED, &asin, "Login required.");
    }
    if !state.admin_writes_allowed {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            &asin,
            "Admin writes disabled (schema head not in the known-good list).",
        );
    }
    let Some(db_rw_path) = &state.db_path_rw else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            &asin,
            "No writable DB configured (LIBATION_DB_RW unset).",
        );
    };

    // Resolve ASIN -> BookId via the read-only pool.
    let book_id = {
        let asin_for_query = asin.clone();
        match state
            .db
            .ro(move |conn| db::book_id_for_asin(conn, &asin_for_query))
            .await
        {
            Ok(Some(id)) => id,
            Ok(None) => {
                return error_response(
                    StatusCode::NOT_FOUND,
                    &asin,
                    "No book with that ASIN in the library.",
                );
            }
            Err(err) => {
                error!(?err, "DB read failed during requeue");
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, &asin, "DB read failed.");
            }
        }
    };

    // Open + write the RW handle on the blocking pool. RW connections
    // are not pooled (PLAN.md "Admin-mode mechanics"); we open one,
    // run the single UPDATE, and drop it.
    let rw_path = db_rw_path.clone();
    let rows = tokio::task::spawn_blocking(move || -> rusqlite::Result<usize> {
        let mut lib_rw = db::Library::open_rw(&rw_path)?;
        lib_rw.requeue_book(book_id)
    })
    .await;

    match rows {
        Err(join_err) => {
            error!(?join_err, "requeue blocking task panicked");
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &asin,
                "Write failed (see server logs).",
            )
        }
        Ok(Err(err)) => {
            error!(?err, "requeue UPDATE failed");
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &asin,
                "Write failed (see server logs).",
            )
        }
        Ok(Ok(0)) => {
            warn!(
                asin = %asin, %book_id,
                "requeue: UPDATE matched no row (no UserDefinedItem entry for this book)"
            );
            render(RequeueResultTemplate {
                asin: &asin,
                error: Some("No UserDefinedItem row matched - nothing to update."),
            })
        }
        Ok(Ok(rows)) => {
            info!(asin = %asin, %book_id, %rows, "requeue: BookStatus reset to 0");
            render(RequeueResultTemplate {
                asin: &asin,
                error: None,
            })
        }
    }
}

fn render<T: Template>(t: T) -> Response {
    match t.render() {
        Ok(html) => Html(html).into_response(),
        Err(err) => {
            error!(?err, "admin template render failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "template error").into_response()
        }
    }
}

fn error_response(status: StatusCode, asin: &str, msg: &str) -> Response {
    let tpl = RequeueResultTemplate {
        asin,
        error: Some(msg),
    };
    match tpl.render() {
        Ok(html) => (status, Html(html)).into_response(),
        Err(_) => (status, msg.to_string()).into_response(),
    }
}
