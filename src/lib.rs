//! `mcpserver` — A Rust library for building MCP (Model Context Protocol) servers.
//!
//! Implements the MCP 2025-03-26 specification as a pure protocol handler.
//! Define tools and resources in JSON, register async handlers, and call
//! `Server::handle()` from any HTTP framework, Lambda function, or test harness.
//!
//! # Quick start
//!
//! ```rust
//! use mcpserver::{Server, FnToolHandler, text_result, JsonRpcRequest};
//! use serde_json::Value;
//!
//! # async fn example() {
//! let mut server = Server::builder()
//!     .tools_json(r#"[{"name":"echo","description":"echoes","inputSchema":{"type":"object","properties":{"message":{"type":"string"}},"required":["message"]}}]"#.as_bytes())
//!     .server_info("my-server", "0.1.0")
//!     .build();
//!
//! server.handle_tool("echo", FnToolHandler::new(|args: Value| async move {
//!     let msg = args.get("message").and_then(|v| v.as_str()).unwrap_or("");
//!     Ok(text_result(msg))
//! }));
//!
//! // Use from any HTTP framework — just deserialize the body and call handle():
//! let req: JsonRpcRequest = serde_json::from_str(r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#).unwrap();
//! let resp = server.handle(req).await;
//! // resp implements Serialize — pass it to axum::Json, serde_json, etc.
//! let json = serde_json::to_string(&resp).unwrap();
//! # }
//! ```

pub mod loader;
pub mod server;
pub mod types;
mod validate;

// Re-export the most commonly used items at the crate root.
pub use loader::{load_resources, load_tools, parse_resources, parse_tools};
pub use server::{FnToolHandler, ResourceHandler, Server, ServerBuilder, ToolHandler};
pub use types::{
    error_result, new_error_response, text_result, ContentBlock, JsonRpcRequest, JsonRpcResponse,
    McpError, McpResponse, Resource, ResourceContent, RpcError, Tool, ToolResult, PROTOCOL_VERSION,
};
