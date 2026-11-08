variable "login_callback_urls" {
  type        = list(string)
  default     = ["http://localhost:3000/local-auth/callback"]
  description = "Exact client callback URLs. Production callbacks must use HTTPS."
  validation {
    condition     = length(var.login_callback_urls) > 0 && alltrue([for url in var.login_callback_urls : can(regex("^https://[^?#]+$|^http://localhost:[0-9]+/local-auth/callback$", url))])
    error_message = "Use HTTPS callbacks, or localhost callbacks for local development."
  }
}
variable "login_logout_urls" {
  type    = list(string)
  default = ["http://localhost:3000"]
  validation {
    condition     = length(var.login_logout_urls) > 0 && alltrue([for url in var.login_logout_urls : can(regex("^https://[^?#]+$|^http://localhost:[0-9]+$", url))])
    error_message = "Use HTTPS logout URLs, or localhost origins for development."
  }
}
resource "aws_cognito_user_pool" "accounts" {
  name                     = "${var.deployment_name}-accounts"
  deletion_protection      = "ACTIVE"
  username_attributes      = ["email"]
  auto_verified_attributes = ["email"]
  username_configuration { case_sensitive = false }
  admin_create_user_config { allow_admin_create_user_only = true }
  mfa_configuration = "ON"
  software_token_mfa_configuration { enabled = true }
  password_policy {
    minimum_length                   = 14
    require_lowercase                = true
    require_uppercase                = true
    require_numbers                  = true
    require_symbols                  = true
    temporary_password_validity_days = 3
  }
  account_recovery_setting {
    recovery_mechanism {
      name     = "verified_email"
      priority = 1
    }
  }
}
resource "aws_cognito_resource_server" "croaker" {
  identifier   = "croaker"
  name         = "Cirra account API"
  user_pool_id = aws_cognito_user_pool.accounts.id
  scope {
    scope_name        = "access"
    scope_description = "Request account and device enrollment operations"
  }
}
resource "aws_cognito_user_pool_client" "app" {
  name                                 = "${var.deployment_name}-client"
  user_pool_id                         = aws_cognito_user_pool.accounts.id
  generate_secret                      = false
  allowed_oauth_flows_user_pool_client = true
  allowed_oauth_flows                  = ["code"]
  allowed_oauth_scopes                 = ["openid", "croaker/access"]
  supported_identity_providers         = ["COGNITO"]
  callback_urls                        = var.login_callback_urls
  logout_urls                          = var.login_logout_urls
  access_token_validity                = 5
  id_token_validity                    = 5
  refresh_token_validity               = 1
  token_validity_units {
    access_token  = "minutes"
    id_token      = "minutes"
    refresh_token = "days"
  }
  enable_token_revocation       = true
  prevent_user_existence_errors = "ENABLED"
  depends_on                    = [aws_cognito_resource_server.croaker]
}
resource "aws_cognito_user_pool_domain" "login" {
  domain       = "${var.deployment_name}-${data.aws_caller_identity.current.account_id}"
  user_pool_id = aws_cognito_user_pool.accounts.id
}
output "cognito_issuer" { value = "https://${aws_cognito_user_pool.accounts.endpoint}" }
output "cognito_client_id" { value = aws_cognito_user_pool_client.app.id }
output "cognito_domain" { value = "https://${aws_cognito_user_pool_domain.login.domain}.auth.${var.region}.amazoncognito.com" }
output "cognito_user_pool_id" { value = aws_cognito_user_pool.accounts.id }
