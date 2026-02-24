use std::sync::Arc;

use serde::ser::SerializeMap;
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use serde_json::Value;

/// JSON-RPC 2.0 error codes.
pub const ERR_CODE_PARSE: i32 = -32700;
pub const ERR_CODE_INVALID_REQ: i32 = -32600;
pub const ERR_CODE_NO_METHOD: i32 = -32601;
pub const ERR_CODE_BAD_PARAMS: i32 = -32602;
pub const ERR_CODE_INTERNAL: i32 = -32603;

/// MCP Protocol version this server implements.
pub const PROTOCOL_VERSION: &str = "2025-03-26";

// ── Request ──

/// Inbound JSON-RPC 2.0 request.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    #[serde(default)]
    pub id: Option<Value>,
    pub method: String,
    #[serde(default)]
    pub params: Option<Value>,
}

// ── Response ──

/// Response from [`Server::handle()`](crate::Server::handle).
///
/// For cached endpoints (`initialize`, `tools/list`, `resources/list`) the
/// result is pre-serialized JSON shared via `Arc` — per-request cost is a
/// single atomic ref-count increment, zero data copying.
///
/// Implements [`Serialize`] so you can pass it directly to your HTTP
/// framework (e.g. `axum::Json(&resp)`).  The pre-serialized payload is
/// embedded verbatim by the serializer.
///
/// For structured inspection (e.g. in tests), call
/// [`into_json_rpc()`](McpResponse::into_json_rpc).
#[derive(Debug)]
pub struct McpResponse {
    id: Option<Value>,
    kind: ResponseKind,
}

#[derive(Debug)]
enum ResponseKind {
    /// Pre-serialized result — `Arc::clone` is ref-count only, zero data copy.
    Cached(Arc<RawValue>),
    /// Dynamically constructed result.
    Result(Value),
    /// Error.
    Error(RpcError),
    /// Notification sentinel — no response body.
    Notification,
}

impl McpResponse {
    /// True when the request was a notification (no response body needed).
    pub fn is_notification(&self) -> bool {
        matches!(self.kind, ResponseKind::Notification)
    }

    /// Convert to a [`JsonRpcResponse`] for structured inspection.
    ///
    /// For cached results this parses the raw JSON back into a `Value`.
    /// In production code, prefer serializing `McpResponse` directly.
    pub fn into_json_rpc(self) -> JsonRpcResponse {
        match self.kind {
            ResponseKind::Cached(raw) => JsonRpcResponse {
                jsonrpc: "2.0".into(),
                id: self.id,
                result: Some(serde_json::from_str(raw.get()).unwrap()),
                error: None,
            },
            ResponseKind::Result(value) => JsonRpcResponse {
                jsonrpc: "2.0".into(),
                id: self.id,
                result: Some(value),
                error: None,
            },
            ResponseKind::Error(err) => JsonRpcResponse {
                jsonrpc: "2.0".into(),
                id: self.id,
                result: None,
                error: Some(err),
            },
            ResponseKind::Notification => JsonRpcResponse {
                jsonrpc: "2.0".into(),
                id: None,
                result: None,
                error: None,
            },
        }
    }

    // ── Internal constructors ──

    pub(crate) fn cached(id: Option<Value>, raw: &Arc<RawValue>) -> Self {
        McpResponse {
            id,
            kind: ResponseKind::Cached(Arc::clone(raw)),
        }
    }

    pub(crate) fn ok(id: Option<Value>, result: Value) -> Self {
        McpResponse {
            id,
            kind: ResponseKind::Result(result),
        }
    }

    pub(crate) fn error(id: Option<Value>, code: i32, message: impl Into<String>) -> Self {
        McpResponse {
            id,
            kind: ResponseKind::Error(RpcError {
                code,
                message: message.into(),
                data: None,
            }),
        }
    }

    pub(crate) fn notification() -> Self {
        McpResponse {
            id: None,
            kind: ResponseKind::Notification,
        }
    }
}

impl Serialize for McpResponse {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let has_id = self.id.is_some();
        let has_payload = !matches!(self.kind, ResponseKind::Notification);
        let len = 1 + has_id as usize + has_payload as usize;

        let mut map = serializer.serialize_map(Some(len))?;
        map.serialize_entry("jsonrpc", "2.0")?;

        if let Some(ref id) = self.id {
            map.serialize_entry("id", id)?;
        }

        match &self.kind {
            ResponseKind::Cached(raw) => map.serialize_entry("result", raw.as_ref())?,
            ResponseKind::Result(value) => map.serialize_entry("result", value)?,
            ResponseKind::Error(err) => map.serialize_entry("error", err)?,
            ResponseKind::Notification => {}
        }

        map.end()
    }
}

// ── Legacy structured response (kept for deserialization / test inspection) ──

/// Structured JSON-RPC 2.0 response.
///
/// Use [`McpResponse::into_json_rpc()`] to convert from the optimised
/// response type.  You can also construct this directly for custom
/// integrations via [`new_ok_response`] / [`new_error_response`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcError>,
}

impl JsonRpcResponse {
    /// Returns true when this is a notification sentinel (no body needed).
    pub fn is_notification(&self) -> bool {
        self.id.is_none() && self.result.is_none() && self.error.is_none()
    }
}

/// JSON-RPC 2.0 error object.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcError {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

// ── MCP domain types ──

/// MCP tool definition loaded from config.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Tool {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
    /// Parsed schema metadata for validation (not serialized to clients).
    #[serde(skip)]
    pub schema_meta: SchemaMeta,
}

/// MCP resource definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Resource {
    pub name: String,
    pub description: String,
    pub uri: String,
    pub mime_type: String,
}

/// Tool call result returned by handlers.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolResult {
    pub content: Vec<ContentBlock>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub is_error: bool,
}

/// Single content block in a tool result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContentBlock {
    #[serde(rename = "type")]
    pub block_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

/// Resource content returned by resource handlers.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceContent {
    pub uri: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blob: Option<String>,
}

/// Parsed schema metadata used for argument validation.
#[derive(Debug, Clone, Default)]
pub struct SchemaMeta {
    pub required: Vec<String>,
    pub one_of: Vec<SchemaRequirementSet>,
    pub dependencies: std::collections::HashMap<String, Vec<String>>,
}

/// A set of required fields for oneOf validation.
#[derive(Debug, Clone)]
pub struct SchemaRequirementSet {
    pub required: Vec<String>,
}

// ── Convenience constructors ──

/// Create a simple text tool result.
pub fn text_result(text: impl Into<String>) -> ToolResult {
    ToolResult {
        content: vec![ContentBlock {
            block_type: "text".into(),
            text: Some(text.into()),
        }],
        is_error: false,
    }
}

/// Create an error tool result.
pub fn error_result(text: impl Into<String>) -> ToolResult {
    ToolResult {
        content: vec![ContentBlock {
            block_type: "text".into(),
            text: Some(text.into()),
        }],
        is_error: true,
    }
}

/// Build a JSON-RPC error response.
pub fn new_error_response(id: Option<Value>, code: i32, message: impl Into<String>) -> JsonRpcResponse {
    JsonRpcResponse {
        jsonrpc: "2.0".into(),
        id,
        result: None,
        error: Some(RpcError {
            code,
            message: message.into(),
            data: None,
        }),
    }
}

/// Build a JSON-RPC success response.
pub fn new_ok_response(id: Option<Value>, result: Value) -> JsonRpcResponse {
    JsonRpcResponse {
        jsonrpc: "2.0".into(),
        id,
        result: Some(result),
        error: None,
    }
}

/// Build a notification sentinel (empty response, triggers HTTP 202).
pub fn notification_response() -> JsonRpcResponse {
    JsonRpcResponse {
        jsonrpc: "2.0".into(),
        id: None,
        result: None,
        error: None,
    }
}

/// MCP error type for the crate.
#[derive(Debug, thiserror::Error)]
pub enum McpError {
    #[error("validation error: {0}")]
    Validation(String),
    #[error("tool error: {0}")]
    ToolError(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("{0}")]
    Other(String),
}

// Internal params structs for deserialization.

#[derive(Debug, Deserialize)]
pub(crate) struct ToolCallParams {
    pub name: String,
    #[serde(default)]
    pub arguments: Value,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ResourceReadParams {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub uri: Option<String>,
}
