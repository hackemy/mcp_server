# Marketplace MCP Server

A production-grade [Model Context Protocol](https://modelcontextprotocol.io/) (MCP) server built in Rust, implementing the **Streamable HTTP** transport (`2025-03-26` spec). Exposes a marketplace notification platform as MCP tools — deployable as a standalone HTTP server or an AWS Lambda function.

## Architecture

```
mcp_server/
├── config/
│   ├── tools.json              # Tool definitions (15 tools)
│   └── resources.json          # Resource definitions
├── crates/
│   ├── mcpserver/              # Reusable MCP library (protocol, routing, validation)
│   └── marketplace/            # Application binary (tools, auth, DynamoDB, notifications)
├── nginx/
│   └── mcp.conf                # Nginx reverse proxy config
└── scripts/
    └── deploy.sh               # Lambda build & deploy script
```

**Two crates in a Cargo workspace:**

| Crate | Purpose |
|---|---|
| `mcpserver` | Protocol-agnostic MCP library — JSON-RPC routing, tool/resource loading, input validation, Axum HTTP transport |
| `marketplace` | Application layer — JWT auth, DynamoDB storage, SNS/SES notifications, tool handler implementations |

## Prerequisites

- **Rust** 1.75+ (install via [rustup](https://rustup.rs/))
- **AWS CLI** v2 (for deployment and DynamoDB access)
- **cargo-lambda** (optional, for Lambda builds): `cargo install cargo-lambda`
- **Docker** (optional, for cross-compilation fallback)

## Configuration

### Environment Variables

| Variable | Default | Required | Description |
|---|---|---|---|
| `PORT` | `8080` | No | HTTP listen port (local mode only) |
| `TABLE_NAME` | `marketplace` | No | DynamoDB table name |
| `JWT_SECRET` | `""` | Yes | HMAC-SHA256 key for signing JWTs |
| `SES_FROM_EMAIL` | `""` | Yes | Sender email for OTP delivery |
| `VAPID_PUBLIC_KEY` | `""` | No | VAPID public key for web push |
| `VAPID_PRIVATE_KEY` | `""` | No | VAPID private key for web push |
| `AWS_REGION` | — | Yes | AWS region for DynamoDB, SNS, SES |
| `AWS_ACCESS_KEY_ID` | — | Yes* | AWS credentials (not needed if using IAM roles) |
| `AWS_SECRET_ACCESS_KEY` | — | Yes* | AWS credentials (not needed if using IAM roles) |

Create a `.env` file at the project root for local development:

```env
PORT=8080
TABLE_NAME=marketplace
JWT_SECRET=your-secret-key-here
SES_FROM_EMAIL=noreply@example.com
AWS_REGION=us-east-1
AWS_ACCESS_KEY_ID=AKIA...
AWS_SECRET_ACCESS_KEY=...
VAPID_PUBLIC_KEY=...
VAPID_PRIVATE_KEY=...
```

### Tool & Resource Definitions

Tools are defined in `config/tools.json` — each tool specifies its name, description, and JSON Schema for input validation (including `required`, `oneOf`, and `dependencies` constraints). Resources are defined in `config/resources.json`.

To add a new tool:

1. Add its definition to `config/tools.json`
2. Create a handler file in `crates/marketplace/src/tools/`
3. Register it in `crates/marketplace/src/tools/mod.rs` via `register_all()`

### DynamoDB Table

The server uses a single-table design with two Global Secondary Indexes:

| Index | Partition Key | Sort Key |
|---|---|---|
| Main table | `PK` (String) | `SK` (String) |
| GSI1 | `GSI1PK` (String) | `GSI1SK` (String) |
| GSI2 | `GSI2PK` (String) | `GSI2SK` (String) |

Create the table via AWS CLI:

```bash
aws dynamodb create-table \
  --table-name marketplace \
  --attribute-definitions \
    AttributeName=PK,AttributeType=S \
    AttributeName=SK,AttributeType=S \
    AttributeName=GSI1PK,AttributeType=S \
    AttributeName=GSI1SK,AttributeType=S \
    AttributeName=GSI2PK,AttributeType=S \
    AttributeName=GSI2SK,AttributeType=S \
  --key-schema \
    AttributeName=PK,KeyType=HASH \
    AttributeName=SK,KeyType=RANGE \
  --global-secondary-indexes \
    'IndexName=GSI1,KeySchema=[{AttributeName=GSI1PK,KeyType=HASH},{AttributeName=GSI1SK,KeyType=RANGE}],Projection={ProjectionType=ALL}' \
    'IndexName=GSI2,KeySchema=[{AttributeName=GSI2PK,KeyType=HASH},{AttributeName=GSI2SK,KeyType=RANGE}],Projection={ProjectionType=ALL}' \
  --billing-mode PAY_PER_REQUEST \
  --region us-east-1
```

## Running Locally

### Build and Run

```bash
# Build
cargo build

# Run (reads .env if present)
cargo run

# The server listens on http://localhost:8080
```

### Verify It's Running

```bash
# Health check
curl http://localhost:8080/healthz

# Initialize MCP session
curl -X POST http://localhost:8080/mcp \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "id": 1,
    "method": "initialize",
    "params": {}
  }'

# List available tools
curl -X POST http://localhost:8080/mcp \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "id": 2,
    "method": "tools/list",
    "params": {}
  }'
```

### Running Tests

```bash
# Run all tests
cargo test

# Run tests for a specific crate
cargo test -p mcpserver
cargo test -p marketplace

# Run a specific test
cargo test test_otp_verify_success

# Run with output
cargo test -- --nocapture
```

Tests use in-memory mocks for DynamoDB, SNS, and SES — no AWS credentials needed.

## Running with Nginx

Nginx handles TLS termination and proxies to the Rust server. The included config is at `nginx/mcp.conf`.

### 1. Install Nginx

```bash
# macOS
brew install nginx

# Ubuntu/Debian
sudo apt install nginx
```

### 2. Set Up TLS Certificates

For local development, generate self-signed certs:

```bash
sudo mkdir -p /etc/nginx/ssl
sudo openssl req -x509 -nodes -days 365 \
  -newkey rsa:2048 \
  -keyout /etc/nginx/ssl/key.pem \
  -out /etc/nginx/ssl/cert.pem \
  -subj "/CN=mcp.example.com"
```

For production, use [Let's Encrypt](https://letsencrypt.org/) / certbot.

### 3. Configure Nginx

Copy or symlink the provided config:

```bash
# macOS (Homebrew)
cp nginx/mcp.conf /usr/local/etc/nginx/servers/mcp.conf

# Linux
sudo cp nginx/mcp.conf /etc/nginx/sites-available/mcp.conf
sudo ln -s /etc/nginx/sites-available/mcp.conf /etc/nginx/sites-enabled/
```

Edit `nginx/mcp.conf` to set your `server_name` and SSL certificate paths. Key settings already configured:

- `proxy_buffering off` — required for streaming responses
- `proxy_http_version 1.1` — required for keep-alive to the upstream
- `proxy_read_timeout 300s` — long timeout for streaming

### 4. Start Both Services

```bash
# Start the MCP server
cargo run &

# Test nginx config and start
sudo nginx -t
sudo nginx

# Verify
curl https://mcp.example.com/healthz --insecure
```

## Deploying to AWS Lambda

### Prerequisites

1. **AWS Account** with permissions for Lambda, S3, DynamoDB, SNS, SES, IAM, and CloudWatch Logs
2. **AWS CLI** configured with credentials:
   ```bash
   aws configure
   # Enter: Access Key ID, Secret Access Key, Region, Output format
   ```
3. **cargo-lambda** installed:
   ```bash
   cargo install cargo-lambda
   ```

### Required AWS Resources

Before deploying, create the following:

#### IAM Role

Create a Lambda execution role with these policies:

- `AWSLambdaBasicExecutionRole` (CloudWatch Logs)
- Custom policy for DynamoDB, SNS, SES access:

```json
{
  "Version": "2012-10-17",
  "Statement": [
    {
      "Effect": "Allow",
      "Action": [
        "dynamodb:GetItem",
        "dynamodb:PutItem",
        "dynamodb:DeleteItem",
        "dynamodb:Query",
        "dynamodb:BatchWriteItem"
      ],
      "Resource": [
        "arn:aws:dynamodb:*:*:table/marketplace",
        "arn:aws:dynamodb:*:*:table/marketplace/index/*"
      ]
    },
    {
      "Effect": "Allow",
      "Action": ["sns:Publish"],
      "Resource": "*"
    },
    {
      "Effect": "Allow",
      "Action": ["ses:SendEmail"],
      "Resource": "*"
    }
  ]
}
```

#### S3 Bucket

Create a bucket for Lambda deployment artifacts:

```bash
aws s3 mb s3://your-lambda-deploy-bucket --region us-east-1
```

#### Lambda Function

Create the function (first-time only):

```bash
aws lambda create-function \
  --function-name marketplace-mcp \
  --runtime provided.al2023 \
  --handler bootstrap \
  --architectures arm64 \
  --role arn:aws:iam::ACCOUNT_ID:role/your-lambda-role \
  --environment "Variables={TABLE_NAME=marketplace,JWT_SECRET=your-secret,SES_FROM_EMAIL=noreply@example.com,AWS_REGION=us-east-1}" \
  --timeout 30 \
  --memory-size 256 \
  --region us-east-1 \
  --zip-file fileb://lambda.zip
```

#### Lambda Function URL (for HTTP access)

```bash
aws lambda create-function-url-config \
  --function-name marketplace-mcp \
  --auth-type NONE

# Or with IAM auth:
aws lambda create-function-url-config \
  --function-name marketplace-mcp \
  --auth-type AWS_IAM
```

### Deploy

Set the required environment variables (or add them to `.env`):

```bash
export AWS_ACCESS_KEY_ID=AKIA...
export AWS_SECRET_ACCESS_KEY=...
export AWS_REGION=us-east-1
export AWS_ACCOUNT_ID=123456789012
export LAMBDA_FUNCTION_NAME=marketplace-mcp
export LAMBDA_S3_BUCKET=your-lambda-deploy-bucket
export LAMBDA_S3_KEY=marketplace-mcp/lambda.zip
```

Run the deploy script:

```bash
# Build the Lambda binary and package it
./scripts/deploy.sh build

# Deploy to AWS
./scripts/deploy.sh deploy

# Clean up build artifacts
./scripts/deploy.sh clean
```

The build step compiles for `aarch64` (ARM64/Graviton), packages the `bootstrap` binary and `config/` directory into `lambda.zip`, then uploads to S3 and updates the Lambda function code.

### Updating Environment Variables

```bash
aws lambda update-function-configuration \
  --function-name marketplace-mcp \
  --environment "Variables={TABLE_NAME=marketplace,JWT_SECRET=new-secret,SES_FROM_EMAIL=noreply@example.com}"
```

## API Reference

### Endpoints

| Method | Path | Description |
|---|---|---|
| `POST` | `/mcp` | MCP JSON-RPC endpoint |
| `GET` | `/healthz` | Health check |

### MCP Methods

| Method | Description |
|---|---|
| `initialize` | Handshake, returns server capabilities and session ID |
| `ping` | Keepalive |
| `tools/list` | List all available tools |
| `tools/call` | Execute a tool by name |
| `resources/list` | List all available resources |
| `resources/read` | Read a resource by name or URI |
| `notifications/initialized` | Client notification (returns 202) |
| `notifications/cancelled` | Client notification (returns 202) |

### Available Tools

| Tool | Auth | Description |
|---|---|---|
| `otp-request` | No | Send OTP via phone or email |
| `otp-verify` | No | Verify OTP, receive JWT |
| `channel-put` | Yes | Create or update a channel |
| `channel-delete` | Yes | Delete an owned channel |
| `channels-list` | Yes | List your channels |
| `channels-for-category` | Yes | List channels by category |
| `channel-notify` | Yes | Send a message to a channel |
| `channel-messages` | Yes | List messages in a channel |
| `channel-subscribe` | Yes | Subscribe to a channel |
| `channel-unsubscribe` | Yes | Unsubscribe from a channel |
| `subscriptions-list` | Yes | List your subscriptions |
| `web-push-enable` | Yes | Enable browser push notifications |
| `web-push-disable` | Yes | Disable browser push notifications |
| `account-delete` | Yes | Delete your account and all data |

### Example: Full Authentication Flow

```bash
# 1. Request OTP
curl -X POST http://localhost:8080/mcp \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "id": 1,
    "method": "tools/call",
    "params": {
      "name": "otp-request",
      "arguments": {"email": "user@example.com"}
    }
  }'

# 2. Verify OTP (returns JWT)
curl -X POST http://localhost:8080/mcp \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "id": 2,
    "method": "tools/call",
    "params": {
      "name": "otp-verify",
      "arguments": {"email": "user@example.com", "code": "123456"}
    }
  }'

# 3. Use the JWT token for authenticated calls
curl -X POST http://localhost:8080/mcp \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "id": 3,
    "method": "tools/call",
    "params": {
      "name": "channels-list",
      "arguments": {"token": "eyJhbGciOiJIUzI1NiIs..."}
    }
  }'
```

## License

Private — All rights reserved.
