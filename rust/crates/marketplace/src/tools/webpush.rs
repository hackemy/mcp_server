use std::sync::Arc;
use std::collections::HashMap;

use mcpserver::{text_result, error_result, FnToolHandler, ToolResult, McpError};
use serde_json::Value;
use sha2::{Sha256, Digest};

use super::Deps;
use super::channel::authenticate;

pub fn register(srv: &mut mcpserver::Server, deps: Arc<Deps>) {
    let d = deps.clone();
    srv.handle_tool("web-push-enable", FnToolHandler::new(move |args: Value| {
        let deps = d.clone();
        async move { handle_web_push_enable(&deps, args).await }
    }));

    let d = deps;
    srv.handle_tool("web-push-disable", FnToolHandler::new(move |args: Value| {
        let deps = d.clone();
        async move { handle_web_push_disable(&deps, args).await }
    }));
}

async fn handle_web_push_enable(deps: &Deps, args: Value) -> Result<ToolResult, McpError> {
    let user_id = match authenticate(deps, &args) {
        Ok(id) => id,
        Err(msg) => return Ok(error_result(&msg)),
    };

    let subscription = args.get("subscription").and_then(|v| v.as_str()).unwrap_or("");
    if subscription.is_empty() {
        return Ok(error_result("subscription required"));
    }

    let hash = hash_endpoint(subscription);

    let mut attrs = HashMap::new();
    attrs.insert("subscription".into(), Value::String(subscription.into()));

    if let Err(e) = deps.db.put_item(
        &format!("web-push:{}", user_id),
        &hash,
        "", "", "", "",
        attrs,
    ).await {
        tracing::error!("web-push-enable: {}", e);
        return Ok(error_result("failed to enable web push"));
    }

    Ok(text_result("web push enabled"))
}

async fn handle_web_push_disable(deps: &Deps, args: Value) -> Result<ToolResult, McpError> {
    let user_id = match authenticate(deps, &args) {
        Ok(id) => id,
        Err(msg) => return Ok(error_result(&msg)),
    };

    let subscription = args.get("subscription").and_then(|v| v.as_str()).unwrap_or("");
    if subscription.is_empty() {
        return Ok(error_result("subscription required"));
    }

    let hash = hash_endpoint(subscription);

    if let Err(e) = deps.db.delete_item(&format!("web-push:{}", user_id), &hash).await {
        tracing::error!("web-push-disable: {}", e);
        return Ok(error_result("failed to disable web push"));
    }

    Ok(text_result("web push disabled"))
}

/// Creates a stable 16-char hex string from the first 8 bytes of the SHA-256 hash.
pub fn hash_endpoint(subscription: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(subscription.as_bytes());
    let result = hasher.finalize();
    // First 8 bytes = 16 hex chars, matching Go implementation.
    format!("{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        result[0], result[1], result[2], result[3],
        result[4], result[5], result[6], result[7])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hash_endpoint_consistency() {
        let h1 = hash_endpoint("https://example.com/push/abc");
        let h2 = hash_endpoint("https://example.com/push/abc");
        assert_eq!(h1, h2);
        assert_eq!(h1.len(), 16);
    }

    #[test]
    fn test_hash_endpoint_different_inputs() {
        let h1 = hash_endpoint("endpoint-a");
        let h2 = hash_endpoint("endpoint-b");
        assert_ne!(h1, h2);
    }
}
