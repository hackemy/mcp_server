//! Basic MCP server example.
//!
//! Run with: `cargo run --example basic_server`
//! Then test with:
//!   curl -X POST http://localhost:3000/mcp \
//!     -H "Content-Type: application/json" \
//!     -d '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}'

use std::sync::Arc;

use async_trait::async_trait;
use mcpserver::{
    http_router, text_result, FnToolHandler, McpError, ResourceContent, ResourceHandler, Server,
    ToolHandler, ToolResult,
};
use serde_json::Value;

/// A struct-based tool handler for the "echo" tool.
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

/// A struct-based resource handler for the "config" resource.
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

    // Build the server from JSON definition files.
    let mut server = Server::builder()
        .tools_file("examples/tools.json")
        .resources_file("examples/resources.json")
        .server_info("example-server", "0.1.0")
        .build();

    // Register a struct-based handler.
    server.handle_tool("echo", Arc::new(EchoHandler));

    // Register a closure-based handler using FnToolHandler.
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

    // Register a closure-based handler for geocode (stub).
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

    // Register the resource handler.
    server.handle_resource("config", Arc::new(ConfigHandler));

    // Build the Axum router and serve.
    let app = http_router(server);
    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
    println!("MCP server listening on http://localhost:3000");
    println!("  POST /mcp  — MCP JSON-RPC endpoint");
    println!("  GET  /healthz — health check");
    axum::serve(listener, app).await.unwrap();
}
