# 0016: Explicit owner handoff and member removal

- **Status:** Accepted
- **Date:** 2026-11-08
- **Area:** Security, protocol, persistence, interaction
- **Delivery:** Implemented in the reusable Rust client, relay APIs, local test application, and independent desktop runtime.
- **Source:** Accepted next implementation work; exact transaction and authenticated-context design are engineering decisions.

## Context

[ADR-0013](0013-approved-group-admission.md) establishes explicit pinned group additions. A business group also needs ownership handoff and removal of a lost or unwanted device. Neither account administration nor a relay roster update can manufacture MLS keys. Members can be offline, and an ambiguous network response must not cause a second cryptographic transition.

## Decision

The current owner explicitly chooses another current pinned device and verifies its fingerprint. Ownership transfer creates an MLS self-update commit; removal creates exactly one MLS Remove proposal commit. Both rotate the group epoch. Versioned authenticated additional data binds the conversation, request ID, current owner, complete prior roster, action, and target. Recipients require a trusted local approval with the exact context and pinned authority and target keys. They authenticate the sender, prior MLS roster, context, exact proposal set, and unchanged owner signing identity before merging. Relay metadata alone cannot authorize a commit. Existing permissive removal helper APIs are retained for compatibility; lifecycle operations use the stricter context-bound methods.

The client advances its MLS state and saves the exact lifecycle request in one encrypted-device SQLite transaction. Saved requests survive restart and acknowledgment and are reused verbatim. Another unacknowledged admission, lifecycle change, or message outbox blocks preparation of this transition. The relay requires the active current owner and exact expected roster. The lifecycle route permits a 128 KiB JSON body for the prior roster and a decoded commit of at most 64 KiB. A transfer target must remain active. A removal target may already be revoked. SQLite and DynamoDB atomically append the commit, update conversation metadata and sequence, and save event and lifecycle duplicate receipts. Retrying an accepted handoff remains valid for its original active sender after ownership changes, but a changed request conflicts.

The local harness saves an authenticated encrypted owner-vault journal before preparing or publishing the transition, saves the server receipt before updating its roster, and resumes incomplete phases at startup and flush. It retains completed decisions for offline recipients. This is local trusted-device approval distribution, not a production network trust bootstrap. The UI adds fingerprint-confirmed transfer/removal controls to the existing group members dialog. History backup participants include former message/reaction authors; a historical participant entry grants no live group authority.

The independent desktop runtime stores its own trusted group registry, approvals, and durable decision journal in its encrypted device vault. It exports accepted public decision packets for manual distribution and imports them only after full approving-owner fingerprint verification plus separately pinned participant contacts. Import requires the exact saved current owner and prior roster. Import alone cannot change an existing group's owner or access roster: the receiver must atomically authenticate the corresponding MLS commit first, recorded in `applied_group_commits_v1`. The newly admitted recipient can import its own approved admission as a new group, with the saved Welcome delivery floor; importing history never grants these permissions. Publisher recovery retries saved batches. Before any new application, receipt, transient signal, or media message is queued, the native runtime prepares incomplete saved owner intents without network access, including while offline. This prevents a direct send immediately after restart from advancing the old epoch ahead of an unprepared handoff. A crash between the client SQLite transaction and the profile journal write reuses the exact client batch even when newer messages already occupy the outbox. This manual packet distribution is an implemented prototype trust transport, not an automatic production directory.

## Rationale and alternatives

A server-only owner swap would omit authenticated client consent. An unbound empty commit could be substituted for a different transfer target. The context-bound self-update provides a signed handoff and consistent epoch ordering without exporting private state to the new owner. Separate non-atomic metadata and ciphertext writes would permit access and key state to disagree after failures. An administrator decryption or owner-recovery bypass is deliberately absent.

## Consequences

Removed devices immediately lose relay sync and write access. Newly encrypted traffic uses the next group epoch; old device keys cannot decrypt it. Already received history, cached attachments, and plaintext cannot be clawed back. Members must process authorized commits in order; the existing single-prior-epoch policy for delayed applications remains in effect. Removal may leave a one-device group. Removing oneself requires handing off first.

A removed independent device can retain its cached conversation and show a sync access error: the relay excludes it before delivering the removal commit. Its saved plaintext is not erased, and future server access remains denied.

Ownership cannot be recovered automatically if the current owner and its keys are lost. Rejoining a previously removed device using its stale group state is not supported; enroll a fresh device and explicitly admit it. Production trusted approval transport, multi-owner governance, re-add workflows, and stronger identity directory distribution remain future work. The local harness still hosts every demo key on one Mac and is loopback-only.

## Implementation and evidence

- Wire context: [protocol lifecycle](../../crates/protocol/src/lifecycle.rs).
- Authenticated MLS transitions: [crypto lifecycle](../../crates/crypto/src/lifecycle.rs), [crypto tests](../../crates/crypto/tests/lifecycle.rs).
- Durable outbox and atomic receive: [client lifecycle](../../crates/client/src/lifecycle.rs), [receive integration](../../crates/client/src/history.rs), [restart/rollback tests](../../crates/client/tests/lifecycle.rs).
- Atomic stores and duplicate retry authorization: [SQLite](../../crates/relay/src/sqlite.rs), [DynamoDB](../../crates/relay/src/dynamodb.rs), [store contract](../../crates/relay/tests/lifecycle_contract.rs).
- Local recovery journal: [local lifecycle](../../crates/local/src/lifecycle.rs).
- Independent runtime and import/restart/authority test: [native groups](../../crates/device/src/groups.rs).
- UI and browser contract: [GroupLifecycle](../../apps/ui/src/GroupLifecycle.tsx), [browser test](../../apps/ui/tests/group-lifecycle.spec.ts).

Rust crypto, durable client, SQLite store, local recovery, and independent runtime trusted import/restart lifecycle tests have passed. UI production build and the browser ownership-transfer/member-removal flow passed; DynamoDB transaction code requires emulator validation and has not been exercised against a deployed AWS service. No security audit or performance benchmark is claimed.
