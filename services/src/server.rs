// server.rs — theOS Web UI Server
// Serves the HTML UI on localhost:8080
// Bridges WebSocket messages to the daemon

use axum::{
    Router,
    routing::get,
    extract::ws::{WebSocket, WebSocketUpgrade, Message},
    response::{Html, IntoResponse},
};
use std::sync::Arc;
use tokio::sync::broadcast;

pub type EventTx = broadcast::Sender<String>;

pub struct UiServer {
    pub event_tx: EventTx,
}

impl UiServer {
    pub fn new() -> Self {
        let (event_tx, _) = broadcast::channel(32);
        Self { event_tx }
    }

    pub async fn start(&self, port: u16) {
        let tx = self.event_tx.clone();
        let app = Router::new()
            .route("/", get(serve_ui))
            .route("/ws", get(move |ws: WebSocketUpgrade| {
                let tx = tx.clone();
                async move {
                    ws.on_upgrade(move |socket| handle_ws(socket, tx))
                }
            }));

        let addr = format!("0.0.0.0:{}", port);
        println!("theOS UI server running at http://localhost:{}", port);
        let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
        axum::serve(listener, app).await.unwrap();
    }
}

async fn serve_ui() -> impl IntoResponse {
    // In production: read from /usr/share/theos/ui/index.html
    // For now return embedded UI
    Html(include_str!("../../ui/index.html"))
}

async fn handle_ws(mut socket: WebSocket, tx: EventTx) {
    let mut rx = tx.subscribe();
    loop {
        tokio::select! {
            // Forward daemon events to UI
            Ok(event) = rx.recv() => {
                if socket.send(Message::Text(event.into())).await.is_err() {
                    break;
                }
            }
            // Handle UI commands
            Some(Ok(msg)) = socket.recv() => {
                if let Message::Text(text) = msg {
                    println!("UI command: {}", text);
                    handle_ui_command(&text, &tx).await;
                }
            }
        }
    }
}

async fn handle_ui_command(cmd: &str, tx: &EventTx) {
    // Parse JSON commands from the UI
    if cmd.contains("\"action\":\"call\"") {
        println!("UI requested call — triggering VoIP engine");
        let _ = tx.send(r#"{"event":"call_initiated","status":"dialing"}"#.to_string());
    } else if cmd.contains("\"action\":\"hangup\"") {
        println!("UI requested hangup");
        let _ = tx.send(r#"{"event":"call_ended"}"#.to_string());
    } else if cmd.contains("\"action\":\"status\"") {
        let _ = tx.send(r#"{"event":"status","link":"Starlink","latency":45,"quality":96}"#.to_string());
    }
}
