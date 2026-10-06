use axum::{
    extract::State,
    http::StatusCode,
    response::Json,
};
use serde::Deserialize;
use argon2::{Argon2, PasswordHasher};
use std::sync::Arc;

use crate::AppState;

#[derive(Deserialize)]
pub(crate) struct RegisterRequest {
    username: String,
    password: String,
}

pub(crate) async fn register_handler(
    State(app_state): State<Arc<AppState>>,
    Json(payload): Json<RegisterRequest>,
) -> Result<StatusCode, StatusCode> {
    if payload.username.is_empty() || payload.password.is_empty() {
        return Err(StatusCode::BAD_REQUEST);
    }

    let password_hash = Argon2::default()
        .hash_password(payload.password.as_bytes())
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .to_string();

    sqlx::query(
        "INSERT INTO users (username, password_hash) VALUES (?, ?)"
    )
    .bind(&payload.username)
    .bind(&password_hash)
    .execute(&app_state.db)
    .await
    .map_err(|_| StatusCode::CONFLICT)?;

    Ok(StatusCode::CREATED)
}