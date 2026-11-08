# 0014: Encrypted replies and recipient delivery/read receipts

- **Status:** Accepted
- **Date:** 2026-11-08
- **Area:** Messaging protocol, persistence, privacy, interaction
- **Delivery:** Implemented Rust local-client and UI flow; production platform clients pending
- **Source:** User authorized parallel implementation from the architecture backlog. Event formats, status projection, and default-off read receipts are engineering decisions.
- **Related:** Extends [ADR-0007](0007-encrypted-reactions-and-typing-events.md), preserves [ADR-0006](0006-message-reactions-and-emoji-interaction.md), and follows [ADR-0012](0012-functional-interface-copy.md).

## Context

The application could send ordinary messages and encrypted reactions/typing, but
had no reply references or recipient delivery/read status. Relay acceptance must
not be presented as delivery to a recipient. Polling an account must not mark all
its conversations read or announce which history the user viewed without consent.

## Decision

### Replies

Use a versioned encrypted MLS application event containing ordinary content and a
reference to the original message ID and authenticated sender device. Keep the
existing ordinary-content format unchanged. The local bridge verifies that the
target is an available, delivered ordinary/reply message in the same conversation.
Reject unknown targets, sender substitutions, and references to other conversations.

The reference carries no quoted text. The UI receives a quote resolved from an
earlier authenticated target in its own local history. If the target is unavailable,
show that condition and do not invent or trust quoted content supplied by a peer.
The original author, text, and attachment name come from the authenticated history.

Add Reply to the existing long-press/context-menu reaction popup, preserving the
six quick reactions, plus/search catalog, keyboard interaction, hold timing, and
cancellation behavior. Show the selected target above the composer; canceling the
reply preserves text and attachments. Pending sends cannot be selected as targets.
Send retries use the same saved request ID and exact encrypted envelope.

### Receipts

Use versioned encrypted MLS application events carrying a bounded list (1–64) of
message-ID/authenticated-sender references and an absolute delivered/read state.
Receipt senders come from authenticated MLS identity, never a caller-supplied actor.
States are monotonic per device: read implies delivered, and delayed delivered
events cannot downgrade read. Ignore self-receipts and unknown target references.
Receipts are recipient claims, not proof that a human actually read the content.

Generate delivery receipts only after incoming content and its receive cursor/
ratchet have committed. Generate them for ordinary/reply messages only. Receipt,
reaction, and typing events do not generate receipts. Persist receipt events in
the encrypted history/outbox with the existing atomic queue API before transport;
deterministic, domain-separated IDs and projected own state prevent repeated
polling/retries from advancing the ratchet for duplicate receipt actions.
Offline read receipts remain queued and retry the saved ciphertext after restart.

Read receipts default off for each device and are explicitly enabled using the
account menu. Persist this local preference, with missing old preferences treated
as off. Only emit read receipts for incoming message content visible in the active
conversation while the document is visible and the window focused. Background
polling, other conversations, and the hidden mobile conversation panel do not mark
content read. Dialogs, account menus, and content obscured by overlays do not
mark messages read. Disabling the setting stops creation of new read receipts;
already queued or sent receipts remain.

Show Sent for relay-accepted sends, Delivered for recipient delivery, and Read for
opted-in read claims. Groups show recipient device counts; tooltips identify those
devices' profile names. These counts do not promise that all current group members
have received an old message, particularly after membership changes.

## Rationale and alternatives

Authenticated references avoid a forged quote becoming trusted message content.
Absolute receipt state supports retries and delayed delivery without count inflation.
Reusing encrypted durable application events keeps message/receipt contents outside
the relay and preserves existing ratchet/outbox transaction boundaries. Plaintext
server read markers and marking an entire fetched conversation read were rejected.
Default-off read sharing avoids adding an unexpected history-viewing disclosure.

## Consequences

Receipt events consume encrypted transport/storage and persist as control history,
but never appear as message rows, previews, search results, or unread messages.
Existing databases and ordinary message payloads remain compatible. The local
harness still hosts all demo account keys; this does not add device isolation.
Receipt preferences are device-local and are not enterprise policy or cross-device
settings. New participants cannot quote history their device has not received.

Portable backups preserve optional reply references and resolve archive quotes
from earlier archived messages; receipt state is excluded from history backups.
Legacy archives without references still load. Older application builds that reject
unknown fields need an update to read archives containing the new optional reference.
This implementation does not add disappearing-message retention or authenticated
human-attention guarantees. Production native-client receipt integration and a
security review remain future work.

## Implementation and evidence

- [Encrypted app events/projection](../../crates/local/src/events.rs) and
  [local send/receipt bridge](../../crates/local/src/lib.rs).
- [Message UI](../../apps/ui/src/App.tsx), [reaction/reply popup](../../apps/ui/src/MessageBubble.tsx),
  and [reply/receipt styles](../../apps/ui/src/replies.css).
- [Rust tests](../../crates/local/tests/replies_receipts.rs): authenticated target
  lookup, invalid/cross-chat targets, idempotence, recipient delivery versus read,
  opt-in settings, restart/offline retry, strict request schemas, and no plaintext
  persisted content.
- [Browser tests](../../apps/ui/tests/replies-receipts.spec.ts): reply selection,
  cancellation, quote sync/reload and backup restore, delivery, read opt-in,
  hidden-conversation behavior, and modal occlusion.
- Existing gesture/typing/reaction tests remain required regression checks.
