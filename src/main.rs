use axum::{
    extract::{
        ws::{Message, WebSocket},
        State, WebSocketUpgrade,
    },
    http::{header, HeaderMap, StatusCode},
    response::{Html, IntoResponse, Redirect, Response},
    routing::{get, post},
    Router,
};
use futures_util::{SinkExt, StreamExt};
use std::{collections::HashMap, sync::{Arc, Mutex}};
use sqlx::{PgPool, postgres::PgPoolOptions};
use tokio::sync::broadcast;
use uuid::Uuid;
use std::env;

pub(crate) struct AppState {
    pub(crate) db: PgPool,
    tx: broadcast::Sender<String>,
    sessions: Mutex<HashMap<Uuid, String>>,
}

#[allow(private_interfaces)]
mod login;
mod register;
mod delete;

#[tokio::main]
async fn main() {
    let (tx, _rx) = broadcast::channel(16);
    let db_url = env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    let db = PgPoolOptions::new()
        .max_connections(5)
        .connect(&db_url)
        .await
        .expect("Failed to connect to the database");
    sqlx::migrate!("./migrations").run(&db).await.unwrap();
    let app_state = Arc::new(AppState {
        db,
        tx,
        sessions: Mutex::new(HashMap::new()),
    });
    let app = Router::new()
        .route("/", get(login_page))
        .route("/login", post(login::login_handler))
        .route("/register", get(register_page).post(register::register_handler))
        .route("/delete", get(delete_page).post(delete::delete_handler))
        .route("/chat", get(chat_page))
        .route("/ws", get(ws_handler))
        .route("/healthz", get(health_check))
        .with_state(app_state);
    let host = env::var("HOST").unwrap_or_else(|_| "0.0.0.0".to_owned());
    let port = env::var("PORT").unwrap_or_else(|_| "3000".to_owned());
    let address = format!("{host}:{port}");
    let listener = tokio::net::TcpListener::bind(&address).await.unwrap();
    println!("Server running on {address}");
    axum::serve(listener, app).await.unwrap();
}

async fn health_check() -> StatusCode {
    StatusCode::NO_CONTENT
}

async fn login_page() -> Html<&'static str> {
    Html(include_str!("../html/login.html"))
}

async fn register_page() -> Html<&'static str> {
    Html(include_str!("../html/register.html"))
}

async fn delete_page() -> Html<&'static str> {
    Html(include_str!("../html/delete.html"))
}


async fn chat_page(
    headers: HeaderMap,
    State(app_state): State<Arc<AppState>>,
) -> Response {
    if authenticated(&headers, &app_state) {
        Html(include_str!("../html/index.html")).into_response()
    } else {
        Redirect::to("/").into_response()
    }
}

async fn ws_handler(
    ws: WebSocketUpgrade,
    headers: HeaderMap,
    State(app_state): State<Arc<AppState>>,
) -> Response {
    let Some(username) = session_username(&headers, &app_state) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    ws.on_upgrade(|socket| handle_socket(socket, app_state, username))
}

fn authenticated(headers: &HeaderMap, app_state: &AppState) -> bool {
    session_username(headers, app_state).is_some()
}

fn session_username(headers: &HeaderMap, app_state: &AppState) -> Option<String> {
    let Some(cookie_header) = headers.get(header::COOKIE).and_then(|value| value.to_str().ok()) else {
        return None;
    };

    let Some(session_id) = cookie_header
        .split(';')
        .map(str::trim)
        .find_map(|cookie| cookie.strip_prefix("session="))
        .and_then(|value| Uuid::parse_str(value).ok())
    else {
        return None;
    };

    app_state.sessions.lock().unwrap().get(&session_id).cloned()
}

async fn handle_socket(socket: WebSocket, app_state: Arc<AppState>, username: String) {
    let (mut sender, mut receiver) = socket.split();
    let mut rx = app_state.tx.subscribe();

    let app_state_for_broadcast = app_state.clone();
    let mut send_task = tokio::spawn(async move {
        while let Some(Ok(msg)) = receiver.next().await {
            if let Message::Text(text) = msg {
                let message = format!("{username}: {text}");
                let _ = app_state_for_broadcast.tx.send(message);
            }
        }
    });

    let mut recv_task = tokio::spawn(async move {
        while let Ok(text) = rx.recv().await {
            if sender.send(Message::Text(text.into())).await.is_err() {
                break;
            }
        }
    });

    tokio::select! {
        _ = (&mut send_task) => recv_task.abort(),
        _ = (&mut recv_task) => send_task.abort(),
    };
}
