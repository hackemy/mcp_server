pub mod otp;
pub mod channel;
pub mod channel_msg;
pub mod subscription;
pub mod webpush;
pub mod account;

use std::sync::Arc;
use crate::dynamo::DynamoApi;
use crate::notify::sns::SnsApi;
use crate::notify::ses::SesApi;
use crate::notify::webpush::WebPushKeys;

/// Shared dependencies for all tool handlers.
pub struct Deps {
    pub db: Arc<dyn DynamoApi>,
    pub jwt_secret: String,
    pub sns: Arc<dyn SnsApi>,
    pub ses: Arc<dyn SesApi>,
    pub ses_from_email: String,
    pub web_push_keys: WebPushKeys,
}

/// Register all tool handlers on the given MCP server.
pub fn register_all(srv: &mut mcpserver::Server, deps: Arc<Deps>) {
    otp::register(srv, deps.clone());
    channel::register(srv, deps.clone());
    channel_msg::register(srv, deps.clone());
    subscription::register(srv, deps.clone());
    webpush::register(srv, deps.clone());
    account::register(srv, deps);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth;
    use crate::dynamo::mock::MockDynamo;
    use crate::notify::sns::mock::MockSns;
    use crate::notify::ses::mock::MockSes;
    use mcpserver::JsonRpcRequest;
    use serde_json::{json, Value};

    const TEST_SECRET: &str = "test-secret-key-for-hmac256";

    fn test_token(user_id: &str) -> String {
        auth::create_token(TEST_SECRET, user_id, 3600).unwrap()
    }

    fn setup_deps() -> (Arc<Deps>, Arc<MockDynamo>, Arc<MockSns>, Arc<MockSes>) {
        let db = Arc::new(MockDynamo::new());
        let sns = Arc::new(MockSns::new());
        let ses = Arc::new(MockSes::new());

        let deps = Arc::new(Deps {
            db: db.clone(),
            jwt_secret: TEST_SECRET.into(),
            sns: sns.clone(),
            ses: ses.clone(),
            ses_from_email: "noreply@example.com".into(),
            web_push_keys: WebPushKeys::default(),
        });

        (deps, db, sns, ses)
    }

    fn setup_server(deps: Arc<Deps>) -> mcpserver::Server {
        let tools_json = include_bytes!("../../tools.json");
        let resources_json = include_bytes!("../../resources.json");

        let mut srv = mcpserver::Server::builder()
            .tools_json(tools_json)
            .resources_json(resources_json)
            .server_info("test-app", "0.0.1")
            .build();

        register_all(&mut srv, deps);
        srv
    }

    async fn call_tool(srv: &mcpserver::Server, name: &str, args: Value) -> Value {
        let req = JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id: Some(json!(1)),
            method: "tools/call".into(),
            params: Some(json!({
                "name": name,
                "arguments": args,
            })),
        };

        let resp = srv.handle(req).await;
        // Extract the text from the first content block.
        if let Some(result) = resp.result {
            if let Some(content) = result.get("content").and_then(|c| c.as_array()) {
                if let Some(first) = content.first() {
                    if let Some(text) = first.get("text").and_then(|t| t.as_str()) {
                        // Try to parse as JSON; if it fails, return as string.
                        if let Ok(parsed) = serde_json::from_str::<Value>(text) {
                            return parsed;
                        }
                        return Value::String(text.into());
                    }
                }
            }
            return result;
        }

        if let Some(err) = resp.error {
            return json!({"error": err.message});
        }

        Value::Null
    }

    fn is_error(result: &Value) -> bool {
        // Check if it's a string containing typical error messages.
        if let Some(s) = result.as_str() {
            return s.contains("required") || s.contains("invalid") ||
                   s.contains("failed") || s.contains("not found") ||
                   s.contains("expired");
        }
        result.get("error").is_some()
    }

    // ─── OTP Tests ───

    #[tokio::test]
    async fn test_otp_request_phone() {
        let (deps, _db, sns, _ses) = setup_deps();
        let srv = setup_server(deps);

        let result = call_tool(&srv, "otp-request", json!({
            "phone": "+15551234567"
        })).await;

        assert_eq!(result.as_str().unwrap(), "OTP sent");

        // Verify SNS was called.
        let msgs = sns.messages.lock().unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].0, "+15551234567");
    }

    #[tokio::test]
    async fn test_otp_request_email() {
        let (deps, _db, _sns, ses) = setup_deps();
        let srv = setup_server(deps);

        let result = call_tool(&srv, "otp-request", json!({
            "email": "user@example.com"
        })).await;

        assert_eq!(result.as_str().unwrap(), "OTP sent");

        // Verify SES was called.
        let emails = ses.emails.lock().unwrap();
        assert_eq!(emails.len(), 1);
        assert_eq!(emails[0].0, "user@example.com");
    }

    #[tokio::test]
    async fn test_otp_verify_success() {
        let (deps, db, _sns, _ses) = setup_deps();
        let srv = setup_server(deps);

        // Request OTP first.
        call_tool(&srv, "otp-request", json!({
            "phone": "+15551234567"
        })).await;

        // Find the stored OTP code from the mock DB.
        let items = db.query("otp:+15551234567").await.unwrap();
        assert!(!items.is_empty());
        let code = items[0].get("SK").unwrap().as_str().unwrap();

        // Verify it.
        let result = call_tool(&srv, "otp-verify", json!({
            "phone": "+15551234567",
            "code": code
        })).await;

        // Should return a JWT token.
        let token_str = result.as_str().unwrap();
        assert!(!token_str.is_empty());
        let user_id = auth::parse_token(TEST_SECRET, token_str).unwrap();
        assert_eq!(user_id, "+15551234567");

        // OTP should be deleted.
        let items = db.query_with_sk("otp:+15551234567", code).await.unwrap();
        assert!(items.is_empty());
    }

    #[tokio::test]
    async fn test_otp_verify_invalid_code() {
        let (deps, _db, _sns, _ses) = setup_deps();
        let srv = setup_server(deps);

        let result = call_tool(&srv, "otp-verify", json!({
            "phone": "+15551234567",
            "code": "000000"
        })).await;

        assert!(result.as_str().unwrap().contains("invalid"));
    }

    // ─── Channel Tests ───

    #[tokio::test]
    async fn test_channel_put_create() {
        let (deps, _db, _sns, _ses) = setup_deps();
        let srv = setup_server(deps);
        let token = test_token("user1");

        let result = call_tool(&srv, "channel-put", json!({
            "token": token,
            "channel": "",
            "name": "Test Channel",
            "category": "food",
            "poster": "owner"
        })).await;

        let channel_id = result.get("channelId").unwrap().as_str().unwrap();
        assert_eq!(channel_id.len(), 7);
    }

    #[tokio::test]
    async fn test_channel_put_vehicles_category() {
        let (deps, _db, _sns, _ses) = setup_deps();
        let srv = setup_server(deps);
        let token = test_token("user1");

        let result = call_tool(&srv, "channel-put", json!({
            "token": token,
            "channel": "",
            "name": "My Car",
            "category": "VEHICLES",
            "poster": "owner"
        })).await;

        let channel_id = result.get("channelId").unwrap().as_str().unwrap();
        assert_eq!(channel_id, "MYCAR");
    }

    #[tokio::test]
    async fn test_channel_put_unauthenticated() {
        let (deps, _db, _sns, _ses) = setup_deps();
        let srv = setup_server(deps);

        let result = call_tool(&srv, "channel-put", json!({
            "token": "bad-token",
            "channel": "",
            "name": "Test",
            "category": "food",
            "poster": "owner"
        })).await;

        assert!(is_error(&result));
    }

    #[tokio::test]
    async fn test_channels_list() {
        let (deps, _db, _sns, _ses) = setup_deps();
        let srv = setup_server(deps);
        let token = test_token("user1");

        // Create 2 channels.
        call_tool(&srv, "channel-put", json!({
            "token": token,
            "channel": "",
            "name": "Channel A",
            "category": "food",
            "poster": "owner"
        })).await;

        call_tool(&srv, "channel-put", json!({
            "token": token,
            "channel": "",
            "name": "Channel B",
            "category": "food",
            "poster": "owner"
        })).await;

        let result = call_tool(&srv, "channels-list", json!({
            "token": token,
        })).await;

        let items = result.as_array().unwrap();
        assert_eq!(items.len(), 2);
    }

    #[tokio::test]
    async fn test_channel_delete() {
        let (deps, _db, _sns, _ses) = setup_deps();
        let srv = setup_server(deps);
        let token = test_token("user1");

        // Create channel.
        let result = call_tool(&srv, "channel-put", json!({
            "token": token,
            "channel": "",
            "name": "To Delete",
            "category": "food",
            "poster": "owner"
        })).await;
        let channel_id = result.get("channelId").unwrap().as_str().unwrap().to_string();

        // Subscribe another user.
        let token2 = test_token("user2");
        call_tool(&srv, "channel-subscribe", json!({
            "token": token2,
            "channel": channel_id,
        })).await;

        // Delete channel.
        let result = call_tool(&srv, "channel-delete", json!({
            "token": token,
            "channel": channel_id,
        })).await;
        assert_eq!(result.as_str().unwrap(), "channel deleted");

        // Verify it's gone.
        let result = call_tool(&srv, "channels-list", json!({
            "token": token,
        })).await;
        let items = result.as_array().unwrap();
        assert_eq!(items.len(), 0);
    }

    #[tokio::test]
    async fn test_channels_for_category() {
        let (deps, _db, _sns, _ses) = setup_deps();
        let srv = setup_server(deps);
        let token = test_token("user1");

        call_tool(&srv, "channel-put", json!({
            "token": token,
            "channel": "",
            "name": "My Car",
            "category": "VEHICLES",
            "poster": "owner"
        })).await;

        call_tool(&srv, "channel-put", json!({
            "token": token,
            "channel": "",
            "name": "Sushi Spot",
            "category": "FOOD",
            "poster": "owner"
        })).await;

        let result = call_tool(&srv, "channels-for-category", json!({
            "token": token,
            "category": "vehicles",
        })).await;

        let items = result.as_array().unwrap();
        assert_eq!(items.len(), 1);
    }

    // ─── Subscription Tests ───

    #[tokio::test]
    async fn test_channel_subscribe() {
        let (deps, _db, _sns, _ses) = setup_deps();
        let srv = setup_server(deps);
        let token1 = test_token("user1");
        let token2 = test_token("user2");

        // Create channel.
        let result = call_tool(&srv, "channel-put", json!({
            "token": token1,
            "channel": "",
            "name": "Subscribe Me",
            "category": "food",
            "poster": "owner"
        })).await;
        let channel_id = result.get("channelId").unwrap().as_str().unwrap().to_string();

        // Subscribe.
        let result = call_tool(&srv, "channel-subscribe", json!({
            "token": token2,
            "channel": channel_id,
        })).await;
        assert_eq!(result.as_str().unwrap(), "subscribed");
    }

    #[tokio::test]
    async fn test_channel_subscribe_not_found() {
        let (deps, _db, _sns, _ses) = setup_deps();
        let srv = setup_server(deps);
        let token = test_token("user1");

        let result = call_tool(&srv, "channel-subscribe", json!({
            "token": token,
            "channel": "nonexistent",
        })).await;

        assert!(result.as_str().unwrap().contains("not found"));
    }

    #[tokio::test]
    async fn test_channel_unsubscribe() {
        let (deps, _db, _sns, _ses) = setup_deps();
        let srv = setup_server(deps);
        let token1 = test_token("user1");
        let token2 = test_token("user2");

        // Create and subscribe.
        let result = call_tool(&srv, "channel-put", json!({
            "token": token1,
            "channel": "",
            "name": "Unsub Me",
            "category": "food",
            "poster": "owner"
        })).await;
        let channel_id = result.get("channelId").unwrap().as_str().unwrap().to_string();

        call_tool(&srv, "channel-subscribe", json!({
            "token": token2,
            "channel": channel_id,
        })).await;

        // Unsubscribe.
        let result = call_tool(&srv, "channel-unsubscribe", json!({
            "token": token2,
            "channel": channel_id,
        })).await;
        assert_eq!(result.as_str().unwrap(), "unsubscribed");

        // Verify list is empty.
        let result = call_tool(&srv, "subscriptions-list", json!({
            "token": token2,
        })).await;
        let items = result.as_array().unwrap();
        assert_eq!(items.len(), 0);
    }

    #[tokio::test]
    async fn test_subscriptions_list() {
        let (deps, _db, _sns, _ses) = setup_deps();
        let srv = setup_server(deps);
        let token1 = test_token("user1");
        let token2 = test_token("user2");

        // Create 2 channels and subscribe to both.
        for name in &["Channel A", "Channel B"] {
            let result = call_tool(&srv, "channel-put", json!({
                "token": token1,
                "channel": "",
                "name": name,
                "category": "food",
                "poster": "owner"
            })).await;
            let ch = result.get("channelId").unwrap().as_str().unwrap().to_string();
            call_tool(&srv, "channel-subscribe", json!({
                "token": token2,
                "channel": ch,
            })).await;
        }

        let result = call_tool(&srv, "subscriptions-list", json!({
            "token": token2,
        })).await;
        let items = result.as_array().unwrap();
        assert_eq!(items.len(), 2);
    }

    // ─── Channel Message Tests ───

    #[tokio::test]
    async fn test_channel_notify() {
        let (deps, _db, _sns, _ses) = setup_deps();
        let srv = setup_server(deps);
        let token = test_token("user1");

        // Create channel.
        let result = call_tool(&srv, "channel-put", json!({
            "token": token,
            "channel": "",
            "name": "Notify Test",
            "category": "food",
            "poster": "owner"
        })).await;
        let channel_id = result.get("channelId").unwrap().as_str().unwrap().to_string();

        // Send message.
        let result = call_tool(&srv, "channel-notify", json!({
            "token": token,
            "channel": channel_id,
            "message": "Hello everyone!"
        })).await;

        assert_eq!(result.get("stored").unwrap().as_bool().unwrap(), true);
    }

    #[tokio::test]
    async fn test_channel_messages() {
        let (deps, _db, _sns, _ses) = setup_deps();
        let srv = setup_server(deps);
        let token = test_token("user1");

        // Create channel.
        let result = call_tool(&srv, "channel-put", json!({
            "token": token,
            "channel": "",
            "name": "Msg Test",
            "category": "food",
            "poster": "owner"
        })).await;
        let channel_id = result.get("channelId").unwrap().as_str().unwrap().to_string();

        // Send 2 messages.
        for msg in &["First msg", "Second msg"] {
            call_tool(&srv, "channel-notify", json!({
                "token": token,
                "channel": channel_id,
                "message": msg
            })).await;
        }

        // List messages.
        let result = call_tool(&srv, "channel-messages", json!({
            "token": token,
            "channel": channel_id,
        })).await;
        let items = result.as_array().unwrap();
        assert_eq!(items.len(), 2);
    }

    #[tokio::test]
    async fn test_channel_notify_unauthenticated() {
        let (deps, _db, _sns, _ses) = setup_deps();
        let srv = setup_server(deps);

        let result = call_tool(&srv, "channel-notify", json!({
            "token": "bad-token",
            "channel": "ch1",
            "message": "Hello"
        })).await;

        assert!(is_error(&result));
    }

    // ─── WebPush Tests ───

    #[tokio::test]
    async fn test_web_push_enable() {
        let (deps, db, _sns, _ses) = setup_deps();
        let srv = setup_server(deps);
        let token = test_token("user1");

        let result = call_tool(&srv, "web-push-enable", json!({
            "token": token,
            "subscription": "{\"endpoint\":\"https://push.example.com/abc\"}"
        })).await;

        assert_eq!(result.as_str().unwrap(), "web push enabled");

        // Verify stored in DB.
        let items = db.query("web-push:user1").await.unwrap();
        assert_eq!(items.len(), 1);
    }

    #[tokio::test]
    async fn test_web_push_disable() {
        let (deps, db, _sns, _ses) = setup_deps();
        let srv = setup_server(deps);
        let token = test_token("user1");

        let sub = "{\"endpoint\":\"https://push.example.com/abc\"}";

        // Enable.
        call_tool(&srv, "web-push-enable", json!({
            "token": token,
            "subscription": sub,
        })).await;

        // Disable.
        let result = call_tool(&srv, "web-push-disable", json!({
            "token": token,
            "subscription": sub,
        })).await;
        assert_eq!(result.as_str().unwrap(), "web push disabled");

        // Verify removed.
        let items = db.query("web-push:user1").await.unwrap();
        assert_eq!(items.len(), 0);
    }

    #[tokio::test]
    async fn test_web_push_enable_unauthenticated() {
        let (deps, _db, _sns, _ses) = setup_deps();
        let srv = setup_server(deps);

        let result = call_tool(&srv, "web-push-enable", json!({
            "token": "bad-token",
            "subscription": "{\"endpoint\":\"https://push.example.com/abc\"}"
        })).await;

        assert!(is_error(&result));
    }

    #[tokio::test]
    async fn test_web_push_enable_missing_subscription() {
        let (deps, _db, _sns, _ses) = setup_deps();
        let srv = setup_server(deps);
        let token = test_token("user1");

        let result = call_tool(&srv, "web-push-enable", json!({
            "token": token,
        })).await;

        // Should get a validation error from schema (subscription is required).
        assert!(is_error(&result) || result.as_str().map_or(false, |s| s.contains("required") || s.contains("subscription")));
    }

    // ─── Account Delete Tests ───

    #[tokio::test]
    async fn test_account_delete() {
        let (deps, db, _sns, _ses) = setup_deps();
        let srv = setup_server(deps);
        let token1 = test_token("user1");
        let token2 = test_token("user2");

        // Create channel.
        let result = call_tool(&srv, "channel-put", json!({
            "token": token1,
            "channel": "",
            "name": "Delete Me",
            "category": "food",
            "poster": "owner"
        })).await;
        let channel_id = result.get("channelId").unwrap().as_str().unwrap().to_string();

        // Subscribe user2.
        call_tool(&srv, "channel-subscribe", json!({
            "token": token2,
            "channel": channel_id,
        })).await;

        // Send a message.
        call_tool(&srv, "channel-notify", json!({
            "token": token1,
            "channel": channel_id,
            "message": "Hello"
        })).await;

        // Enable web push for user1.
        call_tool(&srv, "web-push-enable", json!({
            "token": token1,
            "subscription": "{\"endpoint\":\"https://push.example.com/xyz\"}"
        })).await;

        // Delete user1 account.
        let result = call_tool(&srv, "account-delete", json!({
            "token": token1,
        })).await;
        assert_eq!(result.as_str().unwrap(), "account deleted");

        // Verify channels gone.
        let items = db.query("channel:user1").await.unwrap();
        assert_eq!(items.len(), 0);

        // Verify web push subs gone.
        let items = db.query("web-push:user1").await.unwrap();
        assert_eq!(items.len(), 0);

        // Verify messages gone.
        let items = db.query(&format!("message:{}", channel_id)).await.unwrap();
        assert_eq!(items.len(), 0);
    }

    #[tokio::test]
    async fn test_account_delete_unauthenticated() {
        let (deps, _db, _sns, _ses) = setup_deps();
        let srv = setup_server(deps);

        let result = call_tool(&srv, "account-delete", json!({
            "token": "bad-token",
        })).await;

        assert!(is_error(&result));
    }
}
