# mcpserver

A Rust library for building [Model Context Protocol](https://modelcontextprotocol.io/) (MCP) servers, implementing the **2025-03-26** specification with Streamable HTTP transport.

Define your tools and resources in JSON, register async handlers, and serve over HTTP with Axum — or call `Server::handle()` directly for Lambda / custom integrations.

## Installation

```toml
[dependencies]
mcpserver = "0.1"
```

Or via the CLI:

```bash
cargo add mcpserver
```

## Quick start

```rust
use std::sync::Arc;
use mcpserver::{Server, FnToolHandler, http_router, text_result};
use serde_json::Value;

#[tokio::main]
async fn main() {
    let mut server = Server::builder()
        .tools_file("tools.json")
        .resources_file("resources.json")
        .server_info("my-server", "0.1.0")
        .build();

    server.handle_tool("echo", FnToolHandler::new(|args: Value| async move {
        let msg = args.get("message").and_then(|v| v.as_str()).unwrap_or("");
        Ok(text_result(msg))
    }));

    let app = http_router(server);
    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
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

## HTTP transport

`http_router()` returns an `axum::Router` with a single route: `POST /mcp` (the MCP JSON-RPC endpoint). Session management via `mcp-session-id` headers is handled automatically.

Merge it into your own router to add health checks, landing pages, or middleware:

```rust
use axum::{routing::get, Json, Router};
use mcpserver::{Server, http_router};

let server = Server::builder().build();
let app = Router::new()
    .route("/healthz", get(|| async { Json(serde_json::json!({"status": "ok"})) }))
    .merge(http_router(server));
```

## Custom integration (Lambda, etc.)

For environments where you control the HTTP layer (e.g., AWS Lambda), use `Server::handle()` directly:

```rust
use mcpserver::{Server, JsonRpcRequest};

async fn my_lambda_handler(server: &Server, body: &str) -> String {
    let req: JsonRpcRequest = serde_json::from_str(body).unwrap();
    let resp = server.handle(req).await;
    serde_json::to_string(&resp).unwrap()
}
```

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
| `initialize` | Handshake, returns server capabilities and session ID |
| `ping` | Keepalive |
| `tools/list` | List available tools |
| `tools/call` | Execute a tool |
| `resources/list` | List available resources |
| `resources/read` | Read a resource by name or URI |
| `notifications/initialized` | Client notification (HTTP 202) |
| `notifications/cancelled` | Client notification (HTTP 202) |

## License

MIT — see [LICENSE](LICENSE).
