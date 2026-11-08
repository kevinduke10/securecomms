variable "enable_web_ui" {
  type    = bool
  default = true
}

resource "aws_s3_bucket" "web" {
  count         = var.enable_web_ui ? 1 : 0
  bucket        = "${var.deployment_name}-${data.aws_caller_identity.current.account_id}-web"
  force_destroy = false
}
resource "aws_s3_bucket_public_access_block" "web" {
  count                   = var.enable_web_ui ? 1 : 0
  bucket                  = aws_s3_bucket.web[0].id
  block_public_acls       = true
  block_public_policy     = true
  ignore_public_acls      = true
  restrict_public_buckets = true
}
resource "aws_s3_bucket_server_side_encryption_configuration" "web" {
  count  = var.enable_web_ui ? 1 : 0
  bucket = aws_s3_bucket.web[0].id
  rule {
    apply_server_side_encryption_by_default { sse_algorithm = "AES256" }
  }
}
resource "aws_cloudfront_origin_access_control" "web" {
  count                             = var.enable_web_ui ? 1 : 0
  name                              = "${var.deployment_name}-web"
  origin_access_control_origin_type = "s3"
  signing_behavior                  = "always"
  signing_protocol                  = "sigv4"
}
resource "aws_cloudfront_function" "web_routes" {
  count   = var.enable_web_ui ? 1 : 0
  name    = "${var.deployment_name}-web-routes"
  runtime = "cloudfront-js-2.0"
  publish = true
  code    = <<-JS
    function handler(event) {
      var request = event.request;
      if (request.uri === '/' || request.uri === '/auth/callback') {
        request.uri = '/index.html';
      }
      return request;
    }
  JS
}
resource "aws_cloudfront_response_headers_policy" "web" {
  count = var.enable_web_ui ? 1 : 0
  name  = "${var.deployment_name}-web-security"
  security_headers_config {
    content_security_policy {
      content_security_policy = "default-src 'none'; script-src 'self' 'wasm-unsafe-eval'; worker-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; font-src 'self'; connect-src 'self' ${aws_apigatewayv2_api.relay.api_endpoint} https://${var.deployment_name}-${data.aws_caller_identity.current.account_id}.auth.${var.region}.amazoncognito.com https://${aws_s3_bucket.media.bucket_regional_domain_name}; object-src 'none'; base-uri 'none'; frame-ancestors 'none'; form-action 'none'"
      override                = true
    }
    content_type_options { override = true }
    frame_options {
      frame_option = "DENY"
      override     = true
    }
    referrer_policy {
      referrer_policy = "no-referrer"
      override        = true
    }
    strict_transport_security {
      access_control_max_age_sec = 31536000
      include_subdomains         = true
      override                   = true
    }
  }
  custom_headers_config {
    items {
      header   = "Permissions-Policy"
      value    = "camera=(), microphone=(), geolocation=()"
      override = true
    }
  }
}
resource "aws_cloudfront_distribution" "web" {
  count               = var.enable_web_ui ? 1 : 0
  enabled             = true
  is_ipv6_enabled     = true
  default_root_object = "index.html"
  price_class         = "PriceClass_100"
  comment             = "${var.deployment_name} encrypted browser client"
  origin {
    domain_name              = aws_s3_bucket.web[0].bucket_regional_domain_name
    origin_id                = "web"
    origin_access_control_id = aws_cloudfront_origin_access_control.web[0].id
  }
  origin {
    domain_name = replace(aws_apigatewayv2_api.relay.api_endpoint, "https://", "")
    origin_id   = "relay"
    custom_origin_config {
      http_port              = 80
      https_port             = 443
      origin_protocol_policy = "https-only"
      origin_ssl_protocols   = ["TLSv1.2"]
    }
  }
  default_cache_behavior {
    target_origin_id       = "web"
    viewer_protocol_policy = "redirect-to-https"
    allowed_methods        = ["GET", "HEAD", "OPTIONS"]
    cached_methods         = ["GET", "HEAD"]
    compress               = true
    # CachingDisabled: index/callback must never retain a previous app version.
    cache_policy_id            = "4135ea2d-6df8-44a3-9df3-4b5a84be39ad"
    response_headers_policy_id = aws_cloudfront_response_headers_policy.web[0].id
    function_association {
      event_type   = "viewer-request"
      function_arn = aws_cloudfront_function.web_routes[0].arn
    }
  }
  ordered_cache_behavior {
    path_pattern           = "/v1/*"
    target_origin_id       = "relay"
    viewer_protocol_policy = "https-only"
    allowed_methods        = ["DELETE", "GET", "HEAD", "OPTIONS", "PATCH", "POST", "PUT"]
    cached_methods         = ["GET", "HEAD"]
    compress               = true
    cache_policy_id        = "4135ea2d-6df8-44a3-9df3-4b5a84be39ad"
    # AllViewerExceptHostHeader forwards Authorization, bodies and sync cursors.
    origin_request_policy_id   = "b689b0a8-53d0-40ab-baf2-68738e2966ac"
    response_headers_policy_id = aws_cloudfront_response_headers_policy.web[0].id
  }
  restrictions {
    geo_restriction { restriction_type = "none" }
  }
  viewer_certificate { cloudfront_default_certificate = true }
}
resource "aws_s3_bucket_policy" "web" {
  count  = var.enable_web_ui ? 1 : 0
  bucket = aws_s3_bucket.web[0].id
  policy = jsonencode({
    Version = "2012-10-17"
    Statement = [{
      Sid       = "CloudFrontReadOnly", Effect = "Allow"
      Principal = { Service = "cloudfront.amazonaws.com" }
      Action    = "s3:GetObject", Resource = "${aws_s3_bucket.web[0].arn}/*"
      Condition = { StringEquals = { "AWS:SourceArn" = aws_cloudfront_distribution.web[0].arn } }
      }, {
      Sid       = "RequireTLS", Effect = "Deny", Principal = "*", Action = "s3:*"
      Resource  = [aws_s3_bucket.web[0].arn, "${aws_s3_bucket.web[0].arn}/*"]
      Condition = { Bool = { "aws:SecureTransport" = "false" } }
    }]
  })
}
resource "aws_s3_bucket_cors_configuration" "media_web" {
  count  = var.enable_web_ui ? 1 : 0
  bucket = aws_s3_bucket.media.id
  cors_rule {
    allowed_headers = ["content-type", "content-length", "x-amz-checksum-sha256", "if-none-match"]
    allowed_methods = ["GET", "HEAD", "PUT"]
    allowed_origins = ["https://${aws_cloudfront_distribution.web[0].domain_name}"]
    expose_headers  = ["ETag", "content-length"]
    max_age_seconds = 300
  }
}
output "web_url" { value = var.enable_web_ui ? "https://${aws_cloudfront_distribution.web[0].domain_name}" : null }
output "web_bucket_name" { value = var.enable_web_ui ? aws_s3_bucket.web[0].id : null }
output "web_distribution_id" { value = var.enable_web_ui ? aws_cloudfront_distribution.web[0].id : null }
