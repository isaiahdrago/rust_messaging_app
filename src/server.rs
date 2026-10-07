use axum::{
    extract::State,
    http::{StatusCode, HeaderMap},
    response::Json,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::{AppState};

#[derive(Deserialize)]
pub(crate) struct CreateServerRequest {
    name: String,
    public: bool,
}

#[derive(Serialize)]
pub(crate) struct CreateServerResponse {
    pub(crate) id: i64,
}

#[derive(Deserialize)]
pub(crate) struct EnterServerRequest {
    name: String,
}

pub(crate) async fn server_handler(
    headers: HeaderMap,
    State(app_state): State<Arc<AppState>>,
    Json(payload): Json<CreateServerRequest>,
) -> Result<(StatusCode, Json<CreateServerResponse>), StatusCode> {

    let name = payload.name.trim();
    let username = crate::session_username(&headers, &app_state)
    .ok_or(StatusCode::UNAUTHORIZED)?;

    if name.is_empty() || name.len() > 16 {
        return Err(StatusCode::BAD_REQUEST);
    }

    let owner_id: i64 = sqlx::query_scalar(
        "SELECT id FROM users WHERE username = ?"
        )
        .bind(&username)
        .fetch_optional(&app_state.db)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::UNAUTHORIZED)?;

    let result = sqlx::query(
        "INSERT INTO servers (name, owner_id, public) VALUES (?, ?, ?)"
        )
        .bind(name)
        .bind(owner_id)
        .bind(&payload.public)
        .execute(&app_state.db)
        .await
        .map_err(|_| StatusCode::CONFLICT)?;

    Ok((
        StatusCode::CREATED,
        Json(CreateServerResponse {
            id: result.last_insert_rowid(),
        }),
    ))
}

pub(crate) async fn enter_server_handler(
    State(app_state): State<Arc<AppState>>,
    Json(payload): Json<EnterServerRequest>,
) -> Result<Json<CreateServerResponse>, StatusCode> {
    let name = payload.name.trim();

    if name.is_empty() {
        return Err(StatusCode::BAD_REQUEST);
    }

    let row = sqlx::query("SELECT id FROM servers WHERE name = ?")
        .bind(name)
        .fetch_optional(&app_state.db)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let Some(row) = row else {
        return Err(StatusCode::NOT_FOUND);
    };

    use sqlx::Row;
    let id: i64 = row
        .try_get("id")
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(CreateServerResponse { id }))
}
