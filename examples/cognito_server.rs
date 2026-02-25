//! MCP server example with AWS Cognito JWT authentication.
//!
//! Demonstrates how to validate Cognito JWTs in Axum middleware and pass
//! the decoded claims as context to MCP tool handlers.
//!
//! ## Setup
//!
//! Set environment variables before running:
//!
//! ```bash
//! export COGNITO_REGION=us-east-1
//! export COGNITO_USER_POOL_ID=us-east-1_aBcDeFgHi
//! export COGNITO_CLIENT_ID=1234567890abcdef
//! cargo run --example cognito_server
//! ```
//!
//! ## Testing
//!
//! ```bash
//! # Get a token from Cognito (e.g. via hosted UI, Amplify, or CLI)
//! TOKEN="eyJraWQiOi..."
//!
//! # Call the protected MCP endpoint
//! curl -s -X POST http://localhost:3000/mcp \
//!   -H "Content-Type: application/json" \
//!   -H "Authorization: Bearer $TOKEN" \
//!   -d '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"whoami","arguments":{}}}' | jq .
//! ```

use std::sync::Arc;

use async_trait::async_trait;
use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use jsonwebtoken::{decode, decode_header, Algorithm, DecodingKey, Validation};
use mcpserver::{
    text_result, FnToolHandler, JsonRpcRequest, McpError, McpResponse, Server, ToolHandler,
    ToolResult,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

// ── Cognito configuration ──

#[derive(Clone)]
struct CognitoConfig {
    region: String,
    user_pool_id: String,
    client_id: String,
    jwks: JwkSet,
}

impl CognitoConfig {
    fn issuer(&self) -> String {
        format!(
            "https://cognito-idp.{}.amazonaws.com/{}",
            self.region, self.user_pool_id
        )
    }

    fn jwks_url(&self) -> String {
        format!("{}/.well-known/jwks.json", self.issuer())
    }
}

// ── JWKS types (matches Cognito's JWKS response) ──

#[derive(Debug, Clone, Deserialize)]
struct JwkSet {
    keys: Vec<Jwk>,
}

#[derive(Debug, Clone, Deserialize)]
struct Jwk {
    kid: String,
    n: String,
    e: String,
}

impl JwkSet {
    fn find_key(&self, kid: &str) -> Option<&Jwk> {
        self.keys.iter().find(|k| k.kid == kid)
    }
}

// ── Cognito JWT claims ──

#[derive(Debug, Deserialize, Serialize)]
struct CognitoClaims {
    sub: String,
    iss: String,
    #[serde(default)]
    email: Option<String>,
    #[serde(default, rename = "cognito:username")]
    username: Option<String>,
    #[serde(default, rename = "cognito:groups")]
    groups: Option<Vec<String>>,
    #[serde(default, rename = "custom:tenant_id")]
    tenant_id: Option<String>,
    #[serde(default)]
    token_use: Option<String>,
    #[serde(default)]
    client_id: Option<String>,
}

// ── JWT validation middleware ──

async fn require_cognito_jwt(
    State(config): State<Arc<CognitoConfig>>,
    mut req: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    let auth = req
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if !auth.starts_with("Bearer ") {
        return Err(StatusCode::UNAUTHORIZED);
    }

    let token = &auth[7..];

    // Decode the JWT header to get the key ID (kid).
    let header = decode_header(token).map_err(|_| StatusCode::UNAUTHORIZED)?;
    let kid = header.kid.ok_or(StatusCode::UNAUTHORIZED)?;

    // Find the matching public key from the cached JWKS.
    let jwk = config.jwks.find_key(&kid).ok_or(StatusCode::UNAUTHORIZED)?;

    // Build the RSA decoding key from the JWK's n and e components.
    let decoding_key =
        DecodingKey::from_rsa_components(&jwk.n, &jwk.e).map_err(|_| StatusCode::UNAUTHORIZED)?;

    // Validate the token: signature, issuer, expiry.
    let mut validation = Validation::new(Algorithm::RS256);
    validation.set_issuer(&[config.issuer()]);
    // Cognito access tokens use client_id, ID tokens use aud.
    // Accept both by setting audience to the client ID.
    validation.set_audience(&[&config.client_id]);
    // Cognito ID tokens put client_id in "aud", access tokens don't have "aud".
    // Disable aud validation and check manually if needed.
    validation.validate_aud = false;

    let token_data = decode::<CognitoClaims>(token, &decoding_key, &validation)
        .map_err(|_| StatusCode::UNAUTHORIZED)?;

    // Convert claims to a Value and store as an extension.
    let claims_value = serde_json::to_value(&token_data.claims).unwrap_or(json!({}));
    req.extensions_mut().insert(claims_value);

    Ok(next.run(req).await)
}

// ── Shared state ──

struct AppState {
    server: Server,
}

// ── Axum handlers ──

async fn handle_mcp(
    State(state): State<Arc<AppState>>,
    Extension(claims): Extension<Value>,
    Json(req): Json<JsonRpcRequest>,
) -> Response {
    // Pass the decoded Cognito claims as context to tool handlers.
    let resp: McpResponse = state.server.handle(req, claims).await;

    if resp.is_notification() {
        return (StatusCode::ACCEPTED, Body::empty()).into_response();
    }

    Json(&resp).into_response()
}

// ── Tool handlers ──

/// A tool that returns the caller's identity from Cognito claims.
struct WhoamiHandler;

#[async_trait]
impl ToolHandler for WhoamiHandler {
    async fn call(&self, _args: Value, context: Value) -> Result<ToolResult, McpError> {
        let sub = context
            .get("sub")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let email = context
            .get("email")
            .and_then(|v| v.as_str())
            .unwrap_or("not provided");
        let username = context
            .get("cognito:username")
            .and_then(|v| v.as_str())
            .unwrap_or("not provided");
        let tenant_id = context
            .get("custom:tenant_id")
            .and_then(|v| v.as_str())
            .unwrap_or("none");
        let groups = context
            .get("cognito:groups")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_else(|| "none".into());

        Ok(text_result(format!(
            "sub: {}\nemail: {}\nusername: {}\ntenant_id: {}\ngroups: {}",
            sub, email, username, tenant_id, groups
        )))
    }
}

// ── Main ──

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    // Read Cognito config from environment.
    let region = std::env::var("COGNITO_REGION").expect("COGNITO_REGION must be set");
    let user_pool_id =
        std::env::var("COGNITO_USER_POOL_ID").expect("COGNITO_USER_POOL_ID must be set");
    let client_id = std::env::var("COGNITO_CLIENT_ID").expect("COGNITO_CLIENT_ID must be set");

    let mut cognito_config = CognitoConfig {
        region,
        user_pool_id,
        client_id,
        jwks: JwkSet { keys: vec![] },
    };

    // Fetch Cognito's JWKS once at startup and cache it.
    let jwks_url = cognito_config.jwks_url();
    println!("Fetching JWKS from {}", jwks_url);
    cognito_config.jwks = reqwest::get(&jwks_url)
        .await
        .expect("failed to fetch JWKS")
        .json::<JwkSet>()
        .await
        .expect("failed to parse JWKS");
    println!(
        "Loaded {} keys from JWKS",
        cognito_config.jwks.keys.len()
    );

    let cognito = Arc::new(cognito_config);

    // Build the MCP server.
    let mut server = Server::builder()
        .tools_json(
            r#"[
                {"name":"whoami","description":"Returns the caller's identity from JWT claims","inputSchema":{"type":"object","properties":{}}},
                {"name":"echo","description":"Echoes the input message","inputSchema":{"type":"object","properties":{"message":{"type":"string"}},"required":["message"]}}
            ]"#
            .as_bytes(),
        )
        .server_info("cognito-example", "0.1.0")
        .build();

    server.handle_tool("whoami", Arc::new(WhoamiHandler));

    server.handle_tool(
        "echo",
        FnToolHandler::new(|args: Value, context: Value| async move {
            let user = context
                .get("cognito:username")
                .and_then(|v| v.as_str())
                .unwrap_or("anonymous");
            let msg = args
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            Ok(text_result(format!("[{}] echo: {}", user, msg)))
        }),
    );

    // Wire up HTTP with Cognito middleware.
    let state = Arc::new(AppState { server });

    let app = Router::new()
        .route("/healthz", get(|| async { Json(json!({"status": "ok"})) }))
        .route(
            "/mcp",
            post(handle_mcp).layer(middleware::from_fn_with_state(
                Arc::clone(&cognito),
                require_cognito_jwt,
            )),
        )
        .with_state(state);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
    println!("MCP server listening on http://localhost:3000");
    println!("  POST /mcp     — MCP endpoint (requires Cognito JWT)");
    println!("  GET  /healthz — health check");
    axum::serve(listener, app).await.unwrap();
}
