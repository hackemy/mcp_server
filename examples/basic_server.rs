//! Basic MCP server example with Axum HTTP transport.
//!
//! This shows how to wire `Server::handle()` into an Axum app — the library
//! is a pure protocol handler, so *you* own the HTTP layer (routes, middleware,
//! status codes, session management).
//!
//! Run with: `cargo run --example basic_server`
//! Then test with:
//!   curl -X POST http://localhost:3000/mcp \
//!     -H "Content-Type: application/json" \
//!     -d '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}'

use std::collections::HashSet;
use std::sync::Arc;

use async_trait::async_trait;
use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use mcpserver::{
    text_result, FnToolHandler, JsonRpcRequest, McpError, McpResponse, ResourceContent,
    ResourceHandler, Server, ToolHandler, ToolResult,
};
use serde_json::{json, Value};
use tokio::sync::RwLock;
use uuid::Uuid;

// ── Shared state for the HTTP layer ──

struct AppState {
    server: Server,
    sessions: RwLock<HashSet<String>>,
}

// ── Axum handler: JSON-RPC → Server::handle() → HTTP response ──

async fn handle_mcp(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<JsonRpcRequest>,
) -> Response {
    // Session management: create on initialize, pass through otherwise.
    let session_id = if req.method == "initialize" {
        let id = Uuid::new_v4().to_string();
        state.sessions.write().await.insert(id.clone());
        Some(id)
    } else {
        headers
            .get("mcp-session-id")
            .and_then(|h| h.to_str().ok())
            .map(|s| s.to_string())
    };

    // The library handles all MCP protocol logic.
    // McpResponse holds Arc references to pre-serialized JSON for cached
    // endpoints — zero data copying.
    let resp: McpResponse = state.server.handle(req).await;

    // Notifications get 202 with no body.
    if resp.is_notification() {
        return (StatusCode::ACCEPTED, Body::empty()).into_response();
    }

    // McpResponse implements Serialize — cached results are embedded verbatim.
    let mut response = Json(&resp).into_response();

    if let Some(sid) = session_id {
        response
            .headers_mut()
            .insert("mcp-session-id", sid.parse().unwrap());
    }

    response
}

// ── Tool & resource handlers ──

struct EchoHandler;

#[async_trait]
impl ToolHandler for EchoHandler {
    async fn call(&self, args: Value) -> Result<ToolResult, McpError> {
        let message = args
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("(empty)");
        Ok(text_result(format!("echo: {}", message)))
    }
}

struct ConfigHandler;

#[async_trait]
impl ResourceHandler for ConfigHandler {
    async fn call(&self, uri: &str) -> Result<ResourceContent, McpError> {
        Ok(ResourceContent {
            uri: uri.to_string(),
            mime_type: Some("application/json".into()),
            text: Some(r#"{"debug": false, "version": "1.0"}"#.into()),
            blob: None,
        })
    }
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    // Build the MCP server (pure protocol handler — no HTTP awareness).
    let mut server = Server::builder()
        .tools_file("examples/tools.json")
        .resources_file("examples/resources.json")
        .server_info("example-server", "0.1.0")
        .build();

    server.handle_tool("echo", Arc::new(EchoHandler));

    server.handle_tool(
        "greet",
        FnToolHandler::new(|args: Value| async move {
            let name = args
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("world");
            let style = args
                .get("style")
                .and_then(|v| v.as_str())
                .unwrap_or("casual");
            let greeting = match style {
                "formal" => format!("Good day, {}.", name),
                _ => format!("Hey, {}!", name),
            };
            Ok(text_result(greeting))
        }),
    );

    server.handle_tool(
        "geocode",
        FnToolHandler::new(|args: Value| async move {
            if let Some(address) = args.get("address").and_then(|v| v.as_str()) {
                Ok(text_result(format!(
                    "Geocoded '{}': lat=40.7128, lon=-74.0060",
                    address
                )))
            } else {
                let lat = args.get("lat").and_then(|v| v.as_f64()).unwrap_or(0.0);
                let lon = args.get("lon").and_then(|v| v.as_f64()).unwrap_or(0.0);
                Ok(text_result(format!(
                    "Reverse geocode ({}, {}): 123 Main St",
                    lat, lon
                )))
            }
        }),
    );

    server.handle_resource("config", Arc::new(ConfigHandler));

    // Wire up the HTTP layer — you own the routes, middleware, and status codes.
    let state = Arc::new(AppState {
        server,
        sessions: RwLock::new(HashSet::new()),
    });

    let app = Router::new()
        .route("/healthz", get(|| async { Json(json!({"status": "ok"})) }))
        .route("/mcp", post(handle_mcp))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
    println!("MCP server listening on http://localhost:3000");
    println!("  POST /mcp     — MCP JSON-RPC endpoint");
    println!("  GET  /healthz — health check");
    axum::serve(listener, app).await.unwrap();
}
