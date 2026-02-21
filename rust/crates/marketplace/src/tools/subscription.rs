use std::sync::Arc;
use std::collections::HashMap;

use mcpserver::{text_result, error_result, FnToolHandler, ToolResult, McpError};
use serde_json::Value;

use super::Deps;
use super::channel::authenticate;

pub fn register(srv: &mut mcpserver::Server, deps: Arc<Deps>) {
    let d = deps.clone();
    srv.handle_tool("channel-subscribe", FnToolHandler::new(move |args: Value| {
        let deps = d.clone();
        async move { handle_channel_subscribe(&deps, args).await }
    }));

    let d = deps.clone();
    srv.handle_tool("channel-unsubscribe", FnToolHandler::new(move |args: Value| {
        let deps = d.clone();
        async move { handle_channel_unsubscribe(&deps, args).await }
    }));

    let d = deps;
    srv.handle_tool("subscriptions-list", FnToolHandler::new(move |args: Value| {
        let deps = d.clone();
        async move { handle_subscriptions_list(&deps, args).await }
    }));
}

async fn handle_channel_subscribe(deps: &Deps, args: Value) -> Result<ToolResult, McpError> {
    let user_id = match authenticate(deps, &args) {
        Ok(id) => id,
        Err(msg) => return Ok(error_result(&msg)),
    };

    let channel_id = args.get("channel").and_then(|v| v.as_str()).unwrap_or("");
    if channel_id.is_empty() {
        return Ok(error_result("channel required"));
    }

    // Verify channel exists via GSI1.
    let channels = match deps.db.query_gsi_with_sk("GSI1", "channel", channel_id).await {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("channel-subscribe verify: {}", e);
            return Ok(error_result("failed to verify channel"));
        }
    };

    if channels.is_empty() {
        return Ok(error_result("channel not found"));
    }

    // Create subscription.
    let mut attrs = HashMap::new();
    attrs.insert("subscribedAt".into(), Value::String(channel_id.into()));

    if let Err(e) = deps.db.put_item(
        &format!("subscription:{}", user_id),
        channel_id,
        "subscription",
        channel_id,
        "", "",
        attrs,
    ).await {
        tracing::error!("channel-subscribe put: {}", e);
        return Ok(error_result("failed to subscribe"));
    }

    Ok(text_result("subscribed"))
}

async fn handle_channel_unsubscribe(deps: &Deps, args: Value) -> Result<ToolResult, McpError> {
    let user_id = match authenticate(deps, &args) {
        Ok(id) => id,
        Err(msg) => return Ok(error_result(&msg)),
    };

    let channel_id = args.get("channel").and_then(|v| v.as_str()).unwrap_or("");
    if channel_id.is_empty() {
        return Ok(error_result("channel required"));
    }

    if let Err(e) = deps.db.delete_item(&format!("subscription:{}", user_id), channel_id).await {
        tracing::error!("channel-unsubscribe: {}", e);
        return Ok(error_result("failed to unsubscribe"));
    }

    Ok(text_result("unsubscribed"))
}

async fn handle_subscriptions_list(deps: &Deps, args: Value) -> Result<ToolResult, McpError> {
    let user_id = match authenticate(deps, &args) {
        Ok(id) => id,
        Err(msg) => return Ok(error_result(&msg)),
    };

    let items = match deps.db.query(&format!("subscription:{}", user_id)).await {
        Ok(items) => items,
        Err(e) => {
            tracing::error!("subscriptions-list: {}", e);
            return Ok(error_result("failed to list subscriptions"));
        }
    };

    let buf = serde_json::to_string(&items).unwrap_or_else(|_| "[]".into());
    Ok(text_result(&buf))
}
