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
use std::{collections::HashSet, sync::{Arc, Mutex}};
use sqlx::{sqlite::SqliteConnectOptions, SqlitePool};
use tokio::sync::broadcast;
use uuid::Uuid;

pub(crate) struct AppState {
    pub(crate) db: SqlitePool,
    tx: broadcast::Sender<String>,
    sessions: Mutex<HashSet<Uuid>>,
}

#[allow(private_interfaces)]
mod login;
mod register;
mod delete;

#[tokio::main]
async fn main() {
    let (tx, _rx) = broadcast::channel(16);
    // `chat.db` in the project was a SQL script saved with a database
    // extension, so use a fresh SQLite database file instead.
    let db_options = SqliteConnectOptions::new()
        .filename("chat.sqlite3")
        .create_if_missing(true);
    let db = SqlitePool::connect_with(db_options).await.unwrap();
    sqlx::migrate!("./migrations").run(&db).await.unwrap();
    let app_state = Arc::new(AppState {
        db,
        tx,
        sessions: Mutex::new(HashSet::new()),
    });
    let app = Router::new()
        .route("/", get(login_page))
        .route("/login", post(login::login_handler))
        .route("/register", get(register_page).post(register::register_handler))
        .route("/delete", get(delete_page).post(delete::delete_handler))
        .route("/chat", get(chat_page))
        .route("/ws", get(ws_handler))
        .with_state(app_state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:3000").await.unwrap();
    println!("Server running on ws://127.0.0.1:3000");
    axum::serve(listener, app).await.unwrap();
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
    if !authenticated(&headers, &app_state) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    ws.on_upgrade(|socket| handle_socket(socket, app_state))
}

fn authenticated(headers: &HeaderMap, app_state: &AppState) -> bool {
    let Some(cookie_header) = headers.get(header::COOKIE).and_then(|value| value.to_str().ok()) else {
        return false;
    };

    let Some(session_id) = cookie_header
        .split(';')
        .map(str::trim)
        .find_map(|cookie| cookie.strip_prefix("session="))
        .and_then(|value| Uuid::parse_str(value).ok())
    else {
        return false;
    };

    app_state.sessions.lock().unwrap().contains(&session_id)
}

async fn handle_socket(socket: WebSocket, app_state: Arc<AppState>) {
    let (mut sender, mut receiver) = socket.split();
    let mut rx = app_state.tx.subscribe();

    let app_state_for_broadcast = app_state.clone();
    let mut send_task = tokio::spawn(async move {
        while let Some(Ok(msg)) = receiver.next().await {
            if let Message::Text(text) = msg {
                let _ = app_state_for_broadcast.tx.send(text.to_string());
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
