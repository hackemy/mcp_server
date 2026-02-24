# mcpserver

[![crates.io](https://img.shields.io/crates/v/mcpserver.svg)](https://crates.io/crates/mcpserver)
[![docs.rs](https://docs.rs/mcpserver/badge.svg)](https://docs.rs/mcpserver)

A Rust library for building [Model Context Protocol](https://modelcontextprotocol.io/) (MCP) servers, implementing the **2025-03-26** specification.

`mcpserver` is a **pure protocol handler** — it parses JSON-RPC, routes MCP methods, validates tool arguments, and dispatches to your handlers. It has zero HTTP or transport opinion: you bring your own framework (Axum, Lambda, Warp, etc.) and own the routing, middleware, and status codes.

## Installation

```toml
[dependencies]
mcpserver = "0.3"
serde_json = "1"
```

The library has no runtime or HTTP dependencies. Add `axum`, `tokio`, etc. only if your application needs them.

## Quick start

```rust
use mcpserver::{Server, FnToolHandler, text_result, JsonRpcRequest};
use serde_json::{json, Value};

// Build the server and register handlers.
let mut server = Server::builder()
    .tools_file("tools.json")
    .resources_file("resources.json")
    .server_info("my-server", "0.1.0")
    .build();

server.handle_tool("echo", FnToolHandler::new(|args: Value, _context: Value| async move {
    let msg = args.get("message").and_then(|v| v.as_str()).unwrap_or("");
    Ok(text_result(msg))
}));

// Deserialize from any source, call handle(), serialize the response.
// The second argument is request context (e.g. decoded JWT claims).
let req: JsonRpcRequest = serde_json::from_str(body).unwrap();
let resp = server.handle(req, json!({})).await;

// resp.is_notification() → true for fire-and-forget methods (return 202, no body)
let json = serde_json::to_string(&resp).unwrap();
```

## Defining tools (`tools.json`)

Tools are defined as a JSON array. Each tool has a `name`, `description`, and an `inputSchema` (JSON Schema) that drives automatic argument validation.

```json
[
  {
    "name": "echo",
    "description": "Echoes the input message",
    "inputSchema": {
      "type": "object",
      "properties": {
        "message": { "type": "string" }
      },
      "required": ["message"]
    }
  }
]
```

### Supported schema features

| Feature | Description | Example |
|---|---|---|
| `required` | Fields that must be present | `"required": ["name"]` |
| `oneOf` | At least one set of required fields must match | `"oneOf": [{"required": ["phone"]}, {"required": ["email"]}]` |
| `dependencies` | If field A is present, field B must also be present | `"dependencies": {"lat": ["lon"]}` |

See [`examples/tools.json`](examples/tools.json) for a full example with all three features.

## Defining resources (`resources.json`)

```json
[
  {
    "name": "config",
    "description": "Application configuration",
    "uri": "file:///etc/app/config.json",
    "mimeType": "application/json"
  }
]
```

## Handler patterns

### Struct-based handler

```rust
use async_trait::async_trait;
use mcpserver::{ToolHandler, ToolResult, McpError, text_result};
use serde_json::Value;

struct MyHandler;

#[async_trait]
impl ToolHandler for MyHandler {
    async fn call(&self, args: Value, context: Value) -> Result<ToolResult, McpError> {
        let user_id = context.get("user_id").and_then(|v| v.as_str()).unwrap_or("anonymous");
        Ok(text_result(format!("done by {}", user_id)))
    }
}
```

### Closure-based handler

```rust
use mcpserver::{FnToolHandler, text_result};
use serde_json::Value;

let handler = FnToolHandler::new(|args: Value, _context: Value| async move {
    Ok(text_result("done"))
});
```

### Resource handler

```rust
use async_trait::async_trait;
use mcpserver::{ResourceHandler, ResourceContent, McpError};
use serde_json::Value;

struct ConfigReader;

#[async_trait]
impl ResourceHandler for ConfigReader {
    async fn call(&self, uri: &str, _context: Value) -> Result<ResourceContent, McpError> {
        Ok(ResourceContent {
            uri: uri.to_string(),
            mime_type: Some("application/json".into()),
            text: Some(r#"{"key": "value"}"#.into()),
            blob: None,
        })
    }
}
```

## HTTP integration (Axum example)

Since the library is transport-agnostic, you wire up HTTP yourself. Here's the pattern with Axum:

```rust
use std::sync::Arc;
use axum::{extract::State, http::StatusCode, response::IntoResponse, Json, Router, routing::post, body::Body};
use mcpserver::{Server, JsonRpcRequest};
use serde_json::json;

async fn handle_mcp(
    State(server): State<Arc<Server>>,
    Json(req): Json<JsonRpcRequest>,
) -> impl IntoResponse {
    // Build context from your auth layer (JWT claims, API key metadata, etc.)
    let context = json!({});
    let resp = server.handle(req, context).await;
    if resp.is_notification() {
        return (StatusCode::ACCEPTED, Body::empty()).into_response();
    }
    Json(&resp).into_response()
}

let server = Arc::new(Server::builder().build());
let app = Router::new()
    .route("/mcp", post(handle_mcp))
    .with_state(server);
```

This makes it trivial to mount multiple MCP endpoints with different middleware:

```rust
let app = Router::new()
    .route("/mcp_public", post(handle_mcp))
    .route("/mcp_private", post(handle_mcp).layer(auth_middleware))
    .with_state(server);
```

See [`examples/basic_server.rs`](examples/basic_server.rs) for a complete working example with session management, health checks, and tool handlers.

## Running the demo

```bash
cargo run --example basic_server
```

The demo starts on `http://localhost:3000` with these endpoints:

| Endpoint | Description |
|---|---|
| `POST /mcp` | MCP JSON-RPC endpoint |
| `GET /healthz` | Health check |

### Basic usage (no auth)

```bash
# Initialize a session
curl -s -X POST http://localhost:3000/mcp \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}' | jq .

# List available tools
curl -s -X POST http://localhost:3000/mcp \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":2,"method":"tools/list"}' | jq .

# Call a tool
curl -s -X POST http://localhost:3000/mcp \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"echo","arguments":{"message":"hello"}}}' | jq .

# List resources
curl -s -X POST http://localhost:3000/mcp \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":4,"method":"resources/list"}' | jq .

# Read a resource
curl -s -X POST http://localhost:3000/mcp \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":5,"method":"resources/read","params":{"name":"config"}}' | jq .
```

### With session tracking

The demo returns an `mcp-session-id` header on `initialize`. Pass it back on subsequent requests:

```bash
# Initialize and capture the session ID
SESSION=$(curl -s -D- -X POST http://localhost:3000/mcp \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}' \
  | grep -i mcp-session-id | tr -d '\r' | awk '{print $2}')

echo "Session: $SESSION"

# Use the session ID for subsequent calls
curl -s -X POST http://localhost:3000/mcp \
  -H "Content-Type: application/json" \
  -H "mcp-session-id: $SESSION" \
  -d '{"jsonrpc":"2.0","id":2,"method":"tools/list"}' | jq .
```

### With JWT authentication and identity context

Since `mcpserver` is transport-agnostic, you add auth at the HTTP layer. The decoded JWT claims are passed as `context` to `Server::handle()`, making them available to every tool and resource handler.

```rust
use axum::{extract::Request, middleware::{self, Next}, response::Response, http::StatusCode, Extension};
use serde_json::{json, Value};

async fn require_jwt(mut req: Request, next: Next) -> Result<Response, StatusCode> {
    let auth = req.headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if !auth.starts_with("Bearer ") {
        return Err(StatusCode::UNAUTHORIZED);
    }

    let token = &auth[7..];
    // Decode the JWT (use jsonwebtoken, jwt-simple, etc.)
    let claims = decode_jwt(token).map_err(|_| StatusCode::UNAUTHORIZED)?;

    // Store decoded claims as an extension for the handler to read.
    // The shape depends on your provider (Cognito, Auth0, custom, etc.)
    req.extensions_mut().insert(claims);

    Ok(next.run(req).await)
}

async fn handle_private_mcp(
    State(state): State<Arc<AppState>>,
    Extension(claims): Extension<Value>,  // decoded JWT from middleware
    Json(req): Json<JsonRpcRequest>,
) -> Response {
    // Pass the decoded JWT claims as context — handlers read user_id, tenant_id, etc.
    let resp = state.server.handle(req, claims).await;
    if resp.is_notification() {
        return (StatusCode::ACCEPTED, Body::empty()).into_response();
    }
    Json(&resp).into_response()
}

let app = Router::new()
    // Public — no auth, empty context
    .route("/mcp", post(handle_mcp))
    // Protected — JWT claims flow into handler context
    .route("/mcp_private", post(handle_private_mcp)
        .layer(middleware::from_fn(require_jwt)))
    .with_state(state);
```

Inside any tool handler, read claims from context:

```rust
#[async_trait]
impl ToolHandler for MyHandler {
    async fn call(&self, args: Value, context: Value) -> Result<ToolResult, McpError> {
        let user_id = context.get("sub").and_then(|v| v.as_str()).unwrap_or("anonymous");
        let tenant_id = context.get("custom:tenant_id").and_then(|v| v.as_str());
        // ... use identity to scope queries, check permissions, etc.
        Ok(text_result("done"))
    }
}
```

Then call the protected endpoint with a Bearer token:

```bash
TOKEN="eyJhbGciOiJIUzI1NiIs..."

curl -s -X POST http://localhost:3000/mcp_private \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $TOKEN" \
  -d '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}' | jq .

# Without a token → 401 Unauthorized
curl -s -o /dev/null -w "%{http_code}" -X POST http://localhost:3000/mcp_private \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/list"}'
# 401
```

## Nginx deployment

An example Nginx config for TLS termination is provided in [`nginx/mcp.conf`](nginx/mcp.conf). Key settings:

- `proxy_buffering off` — required for streaming
- `proxy_http_version 1.1` — keep-alive to upstream
- `proxy_read_timeout 300s` — long timeout for streaming

## MCP methods supported

| Method | Description |
|---|---|
| `initialize` | Handshake, returns server capabilities |
| `ping` | Keepalive |
| `tools/list` | List available tools |
| `tools/call` | Execute a tool |
| `resources/list` | List available resources |
| `resources/read` | Read a resource by name or URI |
| `notifications/initialized` | Client notification (no response body) |
| `notifications/cancelled` | Client notification (no response body) |

## License

MIT — see [LICENSE](LICENSE).
