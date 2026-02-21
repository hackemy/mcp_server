use std::sync::Arc;
use std::collections::HashMap;

use mcpserver::{text_result, error_result, FnToolHandler, ToolResult, McpError};
use serde_json::Value;

use crate::auth;
use crate::dynamo::KeyPair;
use super::Deps;

const NANOID_ALPHABET: &[u8] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";

pub fn register(srv: &mut mcpserver::Server, deps: Arc<Deps>) {
    let d = deps.clone();
    srv.handle_tool("channel-put", FnToolHandler::new(move |args: Value| {
        let deps = d.clone();
        async move { handle_channel_put(&deps, args).await }
    }));

    let d = deps.clone();
    srv.handle_tool("channel-delete", FnToolHandler::new(move |args: Value| {
        let deps = d.clone();
        async move { handle_channel_delete(&deps, args).await }
    }));

    let d = deps.clone();
    srv.handle_tool("channels-list", FnToolHandler::new(move |args: Value| {
        let deps = d.clone();
        async move { handle_channels_list(&deps, args).await }
    }));

    let d = deps;
    srv.handle_tool("channels-for-category", FnToolHandler::new(move |args: Value| {
        let deps = d.clone();
        async move { handle_channels_for_category(&deps, args).await }
    }));
}

async fn handle_channel_put(deps: &Deps, args: Value) -> Result<ToolResult, McpError> {
    let user_id = match authenticate(deps, &args) {
        Ok(id) => id,
        Err(msg) => return Ok(error_result(&msg)),
    };

    let name = args.get("name").and_then(|v| v.as_str()).unwrap_or("");
    let category = args.get("category").and_then(|v| v.as_str()).unwrap_or("");
    let poster = args.get("poster").and_then(|v| v.as_str()).unwrap_or("");
    let channel_arg = args.get("channel").and_then(|v| v.as_str()).unwrap_or("");

    // Generate channel ID if not provided (new channel).
    let channel_id = if !channel_arg.is_empty() {
        channel_arg.to_string()
    } else if category.eq_ignore_ascii_case("VEHICLES") {
        name.to_uppercase().replace(' ', "")
    } else {
        nanoid(7)
    };

    let mut attrs: HashMap<String, Value> = HashMap::new();
    attrs.insert("name".into(), Value::String(name.into()));
    attrs.insert("category".into(), Value::String(category.into()));
    attrs.insert("poster".into(), Value::String(poster.into()));
    attrs.insert("owner".into(), Value::String(user_id.clone()));

    if let Some(desc) = args.get("description").and_then(|v| v.as_str()) {
        if !desc.is_empty() {
            attrs.insert("description".into(), Value::String(desc.into()));
        }
    }
    if let Some(addr) = args.get("address").and_then(|v| v.as_str()) {
        if !addr.is_empty() {
            attrs.insert("address".into(), Value::String(addr.into()));
        }
    }
    if let Some(lat) = args.get("geo_lat").and_then(|v| v.as_f64()) {
        attrs.insert("geo_lat".into(), serde_json::json!(lat));
    }
    if let Some(lon) = args.get("geo_lon").and_then(|v| v.as_f64()) {
        attrs.insert("geo_lon".into(), serde_json::json!(lon));
    }

    if let Err(e) = deps.db.put_item(
        &format!("channel:{}", user_id),
        &channel_id,
        "channel",
        &channel_id,
        "channel",
        &category.to_uppercase(),
        attrs,
    ).await {
        tracing::error!("channel-put: {}", e);
        return Ok(error_result("failed to create channel"));
    }

    let result = serde_json::json!({"channelId": channel_id});
    Ok(text_result(&result.to_string()))
}

async fn handle_channel_delete(deps: &Deps, args: Value) -> Result<ToolResult, McpError> {
    let user_id = match authenticate(deps, &args) {
        Ok(id) => id,
        Err(msg) => return Ok(error_result(&msg)),
    };

    let channel_id = args.get("channel").and_then(|v| v.as_str()).unwrap_or("");
    if channel_id.is_empty() {
        return Ok(error_result("channel required"));
    }

    // Delete the channel itself.
    if let Err(e) = deps.db.delete_item(&format!("channel:{}", user_id), channel_id).await {
        tracing::error!("channel-delete: {}", e);
        return Ok(error_result("failed to delete channel"));
    }

    // Cascade: find and delete all subscriptions to this channel via GSI1.
    match deps.db.query_gsi_with_sk("GSI1", "subscription", channel_id).await {
        Ok(subs) if !subs.is_empty() => {
            let pairs: Vec<KeyPair> = subs.iter()
                .filter_map(|s| {
                    let pk = s.get("PK")?.as_str()?;
                    let sk = s.get("SK")?.as_str()?;
                    Some(KeyPair { pk: pk.into(), sk: sk.into() })
                })
                .collect();
            if let Err(e) = deps.db.batch_delete_items(&pairs).await {
                tracing::error!("channel-delete cascade delete: {}", e);
            }
        }
        Err(e) => {
            tracing::error!("channel-delete cascade query: {}", e);
        }
        _ => {}
    }

    Ok(text_result("channel deleted"))
}

async fn handle_channels_list(deps: &Deps, args: Value) -> Result<ToolResult, McpError> {
    let user_id = match authenticate(deps, &args) {
        Ok(id) => id,
        Err(msg) => return Ok(error_result(&msg)),
    };

    let items = match deps.db.query(&format!("channel:{}", user_id)).await {
        Ok(items) => items,
        Err(e) => {
            tracing::error!("channels-list: {}", e);
            return Ok(error_result("failed to list channels"));
        }
    };

    let buf = serde_json::to_string(&items).unwrap_or_else(|_| "[]".into());
    Ok(text_result(&buf))
}

async fn handle_channels_for_category(deps: &Deps, args: Value) -> Result<ToolResult, McpError> {
    if let Err(msg) = authenticate(deps, &args) {
        return Ok(error_result(&msg));
    }

    let category = args.get("category").and_then(|v| v.as_str()).unwrap_or("");
    if category.is_empty() {
        return Ok(error_result("category required"));
    }

    let items = match deps.db.query_gsi_with_sk("GSI2", "channel", &category.to_uppercase()).await {
        Ok(items) => items,
        Err(e) => {
            tracing::error!("channels-for-category: {}", e);
            return Ok(error_result("failed to query channels"));
        }
    };

    let buf = serde_json::to_string(&items).unwrap_or_else(|_| "[]".into());
    Ok(text_result(&buf))
}

/// Validates the JWT token from tool arguments.
pub fn authenticate(deps: &Deps, args: &Value) -> Result<String, String> {
    let token = args.get("token").and_then(|v| v.as_str()).unwrap_or("");
    if token.is_empty() {
        return Err("invalid or expired token".into());
    }
    auth::parse_token(&deps.jwt_secret, token).map_err(|e| e.to_string())
}

/// Generates a random alphanumeric string of the given length.
pub fn nanoid(length: usize) -> String {
    use rand::Rng;
    let mut rng = rand::rng();
    (0..length)
        .map(|_| {
            let idx = rng.random_range(0..NANOID_ALPHABET.len());
            NANOID_ALPHABET[idx] as char
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_nanoid_length() {
        let id = nanoid(7);
        assert_eq!(id.len(), 7);
        assert!(id.chars().all(|c| c.is_ascii_alphanumeric()));
    }
}
