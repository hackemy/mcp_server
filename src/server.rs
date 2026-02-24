use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};
use tracing;

use crate::loader;
use crate::types::*;

/// Handler trait for MCP tools. Implement this or use closures.
#[async_trait]
pub trait ToolHandler: Send + Sync {
    async fn call(&self, args: Value) -> Result<ToolResult, McpError>;
}

/// Handler trait for MCP resources.
#[async_trait]
pub trait ResourceHandler: Send + Sync {
    async fn call(&self, uri: &str) -> Result<ResourceContent, McpError>;
}

/// Wraps an async closure into a ToolHandler.
pub struct FnToolHandler<F> {
    f: F,
}

impl<F, Fut> FnToolHandler<F>
where
    F: Fn(Value) -> Fut + Send + Sync + 'static,
    Fut: std::future::Future<Output = Result<ToolResult, McpError>> + Send + 'static,
{
    pub fn new(f: F) -> Arc<dyn ToolHandler> {
        Arc::new(Self { f })
    }
}

#[async_trait]
impl<F, Fut> ToolHandler for FnToolHandler<F>
where
    F: Fn(Value) -> Fut + Send + Sync + 'static,
    Fut: std::future::Future<Output = Result<ToolResult, McpError>> + Send + 'static,
{
    async fn call(&self, args: Value) -> Result<ToolResult, McpError> {
        (self.f)(args).await
    }
}

/// The MCP server. Create with `ServerBuilder`, register handlers, then serve.
pub struct Server {
    pub(crate) server_name: String,
    pub(crate) server_version: String,
    pub(crate) tools: HashMap<String, Tool>,
    pub(crate) tool_list: Vec<Tool>,
    pub(crate) resources: HashMap<String, Resource>,
    pub(crate) resource_list: Vec<Resource>,
    pub(crate) tool_handlers: HashMap<String, Arc<dyn ToolHandler>>,
    pub(crate) resource_handlers: HashMap<String, Arc<dyn ResourceHandler>>,
}

impl Server {
    /// Create a new server builder.
    pub fn builder() -> ServerBuilder {
        ServerBuilder::default()
    }

    /// Register a tool handler.
    pub fn handle_tool(&mut self, name: impl Into<String>, handler: Arc<dyn ToolHandler>) {
        self.tool_handlers.insert(name.into(), handler);
    }

    /// Register a resource handler.
    pub fn handle_resource(&mut self, name: impl Into<String>, handler: Arc<dyn ResourceHandler>) {
        self.resource_handlers.insert(name.into(), handler);
    }

    /// Route a JSON-RPC request to the appropriate MCP handler.
    pub async fn handle(&self, req: JsonRpcRequest) -> JsonRpcResponse {
        if req.jsonrpc != "2.0" {
            return new_error_response(req.id, ERR_CODE_INVALID_REQ, "jsonrpc must be '2.0'");
        }

        match req.method.as_str() {
            "initialize" => self.handle_initialize(req),
            "ping" => self.handle_ping(req),
            "notifications/initialized" | "notifications/cancelled" => notification_response(),
            "tools/list" => self.handle_tools_list(req),
            "tools/call" => self.handle_tools_call(req).await,
            "resources/list" => self.handle_resources_list(req),
            "resources/read" => self.handle_resources_read(req).await,
            _ => new_error_response(
                req.id,
                ERR_CODE_NO_METHOD,
                format!("Method not found: {}", req.method),
            ),
        }
    }

    fn handle_initialize(&self, req: JsonRpcRequest) -> JsonRpcResponse {
        if let Some(params) = &req.params {
            if let Ok(p) = serde_json::from_value::<InitializeParams>(params.clone()) {
                let client_name = p.client_info.as_ref().map_or("", |c| c.name.as_str());
                let client_version = p.client_info.as_ref().map_or("", |c| c.version.as_str());
                tracing::info!(
                    client_name,
                    client_version,
                    protocol_version = ?p.protocol_version,
                    "initialize"
                );
            }
        }

        let result = json!({
            "protocolVersion": PROTOCOL_VERSION,
            "capabilities": {
                "tools": {"listChanged": false},
                "resources": {"subscribe": false, "listChanged": false},
            },
            "serverInfo": {
                "name": self.server_name,
                "version": self.server_version,
            },
        });

        new_ok_response(req.id, result)
    }

    fn handle_ping(&self, req: JsonRpcRequest) -> JsonRpcResponse {
        new_ok_response(req.id, json!({}))
    }

    fn handle_tools_list(&self, req: JsonRpcRequest) -> JsonRpcResponse {
        let result = json!({ "tools": self.tool_list });
        new_ok_response(req.id, result)
    }

    async fn handle_tools_call(&self, req: JsonRpcRequest) -> JsonRpcResponse {
        let params: ToolCallParams = match req.params.as_ref() {
            Some(p) => match serde_json::from_value(p.clone()) {
                Ok(p) => p,
                Err(e) => {
                    return new_error_response(
                        req.id,
                        ERR_CODE_BAD_PARAMS,
                        format!("invalid params: {}", e),
                    )
                }
            },
            None => {
                return new_error_response(req.id, ERR_CODE_BAD_PARAMS, "params required");
            }
        };

        let args = if params.arguments.is_null() {
            json!({})
        } else {
            params.arguments
        };

        // Find tool definition.
        let tool = match self.tools.get(&params.name) {
            Some(t) => t,
            None => {
                return new_error_response(
                    req.id,
                    ERR_CODE_NO_METHOD,
                    format!("Unknown tool: {}", params.name),
                )
            }
        };

        // Validate arguments.
        if let Err(e) = tool.validate_arguments(&args) {
            return new_error_response(req.id, ERR_CODE_BAD_PARAMS, e);
        }

        // Find handler.
        let handler = match self.tool_handlers.get(&params.name) {
            Some(h) => h,
            None => {
                return new_error_response(
                    req.id,
                    ERR_CODE_INTERNAL,
                    format!("no handler for tool: {}", params.name),
                )
            }
        };

        // Execute handler.
        let result = match handler.call(args).await {
            Ok(r) => r,
            Err(e) => error_result(e.to_string()),
        };

        let result_value = serde_json::to_value(&result).unwrap_or(json!(null));
        new_ok_response(req.id, result_value)
    }

    fn handle_resources_list(&self, req: JsonRpcRequest) -> JsonRpcResponse {
        let result = json!({ "resources": self.resource_list });
        new_ok_response(req.id, result)
    }

    async fn handle_resources_read(&self, req: JsonRpcRequest) -> JsonRpcResponse {
        let params: ResourceReadParams = match req.params.as_ref() {
            Some(p) => match serde_json::from_value(p.clone()) {
                Ok(p) => p,
                Err(e) => {
                    return new_error_response(
                        req.id,
                        ERR_CODE_BAD_PARAMS,
                        format!("invalid params: {}", e),
                    )
                }
            },
            None => {
                return new_error_response(req.id, ERR_CODE_BAD_PARAMS, "params required");
            }
        };

        if params.name.is_none() && params.uri.is_none() {
            return new_error_response(
                req.id,
                ERR_CODE_BAD_PARAMS,
                "either name or uri must be provided",
            );
        }

        // Resolve resource.
        let target = if let Some(name) = &params.name {
            self.resources.get(name).cloned()
        } else {
            let uri = params.uri.as_deref().unwrap_or_default();
            self.resource_list.iter().find(|r| r.uri == uri).cloned()
        };

        let target = match target {
            Some(t) => t,
            None => {
                return new_error_response(req.id, ERR_CODE_BAD_PARAMS, "resource not found")
            }
        };

        // Check for registered handler.
        if let Some(handler) = self.resource_handlers.get(&target.name) {
            match handler.call(&target.uri).await {
                Ok(content) => {
                    let result = json!({ "contents": [content] });
                    new_ok_response(req.id, result)
                }
                Err(e) => new_error_response(
                    req.id,
                    ERR_CODE_INTERNAL,
                    format!("read resource: {}", e),
                ),
            }
        } else {
            // Fallback: return metadata only.
            let result = json!({
                "contents": [{
                    "uri": target.uri,
                    "mimeType": target.mime_type,
                    "text": "",
                }],
            });
            new_ok_response(req.id, result)
        }
    }
}

/// Builder for constructing an MCP Server.
#[derive(Default)]
pub struct ServerBuilder {
    tools: Vec<Tool>,
    resources: Vec<Resource>,
    server_name: Option<String>,
    server_version: Option<String>,
}

impl ServerBuilder {
    /// Load tool definitions from a JSON file.
    pub fn tools_file(mut self, path: impl AsRef<std::path::Path>) -> Self {
        match loader::load_tools(path) {
            Ok(tools) => self.tools.extend(tools),
            Err(e) => tracing::error!("load tools file: {}", e),
        }
        self
    }

    /// Add tool definitions directly.
    pub fn tools(mut self, tools: Vec<Tool>) -> Self {
        self.tools.extend(tools);
        self
    }

    /// Parse tool definitions from raw JSON bytes.
    pub fn tools_json(mut self, data: &[u8]) -> Self {
        match loader::parse_tools(data) {
            Ok(tools) => self.tools.extend(tools),
            Err(e) => tracing::error!("parse tools json: {}", e),
        }
        self
    }

    /// Load resource definitions from a JSON file.
    pub fn resources_file(mut self, path: impl AsRef<std::path::Path>) -> Self {
        match loader::load_resources(path) {
            Ok(resources) => self.resources.extend(resources),
            Err(e) => tracing::error!("load resources file: {}", e),
        }
        self
    }

    /// Add resource definitions directly.
    pub fn resources(mut self, resources: Vec<Resource>) -> Self {
        self.resources.extend(resources);
        self
    }

    /// Parse resource definitions from raw JSON bytes.
    pub fn resources_json(mut self, data: &[u8]) -> Self {
        match loader::parse_resources(data) {
            Ok(resources) => self.resources.extend(resources),
            Err(e) => tracing::error!("parse resources json: {}", e),
        }
        self
    }

    /// Set server name and version.
    pub fn server_info(mut self, name: impl Into<String>, version: impl Into<String>) -> Self {
        self.server_name = Some(name.into());
        self.server_version = Some(version.into());
        self
    }

    /// Build the server.
    pub fn build(self) -> Server {
        let tool_map: HashMap<String, Tool> =
            self.tools.iter().map(|t| (t.name.clone(), t.clone())).collect();
        let res_map: HashMap<String, Resource> =
            self.resources.iter().map(|r| (r.name.clone(), r.clone())).collect();

        Server {
            server_name: self.server_name.unwrap_or_else(|| "mcpserver".into()),
            server_version: self.server_version.unwrap_or_else(|| "1.0.0".into()),
            tools: tool_map,
            tool_list: self.tools,
            resources: res_map,
            resource_list: self.resources,
            tool_handlers: HashMap::new(),
            resource_handlers: HashMap::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct EchoHandler;

    #[async_trait]
    impl ToolHandler for EchoHandler {
        async fn call(&self, args: Value) -> Result<ToolResult, McpError> {
            let msg = args.get("msg").and_then(|v| v.as_str()).unwrap_or("no msg");
            Ok(text_result(format!("echo: {}", msg)))
        }
    }

    fn test_server() -> Server {
        let tools_json = r#"[
            {"name":"echo","description":"echoes","inputSchema":{"type":"object","properties":{"msg":{"type":"string"}},"required":["msg"]}}
        ]"#;
        let resources_json = r#"[
            {"name":"test","description":"test resource","uri":"file:///test.csv","mimeType":"text/csv"}
        ]"#;

        let mut srv = Server::builder()
            .tools_json(tools_json.as_bytes())
            .resources_json(resources_json.as_bytes())
            .server_info("test-server", "0.1.0")
            .build();

        srv.handle_tool("echo", Arc::new(EchoHandler));
        srv
    }

    fn make_req(method: &str, id: Option<Value>, params: Option<Value>) -> JsonRpcRequest {
        JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id,
            method: method.into(),
            params,
        }
    }

    #[tokio::test]
    async fn test_bad_jsonrpc_version() {
        let srv = test_server();
        let req = JsonRpcRequest {
            jsonrpc: "1.0".into(),
            id: Some(json!(1)),
            method: "ping".into(),
            params: None,
        };
        let resp = srv.handle(req).await;
        assert!(resp.error.is_some());
        assert_eq!(resp.error.unwrap().code, ERR_CODE_INVALID_REQ);
    }

    #[tokio::test]
    async fn test_unknown_method() {
        let srv = test_server();
        let resp = srv.handle(make_req("unknown/method", Some(json!(1)), None)).await;
        assert!(resp.error.is_some());
        assert_eq!(resp.error.unwrap().code, ERR_CODE_NO_METHOD);
    }

    #[tokio::test]
    async fn test_initialize() {
        let srv = test_server();
        let params = json!({
            "protocolVersion": "2025-03-26",
            "capabilities": {},
            "clientInfo": {"name": "test", "version": "0.1"}
        });
        let resp = srv.handle(make_req("initialize", Some(json!(1)), Some(params))).await;
        assert!(resp.error.is_none());
        let result = resp.result.unwrap();
        assert_eq!(result["protocolVersion"], PROTOCOL_VERSION);
        assert_eq!(result["serverInfo"]["name"], "test-server");
    }

    #[tokio::test]
    async fn test_ping() {
        let srv = test_server();
        let resp = srv.handle(make_req("ping", Some(json!(1)), None)).await;
        assert!(resp.error.is_none());
        assert_eq!(resp.result.unwrap(), json!({}));
    }

    #[tokio::test]
    async fn test_notifications_return_sentinel() {
        let srv = test_server();
        let resp = srv
            .handle(make_req("notifications/initialized", None, None))
            .await;
        assert!(resp.is_notification());
    }

    #[tokio::test]
    async fn test_tools_list() {
        let srv = test_server();
        let resp = srv.handle(make_req("tools/list", Some(json!(1)), None)).await;
        assert!(resp.error.is_none());
        let result = resp.result.unwrap();
        let tools = result["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0]["name"], "echo");
    }

    #[tokio::test]
    async fn test_tools_call_success() {
        let srv = test_server();
        let params = json!({"name": "echo", "arguments": {"msg": "hello"}});
        let resp = srv.handle(make_req("tools/call", Some(json!(1)), Some(params))).await;
        assert!(resp.error.is_none());
        let result: ToolResult = serde_json::from_value(resp.result.unwrap()).unwrap();
        assert_eq!(result.content[0].text.as_deref(), Some("echo: hello"));
        assert!(!result.is_error);
    }

    #[tokio::test]
    async fn test_tools_call_missing_required() {
        let srv = test_server();
        let params = json!({"name": "echo", "arguments": {}});
        let resp = srv.handle(make_req("tools/call", Some(json!(1)), Some(params))).await;
        assert!(resp.error.is_some());
        assert_eq!(resp.error.unwrap().code, ERR_CODE_BAD_PARAMS);
    }

    #[tokio::test]
    async fn test_tools_call_unknown_tool() {
        let srv = test_server();
        let params = json!({"name": "nonexistent", "arguments": {}});
        let resp = srv.handle(make_req("tools/call", Some(json!(1)), Some(params))).await;
        assert!(resp.error.is_some());
        assert_eq!(resp.error.unwrap().code, ERR_CODE_NO_METHOD);
    }

    #[tokio::test]
    async fn test_tools_call_no_handler() {
        let srv = Server::builder()
            .tools_json(
                r#"[{"name":"no-handler","description":"test","inputSchema":{"type":"object","properties":{}}}]"#.as_bytes(),
            )
            .build();
        let params = json!({"name": "no-handler", "arguments": {}});
        let resp = srv.handle(make_req("tools/call", Some(json!(1)), Some(params))).await;
        assert!(resp.error.is_some());
        assert_eq!(resp.error.unwrap().code, ERR_CODE_INTERNAL);
    }

    #[tokio::test]
    async fn test_resources_list() {
        let srv = test_server();
        let resp = srv.handle(make_req("resources/list", Some(json!(1)), None)).await;
        assert!(resp.error.is_none());
        let result = resp.result.unwrap();
        let resources = result["resources"].as_array().unwrap();
        assert_eq!(resources.len(), 1);
        assert_eq!(resources[0]["name"], "test");
    }

    #[tokio::test]
    async fn test_resources_read_by_name() {
        let srv = test_server();
        let params = json!({"name": "test"});
        let resp = srv.handle(make_req("resources/read", Some(json!(1)), Some(params))).await;
        assert!(resp.error.is_none());
        let result = resp.result.unwrap();
        let contents = result["contents"].as_array().unwrap();
        assert_eq!(contents[0]["uri"], "file:///test.csv");
    }

    #[tokio::test]
    async fn test_resources_read_by_uri() {
        let srv = test_server();
        let params = json!({"uri": "file:///test.csv"});
        let resp = srv.handle(make_req("resources/read", Some(json!(1)), Some(params))).await;
        assert!(resp.error.is_none());
    }

    #[tokio::test]
    async fn test_resources_read_not_found() {
        let srv = test_server();
        let params = json!({"name": "nonexistent"});
        let resp = srv.handle(make_req("resources/read", Some(json!(1)), Some(params))).await;
        assert!(resp.error.is_some());
    }

    #[tokio::test]
    async fn test_resources_read_missing_params() {
        let srv = test_server();
        let params = json!({});
        let resp = srv.handle(make_req("resources/read", Some(json!(1)), Some(params))).await;
        assert!(resp.error.is_some());
    }
}
