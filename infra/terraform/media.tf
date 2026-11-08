# Only client-encrypted bytes enter this private bucket. Object names are scoped UUIDs.
resource "aws_s3_bucket" "media" {
  bucket        = "${var.deployment_name}-${data.aws_caller_identity.current.account_id}-media"
  force_destroy = false
}
resource "aws_s3_bucket_public_access_block" "media" {
  bucket                  = aws_s3_bucket.media.id
  block_public_acls       = true
  block_public_policy     = true
  ignore_public_acls      = true
  restrict_public_buckets = true
}
resource "aws_s3_bucket_ownership_controls" "media" {
  bucket = aws_s3_bucket.media.id
  rule { object_ownership = "BucketOwnerEnforced" }
}
resource "aws_s3_bucket_server_side_encryption_configuration" "media" {
  bucket = aws_s3_bucket.media.id
  rule {
    apply_server_side_encryption_by_default { sse_algorithm = "AES256" }
  }
}
resource "aws_s3_bucket_lifecycle_configuration" "media" {
  bucket = aws_s3_bucket.media.id
  rule {
    id     = "expire-ciphertext"
    status = "Enabled"
    filter { prefix = "" }
    expiration { days = 30 }
    abort_incomplete_multipart_upload { days_after_initiation = 1 }
  }
}
resource "aws_s3_bucket_policy" "media" {
  bucket = aws_s3_bucket.media.id
  policy = jsonencode({
    Version = "2012-10-17"
    Statement = [{
      Sid       = "RequireTLS"
      Effect    = "Deny"
      Principal = "*"
      Action    = "s3:*"
      Resource  = [aws_s3_bucket.media.arn, "${aws_s3_bucket.media.arn}/*"]
      Condition = { Bool = { "aws:SecureTransport" = "false" } }
      }, {
      Sid       = "BoundSignatureAge"
      Effect    = "Deny"
      Principal = "*"
      Action    = ["s3:GetObject", "s3:PutObject"]
      Resource  = "${aws_s3_bucket.media.arn}/*"
      Condition = { NumericGreaterThan = { "s3:signatureAge" = "60000" } }
    }]
  })
}
resource "aws_iam_role_policy" "media" {
  role = aws_iam_role.relay.id
  policy = jsonencode({
    Version = "2012-10-17"
    Statement = [{
      Effect   = "Allow"
      Action   = ["s3:PutObject", "s3:GetObject"]
      Resource = "${aws_s3_bucket.media.arn}/*"
      }, {
      # HeadObject needs ListBucket to distinguish a missing object (404) from denied access (403).
      Effect   = "Allow"
      Action   = ["s3:ListBucket"]
      Resource = aws_s3_bucket.media.arn
    }]
  })
}
output "media_bucket_name" { value = aws_s3_bucket.media.id }
