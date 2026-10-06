use axum::{
    extract::State,
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::Json,
};
use serde::Deserialize;
use sqlx::Row;
use argon2::{Argon2, PasswordHash, PasswordVerifier};
use std::sync::Arc;
use uuid::Uuid;

use crate::AppState;

#[derive(Deserialize)]
pub(crate) struct LoginRequest {
    username: String,
    password: String,
}

pub(crate) async fn login_handler(
    State(app_state): State<Arc<AppState>>,
    Json(payload): Json<LoginRequest>,
) -> Result<(HeaderMap, StatusCode), StatusCode> {
    let row: Option<sqlx::sqlite::SqliteRow> = sqlx::query(
        "SELECT password_hash FROM users WHERE username = ?"
    )
    .bind(&payload.username)
    .fetch_optional(&app_state.db)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let Some(row) = row else {
        return Err(StatusCode::UNAUTHORIZED);
    };

    let stored_hash: String = row.try_get("password_hash").unwrap();

    let parsed_hash = PasswordHash::new(&stored_hash)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let is_valid = Argon2::default()
        .verify_password(payload.password.as_bytes(), &parsed_hash)
        .is_ok();

    if is_valid {
        let session_id = Uuid::new_v4();
        app_state.sessions.lock().unwrap().insert(session_id);

        let mut headers = HeaderMap::new();
        let cookie = format!("session={session_id}; HttpOnly; SameSite=Strict; Path=/");
        headers.insert(
            header::SET_COOKIE,
            HeaderValue::from_str(&cookie).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?,
        );
        Ok((headers, StatusCode::OK))
    } else {
        Err(StatusCode::UNAUTHORIZED)
    }
}
