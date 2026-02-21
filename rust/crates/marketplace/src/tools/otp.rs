use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use std::collections::HashMap;

use mcpserver::{text_result, error_result, FnToolHandler, ToolResult, McpError};
use serde_json::Value;

use crate::auth;
use super::Deps;

const OTP_TTL_SECONDS: u64 = 300; // 5 minutes

pub fn register(srv: &mut mcpserver::Server, deps: Arc<Deps>) {
    let d = deps.clone();
    srv.handle_tool("otp-request", FnToolHandler::new(move |args: Value| {
        let deps = d.clone();
        async move { handle_otp_request(&deps, args).await }
    }));

    let d = deps;
    srv.handle_tool("otp-verify", FnToolHandler::new(move |args: Value| {
        let deps = d.clone();
        async move { handle_otp_verify(&deps, args).await }
    }));
}

async fn handle_otp_request(deps: &Deps, args: Value) -> Result<ToolResult, McpError> {
    let phone = args.get("phone").and_then(|v| v.as_str()).unwrap_or("");
    let email = args.get("email").and_then(|v| v.as_str()).unwrap_or("");

    if phone.is_empty() && email.is_empty() {
        return Ok(error_result("phone or email required"));
    }

    let code = generate_otp();
    let dest = if !phone.is_empty() { phone } else { email };

    // Store OTP in DynamoDB with TTL.
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let ttl = now + OTP_TTL_SECONDS;

    let mut attrs = HashMap::new();
    attrs.insert("TTL".into(), Value::Number(ttl.into()));

    if let Err(e) = deps.db.put_item(
        &format!("otp:{}", dest),
        &code,
        "", "", "", "",
        attrs,
    ).await {
        tracing::error!("otp put: {}", e);
        return Ok(error_result("failed to store OTP"));
    }

    // Deliver via SNS (phone) or SES (email).
    let msg = format!("Your verification code is: {}", code);
    if !phone.is_empty() {
        if let Err(e) = deps.sns.send_sms(phone, &msg).await {
            tracing::error!("otp sms: {}", e);
            return Ok(error_result("failed to send SMS"));
        }
    } else {
        if let Err(e) = deps.ses.send_email(&deps.ses_from_email, email, "Your verification code", &msg).await {
            tracing::error!("otp email: {}", e);
            return Ok(error_result("failed to send email"));
        }
    }

    Ok(text_result("OTP sent"))
}

async fn handle_otp_verify(deps: &Deps, args: Value) -> Result<ToolResult, McpError> {
    let phone = args.get("phone").and_then(|v| v.as_str()).unwrap_or("");
    let email = args.get("email").and_then(|v| v.as_str()).unwrap_or("");
    let code = args.get("code").and_then(|v| v.as_str()).unwrap_or("");

    if code.is_empty() {
        return Ok(error_result("code required"));
    }

    let dest = if !phone.is_empty() { phone } else { email };
    if dest.is_empty() {
        return Ok(error_result("phone or email required"));
    }

    // Look up OTP.
    let items = match deps.db.query_with_sk(&format!("otp:{}", dest), code).await {
        Ok(items) => items,
        Err(e) => {
            tracing::error!("otp query: {}", e);
            return Ok(error_result("verification failed"));
        }
    };

    if items.is_empty() {
        return Ok(error_result("invalid or expired code"));
    }

    // Delete the used OTP.
    let _ = deps.db.delete_item(&format!("otp:{}", dest), code).await;

    // Create JWT — userId derived from destination.
    let token = match auth::create_token(&deps.jwt_secret, dest, 86400) {
        Ok(t) => t,
        Err(e) => {
            tracing::error!("otp create token: {}", e);
            return Ok(error_result("failed to create token"));
        }
    };

    Ok(text_result(&token))
}

/// Generate a cryptographically random 6-digit OTP code.
fn generate_otp() -> String {
    use rand::Rng;
    let mut rng = rand::rng();
    let n: u32 = rng.random_range(0..1_000_000);
    format!("{:06}", n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_otp_format() {
        for _ in 0..100 {
            let code = generate_otp();
            assert_eq!(code.len(), 6);
            assert!(code.chars().all(|c| c.is_ascii_digit()));
        }
    }
}
