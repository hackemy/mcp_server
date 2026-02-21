#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")"/.. && pwd)"
BUILD_DIR="$ROOT_DIR/target"
ZIP_PATH="$ROOT_DIR/lambda.zip"

usage() {
  cat <<'EOF'
Usage: scripts/deploy.sh <command>

Commands:
  build    Build arm64 bootstrap binary and lambda.zip
  deploy   Build (if needed), upload to S3, and update Lambda code
  clean    Remove build artifacts
  help     Show this help

Prerequisites:
  - cargo-lambda: cargo install cargo-lambda
  - AWS CLI: aws configure
EOF
}

require_env() {
  local missing=()
  for var in "$@"; do
    if [[ -z "${!var:-}" ]]; then
      missing+=("$var")
    fi
  done
  if [[ ${#missing[@]} -gt 0 ]]; then
    echo "Missing environment variables: ${missing[*]}" >&2
    exit 1
  fi
}

load_env() {
  if [[ -f "$ROOT_DIR/.env" ]]; then
    # shellcheck disable=SC1090
    source "$ROOT_DIR/.env"
  fi
}

build() {
  echo "Building Lambda bootstrap (arm64, AL2023)..."

  # Use cargo-lambda for cross-compiling to Lambda's ARM64 runtime.
  # If cargo-lambda is available, use it; otherwise fall back to standard cross-compile.
  if command -v cargo-lambda &>/dev/null; then
    cargo lambda build \
      --release \
      --arm64 \
      --features lambda \
      --package app \
      --manifest-path "$ROOT_DIR/Cargo.toml"

    local bootstrap_path="$BUILD_DIR/lambda/app/bootstrap"
  else
    echo "cargo-lambda not found, using standard cross-compile..."
    echo "Install with: cargo install cargo-lambda"
    echo "Or: pip3 install cargo-lambda"

    # Standard cross-compile (requires aarch64 target)
    rustup target add aarch64-unknown-linux-musl 2>/dev/null || true
    cargo build \
      --release \
      --target aarch64-unknown-linux-musl \
      --features lambda \
      --package app \
      --manifest-path "$ROOT_DIR/Cargo.toml"

    local bootstrap_path="$BUILD_DIR/aarch64-unknown-linux-musl/release/app"
  fi

  local pkg_dir="$BUILD_DIR/package"
  rm -rf "$pkg_dir"
  mkdir -p "$pkg_dir"

  cp "$bootstrap_path" "$pkg_dir/bootstrap" 2>/dev/null || {
    echo "Binary not found at $bootstrap_path"
    echo "Checking for binary..."
    find "$BUILD_DIR" -name "app" -type f 2>/dev/null | head -5
    exit 1
  }

  echo "Packaging lambda.zip..."
  (cd "$pkg_dir" && zip -X -r "$ZIP_PATH" . >/dev/null)
  echo "Build complete: $ZIP_PATH"
}

clean() {
  rm -rf "$BUILD_DIR/package" "$ZIP_PATH"
  echo "Packaging artifacts removed"
  echo "Run 'cargo clean' to remove all build artifacts"
}

deploy() {
  load_env
  require_env AWS_ACCESS_KEY_ID AWS_SECRET_ACCESS_KEY AWS_REGION AWS_ACCOUNT_ID \
    LAMBDA_FUNCTION_NAME LAMBDA_S3_BUCKET LAMBDA_S3_KEY

  if [[ ! -f "$ZIP_PATH" ]]; then
    build
  fi

  echo "Uploading lambda.zip to s3://$LAMBDA_S3_BUCKET/$LAMBDA_S3_KEY"
  aws s3 cp "$ZIP_PATH" "s3://$LAMBDA_S3_BUCKET/$LAMBDA_S3_KEY" \
    --region "$AWS_REGION" --only-show-errors

  echo "Updating Lambda function $LAMBDA_FUNCTION_NAME..."
  aws lambda update-function-code \
    --function-name "$LAMBDA_FUNCTION_NAME" \
    --s3-bucket "$LAMBDA_S3_BUCKET" \
    --s3-key "$LAMBDA_S3_KEY" \
    --publish \
    --region "$AWS_REGION" >/dev/null

  echo "Deployment complete"
}

main() {
  local cmd=${1:-help}
  case "$cmd" in
    build) build ;;
    deploy) deploy ;;
    clean) clean ;;
    help|--help|-h) usage ;;
    *) echo "Unknown command: $cmd" >&2; usage; exit 1 ;;
  esac
}

main "$@"
