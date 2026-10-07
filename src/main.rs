use axum::{
    Router, extract::{
        Path, Query, State, WebSocketUpgrade, ws::{Message, WebSocket},
    }, http::{HeaderMap, StatusCode, header}, response::{Html, IntoResponse, Redirect, Response}, routing::{get, post},
};
use futures_util::{SinkExt, StreamExt};
use std::{collections::HashMap, sync::{Arc, Mutex, RwLock}};
use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};
use tokio::sync::broadcast;
use uuid::Uuid;
use serde::{Deserialize};
use std::env;
use tower_http::services::ServeDir;

pub(crate) struct AppState {
    pub(crate) db: SqlitePool,
    tx: broadcast::Sender<String>,
    room_channels: RwLock<HashMap<i64, broadcast::Sender<String>>>,
    sessions: Mutex<HashMap<Uuid, String>>,
    dynamic_routes: Arc<RwLock<HashMap<String, String>>>,
}

#[derive(Deserialize)]
pub(crate) struct WebSocketQuery {
    pub(crate) server_id: i64,
}

#[allow(private_interfaces)]
mod login;
mod register;
mod delete;
mod server;

#[tokio::main]
async fn main() {
    let (tx, _rx) = broadcast::channel(16);
    let db = SqlitePoolOptions::new()
        .max_connections(5)
        .connect("sqlite://chat.sqlite3")
        .await
        .expect("Failed to connect to the database");
    sqlx::migrate!("./migrations").run(&db).await.unwrap();
    let app_state = Arc::new(AppState {
        db,
        tx,
        room_channels: RwLock::new(HashMap::new()),
        sessions: Mutex::new(HashMap::new()),
        dynamic_routes: Arc::new(RwLock::new(HashMap::new())),
    });
    let app = Router::new()
        .route("/", get(login_page))
        .route("/login", post(login::login_handler))
        .route("/register", get(register_page).post(register::register_handler))
        .route("/delete", get(delete_page).post(delete::delete_user_handler))
        .route("/delete_server", get(delete_server_page).post(delete::delete_server_handler))
        .route("/server_pathing", get(server_page))
        .route("/server", post(server::server_handler))
        .route("/enter-server", post(server::enter_server_handler))
        .route("/servers/{id}", get(server_instance_page))
        .route("/add_route", post(register_new_page))
        .route("/ws", get(ws_handler))
        .nest_service("/images", ServeDir::new("images"))
        .with_state(app_state);
    let host = env::var("HOST").unwrap_or_else(|_| "0.0.0.0".to_owned());
    let port = env::var("PORT").unwrap_or_else(|_| "3000".to_owned());
    let address = format!("{host}:{port}");
    let listener = tokio::net::TcpListener::bind(&address).await.unwrap();
    println!("Server running on {address}");
    axum::serve(listener, app).await.unwrap();
}

async fn login_page() -> Html<&'static str> {
    Html(include_str!("../html/login.html"))
}

async fn server_page() -> Html<&'static str> {
    Html(include_str!("../html/server_pathing.html"))
}

async fn server_instance_page(
    Path(id): Path<i64>,
    headers: HeaderMap,
    State(app_state): State<Arc<AppState>>,
) -> Response {
    if !authenticated(&headers, &app_state) {
        return Redirect::to("/").into_response();
    }

    let server_exists = sqlx::query("SELECT id FROM servers WHERE id = ?")
        .bind(id)
        .fetch_optional(&app_state.db)
        .await
        .ok()
        .flatten()
        .is_some();

    if !server_exists {
        return StatusCode::NOT_FOUND.into_response();
    }

    Html(include_str!("../html/index.html")).into_response()
}

async fn register_page() -> Html<&'static str> {
    Html(include_str!("../html/register.html"))
}

async fn delete_page() -> Html<&'static str> {
    Html(include_str!("../html/delete.html"))
}

async fn delete_server_page() -> Html<&'static str> {
    Html(include_str!("../html/delete_server.html"))
}

async fn register_new_page(
     axum::extract::Query(params): axum::extract::Query<HashMap<String, String>>,
    State(app_state): State<Arc<AppState>>,
) -> Response {
    if let (Some(path), Some(content)) = (params.get("path"), params.get("content")) {
        let mut routes = app_state.dynamic_routes.write().unwrap();
        routes.insert(path.clone(), content.clone());
        StatusCode::CREATED.into_response()
    } else {
        Redirect::to("/").into_response()
    }
}

async fn ws_handler(
    ws: WebSocketUpgrade,
    Query(query): Query<WebSocketQuery>,
    headers: HeaderMap,
    State(app_state): State<Arc<AppState>>,
) -> Response {
    let Some(username) = session_username(&headers, &app_state) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    
    let server_exists = sqlx::query("SELECT id FROM servers WHERE id = ?")
        .bind(query.server_id)
        .fetch_optional(&app_state.db)
        .await
        .ok()
        .flatten()
        .is_some();

    if !server_exists {
        return StatusCode::NOT_FOUND.into_response();
    }

    let room_tx = {
        let mut rooms = app_state.room_channels.write().unwrap();

        rooms
            .entry(query.server_id)
            .or_insert_with(|| {
                let (tx, _rx) = broadcast::channel(16);
                tx
            })
            .clone()
    };

    ws.on_upgrade(move |socket| handle_socket(socket, room_tx, username))
}

fn generate_chat_room(name: &str) {

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

async fn handle_socket(socket: WebSocket, room_tx: broadcast::Sender<String>, username: String) {
    let (mut sender, mut receiver) = socket.split();
    let mut rx = room_tx.subscribe();

    let room_for_broadcast = room_tx.clone();
    let mut send_task = tokio::spawn(async move {
        while let Some(Ok(msg)) = receiver.next().await {
            if let Message::Text(text) = msg {
                let message = format!("{username}: {text}");
                let _ = room_for_broadcast.send(message);
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
