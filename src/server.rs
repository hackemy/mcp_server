use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::value::RawValue;
use serde_json::{json, Value};
use tracing;

use crate::loader;
use crate::types::*;

/// Handler trait for MCP tools. Implement this or use closures.
///
/// The `context` parameter carries request-scoped data from the HTTP layer
/// (e.g. decoded JWT claims).  It is moved into the handler — zero clones.
#[async_trait]
pub trait ToolHandler: Send + Sync {
    async fn call(&self, args: Value, context: Value) -> Result<ToolResult, McpError>;
}

/// Handler trait for MCP resources.
///
/// The `context` parameter carries request-scoped data from the HTTP layer.
#[async_trait]
pub trait ResourceHandler: Send + Sync {
    async fn call(&self, uri: &str, context: Value) -> Result<ResourceContent, McpError>;
}

/// Wraps an async closure into a ToolHandler.
pub struct FnToolHandler<F> {
    f: F,
}

impl<F, Fut> FnToolHandler<F>
where
    F: Fn(Value, Value) -> Fut + Send + Sync + 'static,
    Fut: std::future::Future<Output = Result<ToolResult, McpError>> + Send + 'static,
{
    pub fn new(f: F) -> Arc<dyn ToolHandler> {
        Arc::new(Self { f })
    }
}

#[async_trait]
impl<F, Fut> ToolHandler for FnToolHandler<F>
where
    F: Fn(Value, Value) -> Fut + Send + Sync + 'static,
    Fut: std::future::Future<Output = Result<ToolResult, McpError>> + Send + 'static,
{
    async fn call(&self, args: Value, context: Value) -> Result<ToolResult, McpError> {
        (self.f)(args, context).await
    }
}

/// The MCP server. Create with `ServerBuilder`, register handlers, then serve.
pub struct Server {
    pub(crate) tools: HashMap<String, Tool>,
    pub(crate) resources: HashMap<String, Resource>,
    pub(crate) tool_handlers: HashMap<String, Arc<dyn ToolHandler>>,
    pub(crate) resource_handlers: HashMap<String, Arc<dyn ResourceHandler>>,
    /// Pre-serialized initialize result — shared by reference, never copied.
    initialize_result: Arc<RawValue>,
    /// Pre-serialized tools/list result.
    tools_list_result: Arc<RawValue>,
    /// Pre-serialized resources/list result.
    resources_list_result: Arc<RawValue>,
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
    ///
    /// Takes ownership of the request and context, moving fields into
    /// sub-handlers without cloning.  For cached endpoints the response holds
    /// an `Arc` reference to pre-serialized JSON — zero data copying.
    ///
    /// The `context` carries request-scoped data from the HTTP layer (e.g.
    /// decoded JWT claims).  It is moved to the tool/resource handler that
    /// runs — no cloning.  For cached endpoints it is simply dropped.
    /// Pass `Value::Null` or `json!({})` when there is no context.
    pub async fn handle(&self, req: JsonRpcRequest, context: Value) -> McpResponse {
        if req.jsonrpc != "2.0" {
            return McpResponse::error(req.id, ERR_CODE_INVALID_REQ, "jsonrpc must be '2.0'");
        }

        match req.method.as_str() {
            "initialize" => self.handle_initialize(req.id, req.params),
            "ping" => McpResponse::ok(req.id, json!({})),
            "notifications/initialized" | "notifications/cancelled" => McpResponse::notification(),
            "tools/list" => self.handle_tools_list(req.id),
            "tools/call" => self.handle_tools_call(req.id, req.params, context).await,
            "resources/list" => self.handle_resources_list(req.id),
            "resources/read" => self.handle_resources_read(req.id, req.params, context).await,
            _ => McpResponse::error(
                req.id,
                ERR_CODE_NO_METHOD,
                format!("Method not found: {}", req.method),
            ),
        }
    }

    fn handle_initialize(&self, id: Option<Value>, params: Option<Value>) -> McpResponse {
        // Log client info by borrowing directly into the params Value — no
        // deserialization, no clone.
        if let Some(ref params) = params {
            let client_name = params
                .pointer("/clientInfo/name")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let client_version = params
                .pointer("/clientInfo/version")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let protocol_version = params
                .get("protocolVersion")
                .and_then(|v| v.as_str());
            tracing::info!(
                client_name,
                client_version,
                protocol_version,
                "initialize"
            );
        }

        McpResponse::cached(id, &self.initialize_result)
    }

    fn handle_tools_list(&self, id: Option<Value>) -> McpResponse {
        McpResponse::cached(id, &self.tools_list_result)
    }

    async fn handle_tools_call(
        &self,
        id: Option<Value>,
        params: Option<Value>,
        context: Value,
    ) -> McpResponse {
        // Consume the params Value directly — no clone.
        let params: ToolCallParams = match params {
            Some(p) => match serde_json::from_value(p) {
                Ok(p) => p,
                Err(e) => {
                    return McpResponse::error(
                        id,
                        ERR_CODE_BAD_PARAMS,
                        format!("invalid params: {}", e),
                    )
                }
            },
            None => {
                return McpResponse::error(id, ERR_CODE_BAD_PARAMS, "params required");
            }
        };

        let args = if params.arguments.is_null() {
            json!({})
        } else {
            params.arguments
        };

        // Find tool definition (borrow, no clone).
        let tool = match self.tools.get(&params.name) {
            Some(t) => t,
            None => {
                return McpResponse::error(
                    id,
                    ERR_CODE_NO_METHOD,
                    format!("Unknown tool: {}", params.name),
                )
            }
        };

        // Validate arguments.
        if let Err(e) = tool.validate_arguments(&args) {
            return McpResponse::error(id, ERR_CODE_BAD_PARAMS, e);
        }

        // Find handler (borrow, no clone).
        let handler = match self.tool_handlers.get(&params.name) {
            Some(h) => h,
            None => {
                return McpResponse::error(
                    id,
                    ERR_CODE_INTERNAL,
                    format!("no handler for tool: {}", params.name),
                )
            }
        };

        // Execute handler and convert result to Value.
        let result = match handler.call(args, context).await {
            Ok(r) => r,
            Err(e) => error_result(e.to_string()),
        };

        let result_value = serde_json::to_value(&result).unwrap_or(json!(null));
        McpResponse::ok(id, result_value)
    }

    fn handle_resources_list(&self, id: Option<Value>) -> McpResponse {
        McpResponse::cached(id, &self.resources_list_result)
    }

    async fn handle_resources_read(
        &self,
        id: Option<Value>,
        params: Option<Value>,
        context: Value,
    ) -> McpResponse {
        // Consume the params Value directly — no clone.
        let params: ResourceReadParams = match params {
            Some(p) => match serde_json::from_value(p) {
                Ok(p) => p,
                Err(e) => {
                    return McpResponse::error(
                        id,
                        ERR_CODE_BAD_PARAMS,
                        format!("invalid params: {}", e),
                    )
                }
            },
            None => {
                return McpResponse::error(id, ERR_CODE_BAD_PARAMS, "params required");
            }
        };

        if params.name.is_none() && params.uri.is_none() {
            return McpResponse::error(
                id,
                ERR_CODE_BAD_PARAMS,
                "either name or uri must be provided",
            );
        }

        // Resolve resource by borrowing — no clone of the Resource struct.
        let target: Option<&Resource> = if let Some(name) = &params.name {
            self.resources.get(name)
        } else {
            let uri = params.uri.as_deref().unwrap_or_default();
            self.resources.values().find(|r| r.uri == uri)
        };

        let target = match target {
            Some(t) => t,
            None => {
                return McpResponse::error(id, ERR_CODE_BAD_PARAMS, "resource not found")
            }
        };

        // Check for registered handler.
        if let Some(handler) = self.resource_handlers.get(&target.name) {
            match handler.call(&target.uri, context).await {
                Ok(content) => {
                    let result = json!({ "contents": [content] });
                    McpResponse::ok(id, result)
                }
                Err(e) => McpResponse::error(
                    id,
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
            McpResponse::ok(id, result)
        }
    }
}

/// Serialize a Value to a pre-validated `Box<RawValue>`.
fn to_raw(value: &Value) -> Box<RawValue> {
    RawValue::from_string(serde_json::to_string(value).unwrap()).unwrap()
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
        let server_name = self.server_name.unwrap_or_else(|| "mcpserver".into());
        let server_version = self.server_version.unwrap_or_else(|| "1.0.0".into());

        // Pre-serialize cached results once into RawValue (shared via Arc).
        let initialize_result: Arc<RawValue> = Arc::from(to_raw(&json!({
            "protocolVersion": PROTOCOL_VERSION,
            "capabilities": {
                "tools": {"listChanged": false},
                "resources": {"subscribe": false, "listChanged": false},
            },
            "serverInfo": {
                "name": server_name,
                "version": server_version,
            },
        })));

        let tools_list_result: Arc<RawValue> =
            Arc::from(to_raw(&json!({ "tools": self.tools })));

        let resources_list_result: Arc<RawValue> =
            Arc::from(to_raw(&json!({ "resources": self.resources })));

        // Move tools and resources into HashMaps — only the key String is
        // cloned, the structs themselves are moved.
        let tool_map: HashMap<String, Tool> = self
            .tools
            .into_iter()
            .map(|t| {
                let name = t.name.clone();
                (name, t)
            })
            .collect();
        let res_map: HashMap<String, Resource> = self
            .resources
            .into_iter()
            .map(|r| {
                let name = r.name.clone();
                (name, r)
            })
            .collect();

        Server {
            tools: tool_map,
            resources: res_map,
            tool_handlers: HashMap::new(),
            resource_handlers: HashMap::new(),
            initialize_result,
            tools_list_result,
            resources_list_result,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct EchoHandler;

    #[async_trait]
    impl ToolHandler for EchoHandler {
        async fn call(&self, args: Value, _context: Value) -> Result<ToolResult, McpError> {
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
        let resp = srv.handle(req, json!({})).await.into_json_rpc();
        assert!(resp.error.is_some());
        assert_eq!(resp.error.unwrap().code, ERR_CODE_INVALID_REQ);
    }

    #[tokio::test]
    async fn test_unknown_method() {
        let srv = test_server();
        let resp = srv.handle(make_req("unknown/method", Some(json!(1)), None), json!({})).await.into_json_rpc();
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
        let resp = srv.handle(make_req("initialize", Some(json!(1)), Some(params)), json!({})).await.into_json_rpc();
        assert!(resp.error.is_none());
        let result = resp.result.unwrap();
        assert_eq!(result["protocolVersion"], PROTOCOL_VERSION);
        assert_eq!(result["serverInfo"]["name"], "test-server");
    }

    #[tokio::test]
    async fn test_ping() {
        let srv = test_server();
        let resp = srv.handle(make_req("ping", Some(json!(1)), None), json!({})).await.into_json_rpc();
        assert!(resp.error.is_none());
        assert_eq!(resp.result.unwrap(), json!({}));
    }

    #[tokio::test]
    async fn test_notifications_return_sentinel() {
        let srv = test_server();
        let resp = srv
            .handle(make_req("notifications/initialized", None, None), json!({}))
            .await;
        assert!(resp.is_notification());
    }

    #[tokio::test]
    async fn test_tools_list() {
        let srv = test_server();
        let resp = srv.handle(make_req("tools/list", Some(json!(1)), None), json!({})).await.into_json_rpc();
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
        let resp = srv.handle(make_req("tools/call", Some(json!(1)), Some(params)), json!({})).await.into_json_rpc();
        assert!(resp.error.is_none());
        let result: ToolResult = serde_json::from_value(resp.result.unwrap()).unwrap();
        assert_eq!(result.content[0].text.as_deref(), Some("echo: hello"));
        assert!(!result.is_error);
    }

    #[tokio::test]
    async fn test_tools_call_missing_required() {
        let srv = test_server();
        let params = json!({"name": "echo", "arguments": {}});
        let resp = srv.handle(make_req("tools/call", Some(json!(1)), Some(params)), json!({})).await.into_json_rpc();
        assert!(resp.error.is_some());
        assert_eq!(resp.error.unwrap().code, ERR_CODE_BAD_PARAMS);
    }

    #[tokio::test]
    async fn test_tools_call_unknown_tool() {
        let srv = test_server();
        let params = json!({"name": "nonexistent", "arguments": {}});
        let resp = srv.handle(make_req("tools/call", Some(json!(1)), Some(params)), json!({})).await.into_json_rpc();
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
        let resp = srv.handle(make_req("tools/call", Some(json!(1)), Some(params)), json!({})).await.into_json_rpc();
        assert!(resp.error.is_some());
        assert_eq!(resp.error.unwrap().code, ERR_CODE_INTERNAL);
    }

    #[tokio::test]
    async fn test_resources_list() {
        let srv = test_server();
        let resp = srv.handle(make_req("resources/list", Some(json!(1)), None), json!({})).await.into_json_rpc();
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
        let resp = srv.handle(make_req("resources/read", Some(json!(1)), Some(params)), json!({})).await.into_json_rpc();
        assert!(resp.error.is_none());
        let result = resp.result.unwrap();
        let contents = result["contents"].as_array().unwrap();
        assert_eq!(contents[0]["uri"], "file:///test.csv");
    }

    #[tokio::test]
    async fn test_resources_read_by_uri() {
        let srv = test_server();
        let params = json!({"uri": "file:///test.csv"});
        let resp = srv.handle(make_req("resources/read", Some(json!(1)), Some(params)), json!({})).await.into_json_rpc();
        assert!(resp.error.is_none());
    }

    #[tokio::test]
    async fn test_resources_read_not_found() {
        let srv = test_server();
        let params = json!({"name": "nonexistent"});
        let resp = srv.handle(make_req("resources/read", Some(json!(1)), Some(params)), json!({})).await.into_json_rpc();
        assert!(resp.error.is_some());
    }

    #[tokio::test]
    async fn test_resources_read_missing_params() {
        let srv = test_server();
        let params = json!({});
        let resp = srv.handle(make_req("resources/read", Some(json!(1)), Some(params)), json!({})).await.into_json_rpc();
        assert!(resp.error.is_some());
    }

    /// Verify that serializing an McpResponse produces valid JSON-RPC.
    #[tokio::test]
    async fn test_serialize_cached_response() {
        let srv = test_server();
        let resp = srv.handle(make_req("tools/list", Some(json!(1)), None), json!({})).await;
        let json_str = serde_json::to_string(&resp).unwrap();
        let parsed: JsonRpcResponse = serde_json::from_str(&json_str).unwrap();
        assert_eq!(parsed.jsonrpc, "2.0");
        assert_eq!(parsed.id, Some(json!(1)));
        let tools = parsed.result.unwrap()["tools"].as_array().unwrap().len();
        assert_eq!(tools, 1);
    }
}
