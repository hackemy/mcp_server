use std::collections::HashSet;
use std::sync::Arc;

use axum::{
    body::Body,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde_json::json;
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::server::Server;
use crate::types::JsonRpcRequest;

/// Shared state for the HTTP handler.
pub(crate) struct HttpState {
    server: Server,
    sessions: RwLock<HashSet<String>>,
}

/// Create an Axum router for the MCP server.
pub fn http_router(server: Server) -> Router {
    let state = Arc::new(HttpState {
        server,
        sessions: RwLock::new(HashSet::new()),
    });

    Router::new()
        .route("/mcp", post(handle_mcp))
        .route("/healthz", get(handle_healthz))
        .with_state(state)
}

async fn handle_healthz() -> impl IntoResponse {
    Json(json!({"status": "ok"}))
}

async fn handle_mcp(
    State(state): State<Arc<HttpState>>,
    headers: HeaderMap,
    Json(req): Json<JsonRpcRequest>,
) -> Response {
    // Session management: create on initialize, validate after.
    let session_id = if req.method == "initialize" {
        let id = Uuid::new_v4().to_string();
        state.sessions.write().await.insert(id.clone());
        Some(id)
    } else {
        // Check for existing session.
        if let Some(hdr) = headers.get("mcp-session-id") {
            let id = hdr.to_str().unwrap_or_default().to_string();
            let sessions = state.sessions.read().await;
            if !sessions.contains(&id) {
                // Unknown session — still allow for stateless usage.
            }
            Some(id)
        } else {
            None
        }
    };

    let resp = state.server.handle(req).await;

    // Notification: return 202 with no body.
    if resp.is_notification() {
        return (StatusCode::ACCEPTED, Body::empty()).into_response();
    }

    let mut response = Json(&resp).into_response();

    // Attach session ID header.
    if let Some(sid) = session_id {
        response.headers_mut().insert(
            "mcp-session-id",
            sid.parse().unwrap(),
        );
    }

    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::Server;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    fn test_router() -> Router {
        let srv = Server::builder()
            .tools_json(
                r#"[{"name":"echo","description":"test","inputSchema":{"type":"object","properties":{}}}]"#.as_bytes(),
            )
            .resources_json(r#"[]"#.as_bytes())
            .server_info("test", "0.1")
            .build();
        http_router(srv)
    }

    fn json_body(body: serde_json::Value) -> Body {
        Body::from(serde_json::to_vec(&body).unwrap())
    }

    #[tokio::test]
    async fn test_health_check() {
        let app = test_router();
        let req = Request::builder()
            .method("GET")
            .uri("/healthz")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_initialize_returns_session_id() {
        let app = test_router();
        let body = json!({
            "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": {"protocolVersion": "2025-03-26", "capabilities": {}, "clientInfo": {"name": "test", "version": "0.1"}}
        });
        let req = Request::builder()
            .method("POST")
            .uri("/mcp")
            .header("content-type", "application/json")
            .body(json_body(body))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert!(resp.headers().contains_key("mcp-session-id"));
    }

    #[tokio::test]
    async fn test_notification_returns_202() {
        let app = test_router();
        let body = json!({"jsonrpc": "2.0", "method": "notifications/initialized"});
        let req = Request::builder()
            .method("POST")
            .uri("/mcp")
            .header("content-type", "application/json")
            .body(json_body(body))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::ACCEPTED);
    }

    #[tokio::test]
    async fn test_tools_list() {
        let app = test_router();
        let body = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"});
        let req = Request::builder()
            .method("POST")
            .uri("/mcp")
            .header("content-type", "application/json")
            .body(json_body(body))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_invalid_json() {
        let app = test_router();
        let req = Request::builder()
            .method("POST")
            .uri("/mcp")
            .header("content-type", "application/json")
            .body(Body::from("{bad json"))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        // Axum returns 422 for malformed JSON by default
        assert!(resp.status().is_client_error());
    }

    #[tokio::test]
    async fn test_method_not_allowed() {
        let app = test_router();
        let req = Request::builder()
            .method("GET")
            .uri("/mcp")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::METHOD_NOT_ALLOWED);
    }
}
