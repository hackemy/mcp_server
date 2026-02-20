#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")"/.. && pwd)"
BIN_DIR="$ROOT_DIR/bin"
ZIP_PATH="$ROOT_DIR/lambda.zip"
GO_BUILD=${GO_BUILD:-"$(command -v go)"}

usage() {
  cat <<'EOF'
Usage: scripts/deploy.sh <command>

Commands:
  build    Build arm64 bootstrap binary and lambda.zip
  deploy   Build (if needed), upload to S3, and update Lambda code
  clean    Remove build artifacts
  help     Show this help
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
  mkdir -p "$BIN_DIR"
  local pkg_dir="$BIN_DIR/package"
  rm -rf "$pkg_dir"
  mkdir -p "$pkg_dir"

  echo "Building Lambda bootstrap (arm64, AL2023)..."
  GOOS=linux GOARCH=arm64 CGO_ENABLED=0 "$GO_BUILD" build \
    -tags lambda.norpc \
    -o "$pkg_dir/bootstrap" ./cmd/lambda

  if [[ -d "$ROOT_DIR/config" ]]; then
    echo "Copying config/ into package..."
    rsync -a "$ROOT_DIR/config" "$pkg_dir/"
  fi

  echo "Packaging lambda.zip..."
  (cd "$pkg_dir" && zip -X -r "$ZIP_PATH" . >/dev/null)
  echo "Build complete: $ZIP_PATH"
}

clean() {
  rm -rf "$BIN_DIR" "$ZIP_PATH"
  echo "Artifacts removed"
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
