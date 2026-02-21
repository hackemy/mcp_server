use std::sync::Arc;
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use mcpserver::{text_result, error_result, FnToolHandler, ToolResult, McpError};
use serde_json::Value;

use crate::notify::webpush;
use super::Deps;
use super::channel::authenticate;

pub fn register(srv: &mut mcpserver::Server, deps: Arc<Deps>) {
    let d = deps.clone();
    srv.handle_tool("channel-notify", FnToolHandler::new(move |args: Value| {
        let deps = d.clone();
        async move { handle_channel_notify(&deps, args).await }
    }));

    let d = deps;
    srv.handle_tool("channel-messages", FnToolHandler::new(move |args: Value| {
        let deps = d.clone();
        async move { handle_channel_messages(&deps, args).await }
    }));
}

async fn handle_channel_notify(deps: &Deps, args: Value) -> Result<ToolResult, McpError> {
    let user_id = match authenticate(deps, &args) {
        Ok(id) => id,
        Err(msg) => return Ok(error_result(&msg)),
    };

    let channel_id = args.get("channel").and_then(|v| v.as_str()).unwrap_or("");
    let message = args.get("message").and_then(|v| v.as_str()).unwrap_or("");

    if channel_id.is_empty() || message.is_empty() {
        return Ok(error_result("channel and message required"));
    }

    // Store the message in DynamoDB.
    // Use nanosecond precision to avoid collisions in rapid succession.
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos()
        .to_string();

    let mut attrs = HashMap::new();
    attrs.insert("sender".into(), Value::String(user_id.clone()));
    attrs.insert("message".into(), Value::String(message.into()));

    if let Err(e) = deps.db.put_item(
        &format!("message:{}", channel_id),
        &ts,
        "", "", "", "",
        attrs,
    ).await {
        tracing::error!("channel-notify put: {}", e);
        return Ok(error_result("failed to store message"));
    }

    // Fan-out: find all subscribers to this channel via GSI1.
    let subs = match deps.db.query_gsi_with_sk("GSI1", "subscription", channel_id).await {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("channel-notify query subs: {}", e);
            Vec::new()
        }
    };

    // For each subscriber, find their web-push subscriptions and send.
    let mut push_count: usize = 0;
    for sub in &subs {
        let sub_pk = match sub.get("PK").and_then(|v| v.as_str()) {
            Some(pk) => pk,
            None => continue,
        };

        // Extract subscriber userId from PK ("subscription:{userId}").
        let subscriber_id = &sub_pk["subscription:".len()..];

        // Get web-push subscriptions for this subscriber.
        let push_subs = match deps.db.query(&format!("web-push:{}", subscriber_id)).await {
            Ok(ps) => ps,
            Err(e) => {
                tracing::error!("channel-notify query push subscriber={}: {}", subscriber_id, e);
                continue;
            }
        };

        let payload = serde_json::json!({
            "channel": channel_id,
            "message": message,
            "sender": user_id,
        });

        for ps in &push_subs {
            let sub_json = match ps.get("subscription").and_then(|v| v.as_str()) {
                Some(s) => s,
                None => continue,
            };
            if let Err(e) = webpush::send_web_push(sub_json, &payload, &deps.web_push_keys) {
                tracing::error!("web-push send subscriber={}: {}", subscriber_id, e);
            } else {
                push_count += 1;
            }
        }
    }

    let result = serde_json::json!({
        "stored": true,
        "pushSent": push_count,
        "subscribers": subs.len(),
    });
    Ok(text_result(&result.to_string()))
}

async fn handle_channel_messages(deps: &Deps, args: Value) -> Result<ToolResult, McpError> {
    if let Err(msg) = authenticate(deps, &args) {
        return Ok(error_result(&msg));
    }

    let channel_id = args.get("channel").and_then(|v| v.as_str()).unwrap_or("");
    if channel_id.is_empty() {
        return Ok(error_result("channel required"));
    }

    let items = match deps.db.query(&format!("message:{}", channel_id)).await {
        Ok(items) => items,
        Err(e) => {
            tracing::error!("channel-messages: {}", e);
            return Ok(error_result("failed to list messages"));
        }
    };

    let buf = serde_json::to_string(&items).unwrap_or_else(|_| "[]".into());
    Ok(text_result(&buf))
}
