mod auth;
mod dynamo;
mod notify;
mod tools;

use std::sync::Arc;

use dynamo::{DynamoClient, DynamoApi};
use notify::sns::{SnsClient, SnsApi};
use notify::ses::{SesClient, SesApi};
use notify::webpush::WebPushKeys;
use tools::Deps;

async fn build_deps() -> Result<Arc<Deps>, Box<dyn std::error::Error>> {
    let table_name = std::env::var("TABLE_NAME").unwrap_or_else(|_| "app".into());
    let db = DynamoClient::new(&table_name).await?;

    let aws_config = aws_config::load_defaults(aws_config::BehaviorVersion::latest()).await;
    let sns = SnsClient::new(&aws_config);
    let ses = SesClient::new(&aws_config);

    Ok(Arc::new(Deps {
        db: Arc::new(db) as Arc<dyn DynamoApi>,
        jwt_secret: std::env::var("JWT_SECRET").unwrap_or_default(),
        sns: Arc::new(sns) as Arc<dyn SnsApi>,
        ses: Arc::new(ses) as Arc<dyn SesApi>,
        ses_from_email: std::env::var("SES_FROM_EMAIL").unwrap_or_default(),
        web_push_keys: WebPushKeys {
            vapid_public_key: std::env::var("VAPID_PUBLIC_KEY").unwrap_or_default(),
            vapid_private_key: std::env::var("VAPID_PRIVATE_KEY").unwrap_or_default(),
        },
    }))
}

fn build_server(deps: Arc<Deps>) -> mcpserver::Server {
    let mut srv = mcpserver::Server::builder()
        .tools_json(include_bytes!("../tools.json"))
        .resources_json(include_bytes!("../resources.json"))
        .server_info("app-mcp", "1.0.0")
        .build();

    tools::register_all(&mut srv, deps);
    srv
}

#[cfg(not(feature = "lambda"))]
#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .json()
        .init();

    let deps = build_deps().await.expect("failed to build dependencies");
    let srv = build_server(deps);

    let port = std::env::var("PORT").unwrap_or_else(|_| "8080".into());
    let addr = format!("0.0.0.0:{}", port);

    tracing::info!(addr = %addr, "starting MCP server");

    let router = mcpserver::http_router(srv);
    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
    axum::serve(listener, router).await.unwrap();
}

#[cfg(feature = "lambda")]
mod lambda;

#[cfg(feature = "lambda")]
#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .json()
        .with_target(false)
        .init();

    let deps = build_deps().await.expect("failed to build dependencies");
    let srv = build_server(deps);

    tracing::info!("starting Lambda handler");
    lambda::run(srv).await;
}
