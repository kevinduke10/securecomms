#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
container="croaker-dynamodb-test-$$"
cleanup() { docker rm -f "$container" >/dev/null 2>&1 || true; }
trap cleanup EXIT
docker run --rm -d --name "$container" -p 127.0.0.1::8000 amazon/dynamodb-local:latest -jar DynamoDBLocal.jar -inMemory -sharedDb >/dev/null
port="$(docker port "$container" 8000/tcp | awk -F: '{print $NF}')"
export CROAKER_DYNAMODB_ENDPOINT="http://127.0.0.1:$port"
# These are deliberately dummy credentials, used only against the loopback emulator.
export AWS_ACCESS_KEY_ID=croakerlocal
export AWS_SECRET_ACCESS_KEY=croakerlocal
export AWS_DEFAULT_REGION=us-east-1
export AWS_EC2_METADATA_DISABLED=true
ready=false
for _ in $(seq 1 30); do
  if aws dynamodb list-tables --endpoint-url "$CROAKER_DYNAMODB_ENDPOINT" >/dev/null 2>&1; then
    ready=true
    break
  fi
  sleep 1
done
if [[ "$ready" != true ]]; then
  docker logs "$container" >&2
  exit 1
fi
cargo test -p croaker-relay --features aws --test store_contract dynamodb_store_contract -- --ignored --nocapture

cargo test -p croaker-relay --features aws --test enrollment_contract dynamodb_enrollment_contract -- --ignored --nocapture

cargo test -p croaker-relay --features aws --test membership_contract dynamodb_admission_contract -- --ignored --nocapture

cargo test -p croaker-relay --features aws --test lifecycle_contract dynamodb_lifecycle_contract -- --ignored --nocapture

cargo test -p croaker-relay --features aws --test media dynamodb_media_contract -- --ignored --nocapture
