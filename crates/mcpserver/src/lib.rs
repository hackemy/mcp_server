//! `mcpserver` — A reusable MCP (Model Context Protocol) server library.
//!
//! Implements the MCP 2025-03-26 specification with Streamable HTTP transport.
//! Configure with tools/resources JSON, register handlers, and serve via Axum.

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
