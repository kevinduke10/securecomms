# 0004: Commit cryptographic state atomically and keep recovery separate

- **Status:** Accepted
- **Date:** 2026-11-07
- **Area:** Client persistence, delivery, and recovery
- **Delivery:** Transactional state/history and portable recovery implemented; cloud recovery pending
- **Source:** Engineering design in the accepted plan; user explicitly chose user-held recovery keys.

## Context

MLS state advances during message processing. Crashes, ambiguous network results,
and partial writes must not roll ratchets back, lose accepted content, or cause a
retry to encrypt the same send again from inconsistent state.

## Decision

Use encrypted device snapshots and authenticated encrypted history records in a
transactional SQLite client store. Commit outgoing ciphertext/outbox entries and
advanced MLS state together before exposing a send. Retry the exact saved envelope
with its stable idempotency identifier.

On receipt, commit authenticated MLS state, local history, and the sync cursor in
one transaction. A failed page rolls back all three. Keep the relay's accepted
event, deduplication receipt, and sequence update transactional as well.

The user chose user-held recovery keys for eventual encrypted history backups.
Recovery must restore history and enroll a fresh device without resurrecting old
ratchets. A live device vault is not that backup format: do not clone it onto a
second active device or roll it back after sending.

## Rationale and alternatives

Separate state and message writes can expose cryptographic state reuse or content
loss after interruption. Atomic persistence and saved-envelope retries make
restart behavior explicit and testable. History backups require a different
lifecycle from live protocol secrets.

## Consequences

Local state changes need transaction boundaries, authenticated row contexts, and
failure handling. Keep keys in production OS secure storage. Treat plaintext
history returned to a client UI as sensitive client data, not relay storage.

Server delivery payloads currently expire after 30 days; local history is separate.
Deduplication receipts persist indefinitely in the prototype. Archival, tenant
deletion, automatic cloud recovery, and disappearing-message policy need later work.

## Implementation and evidence

See [client store](../../crates/client/src/lib.rs),
[history processing](../../crates/client/src/history.rs),
[receive rollback tests](../../crates/client/tests/receive.rs), and
[relay store tests](../../crates/relay/tests/store_contract.rs). Device snapshots
and encrypted local history work. [ADR-0009](0009-user-held-history-backup-files.md)
implements portable user-key history archives and read-only restores; automatic
cloud backups and production replacement-device enrollment remain pending.
