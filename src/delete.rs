use axum::{
    extract::State, http::{HeaderMap, StatusCode}, response::Json,
};
use serde::{Deserialize};
use sqlx::Row;
use argon2::{Argon2, PasswordHash, PasswordVerifier};
use std::sync::Arc;

use crate::AppState;

#[derive(Deserialize)]
pub(crate) struct DeleteRequest {
    username: String,
    password: String,
}

#[derive(Deserialize)]
pub(crate) struct DeleteServerRequest {
    server: String,
}

pub(crate) async fn delete_user_handler(
    State(app_state): State<Arc<AppState>>,
    Json(payload): Json<DeleteRequest>,
) -> Result<StatusCode, StatusCode> {
    if payload.username.is_empty() || payload.password.is_empty() {
        return Err(StatusCode::BAD_REQUEST);
    
    }
    let row = sqlx::query(
        "SELECT id, password_hash FROM users WHERE username = ?"
    )
    .bind(&payload.username)
    .fetch_optional(&app_state.db)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let Some(row) = row else {
        return Err(StatusCode::UNAUTHORIZED);
    };

    let stored_hash: String = row.try_get("password_hash").unwrap();
    let owner_id: i64 = row
        .try_get("id")
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let parsed_hash = PasswordHash::new(&stored_hash)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let is_valid = Argon2::default()
        .verify_password(payload.password.as_bytes(), &parsed_hash)
        .is_ok();

    if is_valid {
        sqlx::query(
            "DELETE FROM servers WHERE owner_id = ?"
            )
            .bind(owner_id)
            .execute(&app_state.db)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

        sqlx::query(
            "DELETE FROM users WHERE id = ?"
                )
                .bind(owner_id)
                .execute(&app_state.db)
                .await
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

        Ok(StatusCode::OK)
    } else {
        Err(StatusCode::UNAUTHORIZED)
    }
}

pub(crate) async fn delete_server_handler(
    headers: HeaderMap,
    State(app_state): State<Arc<AppState>>,
    Json(payload): Json<DeleteServerRequest>,
) -> Result<StatusCode, StatusCode> {
    if payload.server.is_empty() {
        return Err(StatusCode::BAD_REQUEST);
    
    }
    let row = sqlx::query(
        "SELECT owner_id FROM servers WHERE name = ?"
    )
    .bind(&payload.server)
    .fetch_optional(&app_state.db)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let Some(row) = row else {
        return Err(StatusCode::UNAUTHORIZED);
    };

    let username = crate::session_username(&headers, &app_state)
        .ok_or(StatusCode::UNAUTHORIZED)?;

    let owner_id: i64 = sqlx::query_scalar(
        "SELECT id FROM users WHERE username = ?"
        )
        .bind(&username)
        .fetch_optional(&app_state.db)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::UNAUTHORIZED)?;

    let server_owner_id: i64 = row
        .try_get("owner_id")
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if owner_id == server_owner_id {
        sqlx::query(
            "DELETE FROM servers WHERE name = ?"
        )
        .bind(&payload.server)
        .execute(&app_state.db)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

        Ok(StatusCode::OK)
    } else {
        Err(StatusCode::UNAUTHORIZED)
    }
}
