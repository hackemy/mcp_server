# Architecture

This document captures every architectural decision needed to reproduce `mcpserver` from scratch. It is written for both humans and LLMs.

## Core principle

**The library is a pure MCP protocol handler.** It has zero HTTP, transport, or runtime opinion. The single entry point is:

```rust
pub async fn handle(&self, req: JsonRpcRequest) -> McpResponse
```

The application owns: listening on a port, routing, middleware (auth, rate limiting), HTTP status codes, session management, and TLS. The library owns: JSON-RPC 2.0 parsing, MCP method routing, schema validation, handler dispatch, and response construction.

This means the library's dependency footprint is minimal — `serde`, `serde_json`, `async-trait`, `tracing`, `thiserror`. No `axum`, `tokio`, `hyper`, or any HTTP crate.

## Module layout

```
src/
  lib.rs          — Module declarations and public re-exports
  types.rs        — All type definitions, McpResponse, serialization
  server.rs       — Server struct, builder, handler traits, MCP routing
  loader.rs       — JSON file/bytes → Vec<Tool> / Vec<Resource>
  validate.rs     — Tool::validate_arguments() against SchemaMeta
```

### `lib.rs`

Only module declarations (`pub mod ...` / `mod ...`) and `pub use` re-exports. The `validate` module is `mod validate` (not `pub mod`) because `Tool::validate_arguments()` is a public method on a public type — no need to expose the module itself.

Re-exports are the public API surface:

```rust
pub use server::{FnToolHandler, ResourceHandler, Server, ServerBuilder, ToolHandler};
pub use types::{
    error_result, new_error_response, text_result, ContentBlock, JsonRpcRequest, JsonRpcResponse,
    McpError, McpResponse, Resource, ResourceContent, RpcError, Tool, ToolResult, PROTOCOL_VERSION,
};
pub use loader::{load_resources, load_tools, parse_resources, parse_tools};
```

### `types.rs`

All type definitions live here. Key decisions:

- **`JsonRpcRequest`** — `Deserialize + Serialize`. The `id` field is `Option<Value>` because notifications have no id. The `params` field is `Option<Value>` — kept as a raw Value tree and destructured later by the specific handler.

- **`McpResponse`** — The optimized response type returned by `handle()`. Private fields, `pub(crate)` constructors. See [Zero-copy response design](#zero-copy-response-design) below.

- **`JsonRpcResponse`** — Legacy structured response kept for two purposes: (1) deserialization from external JSON-RPC, (2) test inspection via `McpResponse::into_json_rpc()`. Not returned by `handle()`.

- **`Tool`** has `#[serde(skip)]` on `schema_meta` — validation metadata is internal, never serialized to clients. The `#[serde(rename_all = "camelCase")]` on `Tool` means `input_schema` serializes as `inputSchema`, matching the MCP spec.

- **`ToolCallParams`** and **`ResourceReadParams`** are `pub(crate)` — internal deserialize-only structs consumed via `serde_json::from_value()` in the handler methods.

- **`SchemaMeta`** stores parsed validation rules (`required`, `one_of`, `dependencies`) extracted from JSON Schema at load time. This avoids re-parsing the schema on every request.

### `server.rs`

The `Server` struct, builder pattern, handler traits, and all MCP method routing.

**Handler traits:**

```rust
#[async_trait]
pub trait ToolHandler: Send + Sync {
    async fn call(&self, args: Value, context: Value) -> Result<ToolResult, McpError>;
}

#[async_trait]
pub trait ResourceHandler: Send + Sync {
    async fn call(&self, uri: &str, context: Value) -> Result<ResourceContent, McpError>;
}
```

Both take `&self` — handlers are shared via `Arc<dyn ToolHandler>` in the HashMap. Both receive a `context: Value` that carries request-scoped data from the HTTP layer (decoded JWT claims, tenant info, etc.). See [Request context](#request-context) below.

**`FnToolHandler::new()` returns `Arc<dyn ToolHandler>` directly** — not `Self`. This means closure-based handlers can be registered without the caller wrapping in `Arc`:

```rust
server.handle_tool("echo", FnToolHandler::new(|args: Value, _ctx: Value| async move { ... }));
```

**`Server::handle()` takes `JsonRpcRequest` and `context: Value` by value.** This is critical — it destructures `req.id` and `req.params` by move into sub-handlers, and moves `context` to the one handler that runs. No cloning.

**Method routing** is a match on `req.method.as_str()`:

| Method | Handler | Response type | Context |
|---|---|---|---|
| `initialize` | `handle_initialize` | `Cached(Arc<RawValue>)` | dropped |
| `ping` | inline | `Result(json!({}))` | dropped |
| `notifications/*` | inline | `Notification` | dropped |
| `tools/list` | `handle_tools_list` | `Cached(Arc<RawValue>)` | dropped |
| `tools/call` | `handle_tools_call` | `Result(Value)` | moved to handler |
| `resources/list` | `handle_resources_list` | `Cached(Arc<RawValue>)` | dropped |
| `resources/read` | `handle_resources_read` | `Result(Value)` | moved to handler |

### `loader.rs`

**`parse_tools`** deserializes into `Vec<Value>` first, then manually extracts `name`, `description`, `inputSchema` fields and calls `parse_schema_meta()`. This two-step approach is intentional — we need the raw `inputSchema` Value (for serialization back to clients) AND the parsed `SchemaMeta` (for validation).

**`parse_resources`** directly deserializes into `Vec<Resource>` via serde — resources have no schema metadata to extract.

**`parse_schema_meta`** extracts three features from JSON Schema:
- `required` — array of field names
- `oneOf` — array of `{required: [...]}` objects
- `dependencies` — map of field → array of dependent fields

### `validate.rs`

Implements `Tool::validate_arguments(&self, args: &Value) -> Result<(), String>`. Validation checks in order:

1. **`required`** — every listed field must be present in args
2. **`oneOf`** — at least one requirement set must be fully satisfied
3. **`dependencies`** — if field A is present, all its dependents must also be present

Returns `Err(String)` with a human-readable message on failure.

## Zero-copy response design

This is the most important architectural decision. The goal: **cached MCP endpoints (`initialize`, `tools/list`, `resources/list`) must serve responses with zero data copying per request.**

### The problem

These endpoints always return the same JSON. Naively, each request would:
1. Clone the `Value` tree (recursive deep copy)
2. Serialize the clone to JSON bytes

For a server handling thousands of requests, this is wasteful.

### The solution: `Arc<RawValue>`

At build time, `ServerBuilder::build()` pre-serializes each cached result into `Box<RawValue>` (a validated JSON string), then wraps it in `Arc`:

```rust
fn to_raw(value: &Value) -> Box<RawValue> {
    RawValue::from_string(serde_json::to_string(value).unwrap()).unwrap()
}

let tools_list_result: Arc<RawValue> = Arc::from(to_raw(&json!({ "tools": self.tools })));
```

Per request, the handler does `Arc::clone` (atomic ref-count increment, no data copy) and wraps it in `McpResponse::Cached`:

```rust
fn handle_tools_list(&self, id: Option<Value>) -> McpResponse {
    McpResponse::cached(id, &self.tools_list_result)
}
```

### Custom `Serialize` for `McpResponse`

`McpResponse` has a hand-written `Serialize` impl using `serialize_map`. For the `Cached` variant, it embeds the `RawValue` verbatim — the JSON bytes are copied directly to the output buffer without parsing or tree-walking:

```rust
ResponseKind::Cached(raw) => map.serialize_entry("result", raw.as_ref())?,
```

This requires `serde_json` with the `raw_value` feature enabled in `Cargo.toml`:

```toml
serde_json = { version = "1", features = ["raw_value"] }
```

### `ResponseKind` enum

```rust
enum ResponseKind {
    Cached(Arc<RawValue>),  // Pre-serialized, zero-copy
    Result(Value),           // Dynamic (tools/call, resources/read)
    Error(RpcError),         // Error response
    Notification,            // No body (HTTP 202)
}
```

- `Cached` — for endpoints whose response never changes (`initialize`, `tools/list`, `resources/list`)
- `Result` — for dynamic endpoints (`tools/call`, `resources/read`) where the Value is constructed per-request
- `Error` — JSON-RPC error
- `Notification` — sentinel; the HTTP layer returns 202 with empty body

### `into_json_rpc()` for test inspection

Since `McpResponse` fields are private and `Cached` holds raw bytes, tests use `into_json_rpc()` to convert back to `JsonRpcResponse`. This re-parses the `RawValue` into a `Value` tree — acceptable in tests, never in production.

## Build-time serialization order

In `ServerBuilder::build()`, the order of operations matters:

1. **Pre-serialize** `tools_list_result` and `resources_list_result` from `self.tools` / `self.resources` (borrows the Vecs)
2. **Then** consume the Vecs via `into_iter()` to build HashMaps (moves the structs)

This avoids cloning — the Vecs are borrowed for JSON serialization, then moved into maps. Only the `name` String is cloned (for the HashMap key); the `Tool`/`Resource` structs themselves are moved.

```rust
// Step 1: borrow for serialization
let tools_list_result = Arc::from(to_raw(&json!({ "tools": self.tools })));

// Step 2: consume by move
let tool_map: HashMap<String, Tool> = self.tools.into_iter()
    .map(|t| { let name = t.name.clone(); (name, t) })
    .collect();
```

## Move semantics in `handle()`

`handle()` takes `JsonRpcRequest` and `context` by value and destructures them:

```rust
pub async fn handle(&self, req: JsonRpcRequest, context: Value) -> McpResponse {
    match req.method.as_str() {
        "tools/call" => self.handle_tools_call(req.id, req.params, context).await,
        ...
    }
}
```

`req.id`, `req.params`, and `context` are moved into the sub-handler that runs. For cached endpoints (`initialize`, `tools/list`, `resources/list`), the context is simply dropped — it was never forwarded.

Inside `handle_tools_call`, the `params: Option<Value>` is consumed by `serde_json::from_value(p)` which takes ownership and destructures the Value tree without cloning. The `context` is moved directly to the handler's `call()` method.

The tool definition and handler are looked up by borrowing from the HashMaps (`self.tools.get()`, `self.tool_handlers.get()`). No struct cloning.

## Request context

The `context: Value` parameter on `handle()` carries request-scoped data from the HTTP layer to tool/resource handlers. The library is completely agnostic about its contents — it simply moves the value through.

**The flow:**

1. HTTP middleware decodes the JWT (or extracts API key metadata, etc.)
2. The Axum handler builds a `Value` with the claims: `json!({"sub": "user-123", "tenant_id": "acme"})`
3. Passes it to `server.handle(req, context)`
4. `handle()` moves it to `handle_tools_call()` or `handle_resources_read()`
5. The handler method moves it to `handler.call(args, context)`
6. The tool/resource handler reads whatever fields it needs

**Zero clones.** The `Value` is constructed once in the HTTP layer and moved at every step. For cached endpoints it's dropped without ever being read.

**No auth opinion.** The library doesn't know about JWTs, Cognito, Auth0, or any specific provider. It just passes a `Value` through. The HTTP layer decides what goes in it.

**Handlers that don't need context** simply ignore it:

```rust
FnToolHandler::new(|args: Value, _context: Value| async move {
    Ok(text_result("done"))
})
```

## Handler registration

Handlers are registered after `build()`:

```rust
let mut server = Server::builder().tools_file("tools.json").build();
server.handle_tool("echo", Arc::new(EchoHandler));
server.handle_tool("greet", FnToolHandler::new(|args, _ctx| async move { ... }));
```

The `handle_tool` and `handle_resource` methods take `Arc<dyn ToolHandler>` / `Arc<dyn ResourceHandler>`. `FnToolHandler::new()` returns `Arc<dyn ToolHandler>` already.

Struct-based handlers must be wrapped in `Arc::new()` by the caller. Closure-based handlers get this for free from `FnToolHandler::new()`.

## Tool and resource definitions

Tools and resources are defined as JSON arrays, not in Rust code. This is intentional — definitions change frequently and JSON is easier to edit than Rust structs. The builder supports three sources:

- `tools_file("path.json")` — load from disk
- `tools_json(bytes)` — parse from raw bytes (useful for embedded JSON or tests)
- `tools(vec)` — pass pre-built `Vec<Tool>` directly

Same for resources.

## Schema validation

Validation happens at the protocol layer, before the handler is called. If arguments fail validation, the handler never executes and a JSON-RPC error is returned.

Three schema features are supported:
- **`required`** — field must exist in the arguments object
- **`oneOf`** — at least one set of required fields must all be present
- **`dependencies`** — if field A is present, fields B, C, ... must also be present

These cover the common patterns needed for MCP tools. Full JSON Schema validation (type checking, patterns, ranges) is intentionally not implemented — it would add complexity without proportional value for typical MCP use cases.

## Error handling

- **Validation errors** → JSON-RPC error with code `-32602` (bad params)
- **Unknown tool** → JSON-RPC error with code `-32601` (method not found)
- **No handler registered** → JSON-RPC error with code `-32603` (internal error)
- **Handler returns `Err(McpError)`** → converted to `error_result()` (tool result with `is_error: true`), not a JSON-RPC error. This matches MCP spec — tool execution errors are content, not protocol errors.

## The HTTP layer (application concern)

The `examples/basic_server.rs` is the reference integration. Key patterns:

- **Session management**: Create UUID on `initialize`, store in `RwLock<HashSet<String>>`, pass via `mcp-session-id` header. Entirely application-level.
- **Notification → 202**: Check `resp.is_notification()`, return `StatusCode::ACCEPTED` with empty body.
- **Normal response → JSON**: `Json(&resp)` — the custom `Serialize` impl handles cached vs dynamic transparently.
- **Multiple endpoints**: Mount the same handler on different routes with different middleware for public/private access.

## Protocol version

The library implements MCP specification **2025-03-26**. The version string is defined as:

```rust
pub const PROTOCOL_VERSION: &str = "2025-03-26";
```

This is returned in the `initialize` response and should be updated when implementing a newer spec version.

## Supported MCP methods

| Method | Type | Notes |
|---|---|---|
| `initialize` | Cached | Returns capabilities, server info, protocol version |
| `ping` | Dynamic | Returns `{}` |
| `tools/list` | Cached | Returns all registered tool definitions |
| `tools/call` | Dynamic | Validates args, dispatches to handler |
| `resources/list` | Cached | Returns all registered resource definitions |
| `resources/read` | Dynamic | Looks up by name or URI, dispatches to handler |
| `notifications/initialized` | Notification | No response body (HTTP 202) |
| `notifications/cancelled` | Notification | No response body (HTTP 202) |

## Dependencies rationale

| Crate | Why |
|---|---|
| `serde` + `serde_json` | JSON serialization. `raw_value` feature required for `RawValue` / zero-copy. |
| `async-trait` | Handler traits need `async fn` in traits. Can be removed once async trait fns stabilize in Rust. |
| `tracing` | Structured logging in `handle_initialize`. No subscriber — that's the app's job. |
| `thiserror` | Derive `Error` for `McpError` enum. |

Everything else (`axum`, `tokio`, `uuid`, etc.) is in `[dev-dependencies]` for tests and examples only.
