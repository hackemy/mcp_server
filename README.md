# mcpserver

[![crates.io](https://img.shields.io/crates/v/mcpserver.svg)](https://crates.io/crates/mcpserver)
[![docs.rs](https://docs.rs/mcpserver/badge.svg)](https://docs.rs/mcpserver)

A Rust library for building [Model Context Protocol](https://modelcontextprotocol.io/) (MCP) servers, implementing the **2025-03-26** specification.

`mcpserver` is a **pure protocol handler** — it parses JSON-RPC, routes MCP methods, validates tool arguments, and dispatches to your handlers. It has zero HTTP or transport opinion: you bring your own framework (Axum, Lambda, Warp, etc.) and own the routing, middleware, and status codes.

## Installation

```toml
[dependencies]
mcpserver = "0.2"
serde_json = "1"
```

The library has no runtime or HTTP dependencies. Add `axum`, `tokio`, etc. only if your application needs them.

## Quick start

```rust
use mcpserver::{Server, FnToolHandler, text_result, JsonRpcRequest};
use serde_json::Value;

// Build the server and register handlers.
let mut server = Server::builder()
    .tools_file("tools.json")
    .resources_file("resources.json")
    .server_info("my-server", "0.1.0")
    .build();

server.handle_tool("echo", FnToolHandler::new(|args: Value| async move {
    let msg = args.get("message").and_then(|v| v.as_str()).unwrap_or("");
    Ok(text_result(msg))
}));

// Deserialize from any source, call handle(), serialize the response.
let req: JsonRpcRequest = serde_json::from_str(body).unwrap();
let resp = server.handle(req).await;

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
    async fn call(&self, args: Value) -> Result<ToolResult, McpError> {
        Ok(text_result("done"))
    }
}
```

### Closure-based handler

```rust
use mcpserver::{FnToolHandler, text_result};
use serde_json::Value;

let handler = FnToolHandler::new(|args: Value| async move {
    Ok(text_result("done"))
});
```

### Resource handler

```rust
use async_trait::async_trait;
use mcpserver::{ResourceHandler, ResourceContent, McpError};

struct ConfigReader;

#[async_trait]
impl ResourceHandler for ConfigReader {
    async fn call(&self, uri: &str) -> Result<ResourceContent, McpError> {
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

async fn handle_mcp(
    State(server): State<Arc<Server>>,
    Json(req): Json<JsonRpcRequest>,
) -> impl IntoResponse {
    let resp = server.handle(req).await;
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

## Running the example

```bash
cargo run --example basic_server
```

Then in another terminal:

```bash
# Initialize
curl -X POST http://localhost:3000/mcp \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}'

# List tools
curl -X POST http://localhost:3000/mcp \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":2,"method":"tools/list"}'

# Call a tool
curl -X POST http://localhost:3000/mcp \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"echo","arguments":{"message":"hello"}}}'
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
