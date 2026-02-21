use std::sync::Arc;

use lambda_http::{Request, Response, Body, Error, service_fn};
use mcpserver::{JsonRpcRequest, Server, new_error_response};

/// Run the Lambda handler loop.
pub async fn run(srv: Server) {
    let srv = Arc::new(srv);
    let func = service_fn(move |event: Request| {
        let srv = srv.clone();
        async move { handle(event, &srv).await }
    });
    lambda_http::run(func).await.unwrap();
}

async fn handle(event: Request, srv: &Server) -> Result<Response<Body>, Error> {
    let method = event.method().as_str().to_uppercase();
    let path = event.uri().path();

    match (method.as_str(), path) {
        ("GET", "/healthz") => {
            Ok(Response::builder()
                .status(200)
                .header("content-type", "application/json")
                .body(Body::Text(r#"{"status":"ok"}"#.into()))
                .unwrap())
        }
        ("POST", "/mcp") => handle_jsonrpc(event, srv).await,
        _ => {
            Ok(Response::builder()
                .status(404)
                .header("content-type", "application/json")
                .body(Body::Text(r#"{"error":"route_not_found"}"#.into()))
                .unwrap())
        }
    }
}

async fn handle_jsonrpc(event: Request, srv: &Server) -> Result<Response<Body>, Error> {
    let body = match event.body() {
        Body::Text(s) => s.clone(),
        Body::Binary(b) => String::from_utf8_lossy(b).into_owned(),
        Body::Empty => String::new(),
    };

    let rpc_req: JsonRpcRequest = match serde_json::from_str(&body) {
        Ok(r) => r,
        Err(e) => {
            let resp = new_error_response(
                None,
                -32700, // parse error
                format!("invalid JSON: {}", e),
            );
            let json = serde_json::to_string(&resp).unwrap();
            return Ok(Response::builder()
                .status(400)
                .header("content-type", "application/json")
                .body(Body::Text(json))
                .unwrap());
        }
    };

    // Notifications produce no response body.
    if rpc_req.method.starts_with("notifications/") {
        return Ok(Response::builder()
            .status(202)
            .body(Body::Empty)
            .unwrap());
    }

    let resp = srv.handle(rpc_req).await;

    // Check if the response is a notification sentinel.
    if resp.id.is_none() && resp.result.is_none() && resp.error.is_none() {
        return Ok(Response::builder()
            .status(202)
            .body(Body::Empty)
            .unwrap());
    }

    let json = serde_json::to_string(&resp).unwrap();
    Ok(Response::builder()
        .status(200)
        .header("content-type", "application/json")
        .body(Body::Text(json))
        .unwrap())
}
