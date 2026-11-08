# 0002: Share a Rust core and deploy a serverless AWS relay

- **Status:** Accepted
- **Date:** 2026-11-07
- **Area:** Architecture and deployment
- **Delivery:** Foundation implemented; production rollout pending
- **Source:** User requirements for Rust/AWS and the accepted implementation plan.

## Context

The product needs a shared encryption/messaging implementation, low idle hosting
cost, and repeatable deployment into both our AWS account and customers' accounts.

## Decision

Keep protocol contracts, client cryptography, transactional client state, relay
logic, and cloud adapters in separate Rust crates. Use Axum for the HTTP surface,
with SQLite for local relay testing and DynamoDB for AWS. Exercise both stores
against the same behavioral contract.

Run the cloud relay as Rust Lambda on `provided.al2023`, behind API Gateway, with
Terraform for repeatable infrastructure. Keep client crypto/state crates outside
the Lambda dependency graph. Durable correctness must not depend on a warm
Lambda execution environment.

Use the same application/infrastructure design for hosted and dedicated installs.
The current relay uses HTTPS submission and cursor-based pull sync. Future
WebSockets/push should wake clients, while durable catch-up remains authoritative.
Large media should eventually use client-encrypted S3 objects. Future WebRTC
calling requires a separate media path; the video plane is not promised to be
entirely serverless.

## Rationale and alternatives

Rust directly satisfies the user's requested core language and enables shared
behavior across clients and backend services. Managed AWS components support
incremental scale without always-on application servers. The local adapter allows
development and meaningful integration tests without a cloud account.

Transport/crypto library details are engineering choices within the accepted
plan. Rust alone does not guarantee low end-to-end latency; performance and cost
must be measured across the complete system.

## Consequences

Delivery needs explicit idempotency, transactional writes, paging, retries, and
authorization checks. API limits and Lambda payload limits constrain wire sizes.
The prototype's infrastructure limits favor a conservative pilot, not the final
load target. Maintain one deployment path rather than a divergent customer fork.

## Implementation and evidence

- [Workspace boundaries](../../Cargo.toml), [relay](../../crates/relay/src/lib.rs),
  and [Lambda entry point](../../crates/relay/src/bin/lambda.rs).
- [Store contract tests](../../crates/relay/tests/store_contract.rs) and
  [Terraform](../../infra/terraform/main.tf).
- [Architecture details](../architecture.md) document limits and unimplemented
  push/fan-out capabilities. Local validation does not establish an AWS deployment,
  production capacity, or operational readiness.
