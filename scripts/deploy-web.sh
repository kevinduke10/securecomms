#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
export PATH="$HOME/.cargo/bin:$PATH"
aws_cli="${CIRRA_AWS_CLI:-$HOME/.local/bin/aws}"
[[ -x "$aws_cli" ]] || aws_cli="$(command -v aws)"
profile="${CIRRA_AWS_PROFILE:-cirra-deploy}"
region="${CIRRA_AWS_REGION:-us-east-2}"
web_url="$(terraform -chdir=infra/terraform output -raw web_url)"
bucket="$(terraform -chdir=infra/terraform output -raw web_bucket_name)"
[[ "$web_url" == https://*.cloudfront.net && "$bucket" != null ]] || { echo 'Apply Terraform with enable_web_ui=true first.' >&2; exit 1; }
VITE_CIRRA_RELAY="$web_url" bash scripts/build-browser.sh
"$aws_cli" --profile "$profile" --region "$region" s3 sync target/web-dist/assets/ "s3://$bucket/assets/" --cache-control 'public,max-age=31536000,immutable' --only-show-errors
"$aws_cli" --profile "$profile" --region "$region" s3 sync target/web-dist/ "s3://$bucket/" --exclude 'assets/*' --exclude 'index.html' --cache-control 'public,max-age=3600' --only-show-errors
"$aws_cli" --profile "$profile" --region "$region" s3 cp target/web-dist/index.html "s3://$bucket/index.html" --content-type 'text/html; charset=utf-8' --cache-control 'no-store' --only-show-errors
# Retain previous hashed assets so existing open clients can finish safely.
printf 'Published Cirra: %s\n' "$web_url"
