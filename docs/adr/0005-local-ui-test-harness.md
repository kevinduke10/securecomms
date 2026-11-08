# 0005: Test a React UI through local Rust client cores

- **Status:** Accepted
- **Date:** 2026-11-07
- **Area:** Development workflow and client architecture
- **Delivery:** Implemented development workflow; production clients pending
- **Source:** User requested a UI and an easy way to test it locally.

## Context

The Rust foundation initially had a CLI demonstration. The user needed an
interactive interface for trying multiple accounts, messages, groups, media,
offline operation, and later interaction changes without an AWS account.

## Decision

Build a responsive React/TypeScript UI, with a loopback-only Rust process hosting
independent Alice, Bob, and Maya client stores and a ciphertext-only local relay.
Browser tabs choose test accounts; the local Rust client cores perform encryption.
The browser-to-local-client bridge carries plaintext locally and does not provide
independent-device isolation between those test accounts.

Serve built UI assets and `/local-api` from one local address. Use
`./scripts/start-local.sh` or `npm run dev`, with port 3000 and `data/ui/` by
default. Preserve data across refresh/restart; use a different `--data-dir` for a
fresh fixture. Browser tests use isolated temporary workspaces on port 3107.

For testing only, the private development manifest contains credentials and vault
keys. Keep it ignored by Git. Keep the harness loopback-only, validate Host/Origin
and local request headers, and never deploy it as the cloud service. Attachments
are currently capped at 32 KiB inside encrypted message payloads.

## Rationale and alternatives

This makes real Rust encryption and persistence immediately testable through a UI
that can later be reused with Tauri. It avoids requiring cloud resources or native
application distribution for each early interaction change.

## Consequences

Clearly label this as a local test workspace and use test data. Do not describe
browser tabs as production-isolated devices or development manifest storage as OS
secure storage. SSO, native packaging, real device enrollment, push, and large-file
S3 delivery remain separate product milestones.

## Implementation and evidence

See [local harness](../../crates/local/src/lib.rs),
[UI](../../apps/ui/src/App.tsx), [launcher](../../scripts/start-local.sh),
[browser tests](../../apps/ui/tests/messaging.spec.ts), and
[restart/boundary tests](../../crates/local/tests/workspace.rs).
