# 0013: Admit approved devices through atomic owner-authorized MLS additions

- **Status:** Accepted
- **Date:** 2026-11-08
- **Area:** Security, protocols, persistence, interaction
- **Delivery:** Rust core/relay and local owner invitation workflow implemented; production native client integration and deployment pending
- **Source:** User authorized parallel work on the architecture backlog, including existing-group rejoining. The narrow single-device addition protocol, journal, and retention policy are engineering choices.
- **Related:** Extends [ADR-0003](0003-participant-only-encryption-and-trust.md), [ADR-0004](0004-transactional-client-state-and-recovery.md), and [ADR-0010](0010-account-login-and-replacement-device-enrollment.md).

## Context

Replacement enrollment established a fresh device identity and relay credential,
but existing relay conversations had immutable device allowlists. Accepting an
account login, enrolled identity, or relay-supplied roster as automatic MLS admission
would violate the separation between account authorization and conversation keys.
Existing clients also rejected all general commits, intentionally preventing
unapproved membership mutations.

An owner can advance an MLS epoch before an offline participant submits ciphertext
already encrypted in an older epoch. Saved envelopes must remain unchanged. A new
device cannot decrypt an epoch it never joined, and older unavailable packets must
not permanently block later authenticated messages.

## Decision

### Explicit owner admission and delivery

Add a narrow single-device `GroupAdmission`: a stable request ID, the expected prior
device roster, the added device ID, an MLS Add commit, and one addressed Welcome.
Require an active conversation owner and an active target device in the same tenant.
Enrollment authorizes the device; the owner separately confirms the exact verified
identity fingerprint before inviting it. Group membership remains device-based.

`Store::admit` atomically checks the prior roster, inserts the commit and Welcome
as consecutive events, updates the roster and sequence, records the admission
receipt and individual event deduplication receipts, and installs the target's
join floor. SQLite uses a single transaction; DynamoDB uses conditional transactions
checking current metadata/sequence and both active registrations. Changed retries
conflict; identical saved requests return their original sequences. A retry after
target revocation never reactivates its credential.

New-device sync cannot read events before its admission Welcome, even when it asks
for cursor zero. The client also prepares its cursor at the admitted commit sequence
before consuming the Welcome. The Welcome authenticates the new MLS epoch and is
checked against independently pinned device bindings. No old history, old device
keys, or old outbox state is transferred.

Admission ciphertexts are individually validated and together capped at 64 KiB
decoded. The admission HTTP route allows at most 128 KiB to accommodate base64 and
the bounded UUID roster; ordinary event bodies retain their existing 96 KiB limit.

### Transactional client and local recovery

Before producing an addition, drain the owner's existing conversation outbox.
`ClientStore::queue_admission` verifies the expected roster against actual MLS
members, advances the state, and saves the exact commit/Welcome bundle in a separate
transactional admission table. Acknowledged bundles remain as idempotency records;
repeating the same request/package/pin returns the original ciphertext without
advancing keys or resending it as a new operation. Pending admissions are flushed
before later ordinary outgoing envelopes.

The local client adapter first saves an owner-vault-key encrypted journal containing
the verified pins, public target package, expected roster, and stable request ID.
It writes the exact saved client bundle, saves the accepted relay receipt, prepares
the newcomer, updates the local trusted roster, and only then acknowledges. Journal
writes sync the file and containing directory. Startup and retries resume interrupted
steps; a newcomer already joined is not reset or rolled back. Completed journals
retain explicit approval evidence for existing clients. This preserves the local
harness's documented one-Mac key-storage exception and does not provision cloud
client secrets.

Existing recipients merge only a commit with a locally approved event ID, the
expected authenticated owner sender, exactly one Add proposal matching the pinned
added credential/signing key, and unchanged owner identity. Other proposals,
removals, unrelated additions, unapproved commits, and identity substitution remain
rejected. Relay metadata alone is not approval. The ordinary application-message
receive path still cannot silently accept membership changes.

### Delayed epochs and unavailable-message notices

Explicitly retain **one prior MLS epoch** around membership changes, including
upgrading an existing group's persisted configuration before rotating. OpenMLS
normally disables prior-epoch retention. One epoch is a deliberate, bounded
forward-secrecy tradeoff enabling an already participating recipient to decrypt a
saved envelope delivered after one admission. Do not increase this retention
window without another security decision.

Persist each recipient's Welcome epoch atomically with its joined state/cursor.
For Application events only, a client can classify a packet as unavailable when
it is well-framed private MLS Application ciphertext for this exact group and its
embedded epoch is strictly below the maximum of the join epoch and the current
epoch minus one. The join epoch may never exceed the authenticated current epoch.
This is a routing disposition, not a claim that inaccessible old ciphertext was
authenticated. Current/future epochs, wrong-group data, malformed frames, and
commit/control packets cannot use this skip path; authentication failures still
roll back the entire receive page.

An unavailable packet creates no plaintext history, synthetic message, delivery
receipt, or read receipt. Commit only its cursor advancement and a metadata-only
per-conversation unavailable count in the same transaction. The local UI can show
that durable count after restart. The sender keeps its original saved envelope;
a relay acceptance means Sent, not Delivered. Two or more epoch changes may make
an older queued message unavailable, but do not permanently block later current
epoch messages. No automatic re-encryption or hidden resend is introduced.

## Rationale and alternatives

Combining roster mutation and both encrypted events prevents partial authorization
or an admitted recipient with no Welcome. Saved batches and the encrypted journal
make ambiguous responses and local crashes recoverable without ratchet rollback.
Explicit pins and proposal checks preserve the previous rejection of arbitrary MLS
commits while adding the one operation needed for replacement devices.

Automatically copying old secrets, trusting server-provided rosters, publishing
commit and Welcome separately, silently ignoring generic decryption errors, or
re-encrypting an ambiguous old send were rejected. Unbounded prior-epoch retention
would retain excessive old transport secrets. Blocking all later traffic on an
unavailable old packet would make offline membership changes unusable.

## Consequences

The conversation owner must be available and retain a usable identity. Owner
transfer, general roster reconciliation, existing-device remove/re-add, and native
multi-endpoint distribution of approval journals remain separate workflows.
The relay cannot decrypt or validate MLS ciphertext; recipients must validate pins
and the authorized commit. A compromised delivery service can still suppress
traffic and tamper with routing, as in the existing threat model.

Welcomes retain the relay's 30-day delivery window. A device that does not join
within that window may need a fresh identity/admission rather than a replayed old
Welcome. Prior-epoch retention is bounded; unavailable-message notices do not
recover missing plaintext. The counter is local metadata, not an audited delivery
record. Business administrators managing enrollment do not thereby gain group
membership or decryption keys. No production deployment or security audit is claimed.

## Implementation and evidence

- [Protocol batch and validation](../../crates/protocol/src/membership.rs),
  [strict MLS addition and epoch classification](../../crates/crypto/src/lib.rs),
  [transactional client batches](../../crates/client/src/membership.rs), and
  [atomic receive processing](../../crates/client/src/history.rs).
- [Relay contract/API](../../crates/relay/src/lib.rs),
  [SQLite](../../crates/relay/src/sqlite.rs), and
  [DynamoDB](../../crates/relay/src/dynamodb.rs).
- [Encrypted local journal and recovery](../../crates/local/src/membership.rs),
  [group member UI](../../apps/ui/src/GroupMembers.tsx), and
  [browser checks](../../apps/ui/tests/group-membership.spec.ts).
- [Client tests](../../crates/client/tests/membership.rs) cover pinned rejection,
  stable retries across restart/acknowledgment, no copied history, one-epoch late
  decryption, two-admission unavailable handling, corrupted-current/wrong-group
  rejection, atomic count rollback, durable notices, and later valid messages.
- [Shared relay contracts](../../crates/relay/tests/membership_contract.rs) cover
  owner/tenant/credential authorization, roster compare-and-swap, rollback on a
  second-event collision, deduplication, newcomer floors, and revocation.
- [Local integration tests](../../crates/local/tests/group_membership.rs) exercise
  session/fingerprint checks, journal restart/retry recovery, preserved offline
  envelopes, and separation of old history from future group messages.

Validation completed on 2026-11-08: workspace Rust tests, the DynamoDB Local
event/enrollment/admission contracts, Clippy, and the authenticated browser group
admission test pass. The relay's normal dependency graph still excludes the client
and MLS crypto crates. This does not validate a live AWS deployment.
