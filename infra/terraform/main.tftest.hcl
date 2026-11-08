mock_provider "aws" {}

run "pilot_safety_defaults" {
  command = plan
  variables {
    lambda_zip = "main.tf"
    region     = "us-east-2"
  }
  assert {
    condition     = aws_cloudwatch_log_group.api.region == var.region && aws_cloudwatch_log_group.lambda.region == var.region
    error_message = "Operational log groups must follow the selected deployment region."
  }
  assert {
    condition     = aws_dynamodb_table.relay.deletion_protection_enabled && aws_dynamodb_table.relay.point_in_time_recovery[0].enabled
    error_message = "Durable relay data must have deletion protection and point-in-time recovery."
  }
  assert {
    condition     = aws_lambda_function.relay.reserved_concurrent_executions == 10 && aws_lambda_function.relay.memory_size == 256
    error_message = "Keep pilot concurrency and memory bounded."
  }
  assert {
    condition     = aws_budgets_budget.pilot.limit_amount == "200"
    error_message = "The pilot budget defaults to $200."
  }
  assert {
    condition     = aws_cloudwatch_log_group.api.retention_in_days == 14 && aws_cloudwatch_log_group.lambda.retention_in_days == 14
    error_message = "Keep operational logs on a bounded retention schedule."
  }
}

run "account_login_requires_mfa_and_code_flow" {
  command = plan
  variables { lambda_zip = "main.tf" }
  assert {
    condition     = aws_cognito_user_pool.accounts.mfa_configuration == "ON" && aws_cognito_user_pool.accounts.admin_create_user_config[0].allow_admin_create_user_only
    error_message = "Accounts must be invited and require MFA."
  }
  assert {
    condition     = !aws_cognito_user_pool_client.app.generate_secret && length(aws_cognito_user_pool_client.app.allowed_oauth_flows) == 1 && contains(aws_cognito_user_pool_client.app.allowed_oauth_flows, "code") && aws_cognito_user_pool_client.app.access_token_validity == 5
    error_message = "Use a public code-flow client with short-lived access tokens."
  }
}

run "encrypted_media_storage_is_private_and_bounded" {
  command = plan
  variables { lambda_zip = "main.tf" }
  assert {
    condition     = aws_s3_bucket_public_access_block.media.block_public_acls && aws_s3_bucket_public_access_block.media.block_public_policy && aws_s3_bucket_public_access_block.media.ignore_public_acls && aws_s3_bucket_public_access_block.media.restrict_public_buckets
    error_message = "Encrypted media buckets must block every form of public access."
  }
  assert {
    condition     = !aws_s3_bucket.media.force_destroy && contains(keys(aws_lambda_function.relay.environment[0].variables), "CROAKER_MEDIA_BUCKET")
    error_message = "Media storage must preserve data on accidental destroy and be wired to the relay."
  }
  assert {
    condition     = one(aws_s3_bucket_lifecycle_configuration.media.rule).expiration[0].days == 30
    error_message = "Ciphertext object retention must remain bounded."
  }
  assert {
    condition     = one(aws_s3_bucket_server_side_encryption_configuration.media.rule).apply_server_side_encryption_by_default[0].sse_algorithm == "AES256"
    error_message = "S3 encryption at rest must be configured in addition to client encryption."
  }
}

run "browser_hosting_preserves_private_origins_and_native_login" {
  command = plan
  variables { lambda_zip = "main.tf" }
  assert {
    condition     = aws_s3_bucket_public_access_block.web[0].block_public_policy && aws_s3_bucket_public_access_block.web[0].block_public_acls && aws_cloudfront_origin_access_control.web[0].signing_behavior == "always"
    error_message = "The web bucket must remain private and require signed CloudFront access."
  }
  assert {
    condition     = contains(aws_cognito_user_pool_client.app.callback_urls, "app.cirra.ios:/oauth/callback") && contains(aws_cognito_user_pool_client.app.callback_urls, "http://localhost:38317/callback")
    error_message = "Adding web login must preserve native callbacks."
  }
  assert {
    condition     = aws_cloudfront_distribution.web[0].ordered_cache_behavior[0].cache_policy_id == "4135ea2d-6df8-44a3-9df3-4b5a84be39ad"
    error_message = "Authenticated relay traffic must never be cached by the web distribution."
  }
}
