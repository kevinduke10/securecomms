#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
if ! cargo lambda --version >/dev/null 2>&1; then
  echo 'Install Cargo Lambda first: cargo install cargo-lambda --locked' >&2
  exit 1
fi
cargo lambda build --release --arm64 --output-format zip --locked -p croaker-relay --no-default-features --features aws --bin croaker-lambda
mkdir -p artifacts
cp target/lambda/croaker-lambda/bootstrap.zip artifacts/croaker-lambda.zip
echo "Lambda ZIP: $(pwd)/artifacts/croaker-lambda.zip"
