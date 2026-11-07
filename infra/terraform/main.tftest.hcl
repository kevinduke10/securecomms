mock_provider "aws" {}

run "pilot_safety_defaults" {
  command = plan
  variables {
    lambda_zip = "main.tf"
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
