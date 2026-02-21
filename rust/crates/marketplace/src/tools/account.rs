use std::sync::Arc;

use mcpserver::{text_result, error_result, FnToolHandler, ToolResult, McpError};
use serde_json::Value;

use crate::dynamo::KeyPair;
use super::Deps;
use super::channel::authenticate;

pub fn register(srv: &mut mcpserver::Server, deps: Arc<Deps>) {
    let d = deps;
    srv.handle_tool("account-delete", FnToolHandler::new(move |args: Value| {
        let deps = d.clone();
        async move { handle_account_delete(&deps, args).await }
    }));
}

async fn handle_account_delete(deps: &Deps, args: Value) -> Result<ToolResult, McpError> {
    let user_id = match authenticate(deps, &args) {
        Ok(id) => id,
        Err(msg) => return Ok(error_result(&msg)),
    };

    // 1. Get all owned channels — we need to cascade-delete their subscriptions.
    let channels = match deps.db.query(&format!("channel:{}", user_id)).await {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("account-delete query channels: {}", e);
            Vec::new()
        }
    };

    let mut all_pairs: Vec<KeyPair> = Vec::new();

    // For each owned channel, find and queue subscription + message deletions.
    for ch in &channels {
        let ch_id = match ch.get("SK").and_then(|v| v.as_str()) {
            Some(id) => id,
            None => continue,
        };

        // Queue channel deletion.
        all_pairs.push(KeyPair {
            pk: format!("channel:{}", user_id),
            sk: ch_id.into(),
        });

        // Find subscriptions to this channel via GSI1.
        match deps.db.query_gsi_with_sk("GSI1", "subscription", ch_id).await {
            Ok(subs) => {
                for s in &subs {
                    if let (Some(pk), Some(sk)) = (
                        s.get("PK").and_then(|v| v.as_str()),
                        s.get("SK").and_then(|v| v.as_str()),
                    ) {
                        all_pairs.push(KeyPair { pk: pk.into(), sk: sk.into() });
                    }
                }
            }
            Err(e) => {
                tracing::error!("account-delete query subs for channel {}: {}", ch_id, e);
            }
        }

        // Delete messages for this channel.
        match deps.db.query(&format!("message:{}", ch_id)).await {
            Ok(msgs) => {
                for m in &msgs {
                    if let (Some(pk), Some(sk)) = (
                        m.get("PK").and_then(|v| v.as_str()),
                        m.get("SK").and_then(|v| v.as_str()),
                    ) {
                        all_pairs.push(KeyPair { pk: pk.into(), sk: sk.into() });
                    }
                }
            }
            Err(e) => {
                tracing::error!("account-delete query messages channel {}: {}", ch_id, e);
            }
        }
    }

    // 2. Delete user's own subscriptions.
    match deps.db.query(&format!("subscription:{}", user_id)).await {
        Ok(user_subs) => {
            for s in &user_subs {
                if let (Some(pk), Some(sk)) = (
                    s.get("PK").and_then(|v| v.as_str()),
                    s.get("SK").and_then(|v| v.as_str()),
                ) {
                    all_pairs.push(KeyPair { pk: pk.into(), sk: sk.into() });
                }
            }
        }
        Err(e) => {
            tracing::error!("account-delete query user subs: {}", e);
        }
    }

    // 3. Delete user's web-push subscriptions.
    match deps.db.query(&format!("web-push:{}", user_id)).await {
        Ok(push_subs) => {
            for ps in &push_subs {
                if let (Some(pk), Some(sk)) = (
                    ps.get("PK").and_then(|v| v.as_str()),
                    ps.get("SK").and_then(|v| v.as_str()),
                ) {
                    all_pairs.push(KeyPair { pk: pk.into(), sk: sk.into() });
                }
            }
        }
        Err(e) => {
            tracing::error!("account-delete query push subs: {}", e);
        }
    }

    // 4. Batch delete everything.
    if !all_pairs.is_empty() {
        if let Err(e) = deps.db.batch_delete_items(&all_pairs).await {
            tracing::error!("account-delete batch delete: {}", e);
            return Ok(error_result("partial deletion, please retry"));
        }
    }

    Ok(text_result("account deleted"))
}
