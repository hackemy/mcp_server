//! `mcpserver` — A reusable MCP (Model Context Protocol) server library.
//!
//! Implements the MCP 2025-03-26 specification with Streamable HTTP transport.
//! Configure with tools/resources JSON, register handlers, and serve via Axum.
//!
//! # Quick start
//!
//! ```rust,no_run
//! use std::sync::Arc;
//! use mcpserver::{Server, FnToolHandler, http_router, text_result};
//! use serde_json::Value;
//!
//! #[tokio::main]
//! async fn main() {
//!     let mut server = Server::builder()
//!         .tools_file("tools.json")
//!         .resources_file("resources.json")
//!         .server_info("my-server", "0.1.0")
//!         .build();
//!
//!     server.handle_tool("echo", FnToolHandler::new(|args: Value| async move {
//!         let msg = args.get("message").and_then(|v| v.as_str()).unwrap_or("");
//!         Ok(text_result(msg))
//!     }));
//!
//!     let app = http_router(server);
//!     let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
//!     axum::serve(listener, app).await.unwrap();
//! }
//! ```

pub mod loader;
pub mod server;
pub mod transport_http;
pub mod types;
mod validate;

// Re-export the most commonly used items at the crate root.
pub use loader::{load_resources, load_tools, parse_resources, parse_tools};
pub use server::{FnToolHandler, ResourceHandler, Server, ServerBuilder, ToolHandler};
pub use transport_http::http_router;
pub use types::{
    error_result, new_error_response, text_result, ContentBlock, JsonRpcRequest, JsonRpcResponse,
    McpError, Resource, ResourceContent, RpcError, Tool, ToolResult, PROTOCOL_VERSION,
};
