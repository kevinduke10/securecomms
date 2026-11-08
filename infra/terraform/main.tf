terraform {
  required_version = ">= 1.10, < 2.0"
  required_providers {
    aws = { source = "hashicorp/aws", version = "~> 6.0" }
  }
}

provider "aws" {
  region = var.region
  default_tags {
    tags = { Application = "croaker", Deployment = var.deployment_name, ManagedBy = "terraform" }
  }
}

variable "region" {
  type    = string
  default = "us-east-2"
}
variable "deployment_name" {
  type    = string
  default = "croaker-dev"
  validation {
    condition     = can(regex("^[a-z][a-z0-9-]{2,30}$", var.deployment_name))
    error_message = "Use 3–31 lowercase letters, numbers, and hyphens."
  }
}
variable "lambda_zip" {
  type        = string
  description = "Absolute path to the ARM64 Lambda bootstrap ZIP produced by scripts/build-lambda.sh."
}
variable "monthly_budget_usd" {
  type    = number
  default = 200
  validation {
    condition     = var.monthly_budget_usd > 0
    error_message = "Budget must be positive."
  }
}
variable "budget_email" {
  type        = string
  default     = null
  description = "Optional recipient for AWS budget notifications. Budgets alert; they do not cap spending."
}
variable "reserved_concurrency" {
  type    = number
  default = 10
}

data "aws_caller_identity" "current" {}
data "aws_partition" "current" {}

resource "aws_dynamodb_table" "relay" {
  name                        = "${var.deployment_name}-relay"
  billing_mode                = "PAY_PER_REQUEST"
  hash_key                    = "pk"
  range_key                   = "sk"
  deletion_protection_enabled = true
  attribute {
    name = "pk"
    type = "S"
  }
  attribute {
    name = "sk"
    type = "S"
  }
  ttl {
    attribute_name = "expires_at"
    enabled        = true
  }
  point_in_time_recovery { enabled = true }
  server_side_encryption { enabled = true }
}

resource "aws_cloudwatch_log_group" "lambda" {
  region            = var.region
  name              = "/aws/lambda/${var.deployment_name}-relay"
  retention_in_days = 14
}
resource "aws_cloudwatch_log_group" "api" {
  region            = var.region
  name              = "/croaker/${var.deployment_name}/http"
  retention_in_days = 14
}

resource "aws_iam_role" "relay" {
  name = "${var.deployment_name}-relay"
  assume_role_policy = jsonencode({
    Version   = "2012-10-17"
    Statement = [{ Effect = "Allow", Principal = { Service = "lambda.amazonaws.com" }, Action = "sts:AssumeRole" }]
  })
}
resource "aws_iam_role_policy" "relay" {
  role = aws_iam_role.relay.id
  policy = jsonencode({
    Version = "2012-10-17"
    Statement = [
      {
        Effect   = "Allow"
        Action   = ["dynamodb:GetItem", "dynamodb:BatchGetItem", "dynamodb:Query", "dynamodb:PutItem", "dynamodb:UpdateItem", "dynamodb:ConditionCheckItem"]
        Resource = aws_dynamodb_table.relay.arn
      },
      {
        Effect   = "Allow"
        Action   = ["logs:CreateLogStream", "logs:PutLogEvents"]
        Resource = "${aws_cloudwatch_log_group.lambda.arn}:*"
      }
    ]
  })
}

resource "aws_lambda_function" "relay" {
  function_name                  = "${var.deployment_name}-relay"
  role                           = aws_iam_role.relay.arn
  runtime                        = "provided.al2023"
  architectures                  = ["arm64"]
  handler                        = "bootstrap"
  filename                       = var.lambda_zip
  source_code_hash               = filebase64sha256(var.lambda_zip)
  memory_size                    = 256
  timeout                        = 20
  reserved_concurrent_executions = var.reserved_concurrency
  environment { variables = {
    CROAKER_TABLE             = aws_dynamodb_table.relay.name
    CROAKER_MEDIA_BUCKET      = aws_s3_bucket.media.id
    CROAKER_COGNITO_ISSUER    = "https://${aws_cognito_user_pool.accounts.endpoint}"
    CROAKER_COGNITO_CLIENT_ID = aws_cognito_user_pool_client.app.id
    CROAKER_COGNITO_DOMAIN    = "https://${aws_cognito_user_pool_domain.login.domain}.auth.${var.region}.amazoncognito.com"
  } }
  depends_on = [aws_iam_role_policy.relay, aws_cloudwatch_log_group.lambda]
}

resource "aws_apigatewayv2_api" "relay" {
  name          = var.deployment_name
  protocol_type = "HTTP"
}
resource "aws_apigatewayv2_integration" "relay" {
  api_id                 = aws_apigatewayv2_api.relay.id
  integration_type       = "AWS_PROXY"
  integration_uri        = aws_lambda_function.relay.invoke_arn
  payload_format_version = "2.0"
  timeout_milliseconds   = 21000
}
resource "aws_apigatewayv2_route" "relay" {
  api_id    = aws_apigatewayv2_api.relay.id
  route_key = "$default"
  target    = "integrations/${aws_apigatewayv2_integration.relay.id}"
}
resource "aws_apigatewayv2_stage" "relay" {
  api_id      = aws_apigatewayv2_api.relay.id
  name        = "$default"
  auto_deploy = true
  default_route_settings {
    throttling_rate_limit  = 25
    throttling_burst_limit = 50
  }
  access_log_settings {
    destination_arn = aws_cloudwatch_log_group.api.arn
    # Deliberately exclude URLs, query strings, credentials, and payloads.
    format = jsonencode({ requestId = "$context.requestId", status = "$context.status", responseLength = "$context.responseLength" })
  }
}
resource "aws_lambda_permission" "gateway" {
  statement_id  = "AllowApiGateway"
  action        = "lambda:InvokeFunction"
  function_name = aws_lambda_function.relay.function_name
  principal     = "apigateway.amazonaws.com"
  source_arn    = "${aws_apigatewayv2_api.relay.execution_arn}/*/*"
}

resource "aws_cloudwatch_metric_alarm" "errors" {
  alarm_name          = "${var.deployment_name}-relay-errors"
  namespace           = "AWS/Lambda"
  metric_name         = "Errors"
  statistic           = "Sum"
  period              = 300
  evaluation_periods  = 1
  threshold           = 5
  comparison_operator = "GreaterThanOrEqualToThreshold"
  treat_missing_data  = "notBreaching"
  dimensions          = { FunctionName = aws_lambda_function.relay.function_name }
}

resource "aws_budgets_budget" "pilot" {
  name         = "${var.deployment_name}-monthly"
  budget_type  = "COST"
  limit_amount = tostring(var.monthly_budget_usd)
  limit_unit   = "USD"
  time_unit    = "MONTHLY"
  # Account-wide on purpose: captures untagged transfer and ancillary charges.
  dynamic "notification" {
    for_each = var.budget_email == null ? [] : [50, 80, 100]
    content {
      comparison_operator        = "GREATER_THAN"
      threshold                  = notification.value
      threshold_type             = "PERCENTAGE"
      notification_type          = "ACTUAL"
      subscriber_email_addresses = [var.budget_email]
    }
  }
}

output "api_url" { value = aws_apigatewayv2_api.relay.api_endpoint }
output "table_name" { value = aws_dynamodb_table.relay.name }
