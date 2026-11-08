# 0007: Carry reactions and typing as encrypted application events

- **Status:** Accepted
- **Date:** 2026-11-07
- **Area:** Application protocol and presence
- **Delivery:** Implemented local app and Rust client support
- **Source:** User requested reactions and typing indicators; encrypted transport is an engineering choice consistent with participant-only privacy.

## Context

Reactions must persist and synchronize without producing new chat bubbles or
unread counts. Typing needs timely feedback without sharing unsent text or leaving
stale indicators when a tab disappears.

## Decision

Use MLS application payloads for both features. Reaction events identify the
original message and its authenticated sender, and set an absolute emoji state
for the reacting sender. Null removes it. Validate conversation access, ordinary
message targets, and the shared emoji catalog. Persist events encrypted and
project them into counts; repeated requests must not double reactions.

Typing events contain an active flag and expiry, not draft text. Throttle active
pulses to once per two seconds. Stop on send, editor blur, browser inactivity, or
leaving the conversation; the UI idle timeout is currently 2.6 seconds. Indicators
expire after six seconds even if the stop signal is lost.

Authenticate typing through the same client crypto path, committing its ratchet
and receive cursor without persisting its plaintext in chat history. Keep display
state in memory, scoped to conversation and authenticated device, and ordered by
relay sequence. Neither typing nor reactions become message previews or unread
messages. Existing ordinary-message payloads remain readable.

## Rationale and alternatives

Shared encrypted transport preserves the privacy model and offline delivery
machinery. Absolute reaction state is safer for retries than increment/toggle
commands. Typing is transient UI state; persistence as visible chat history would
give misleading behavior after reconnect or restart.

## Consequences

Reactions queue offline and survive restart. Typing does not transmit offline
drafts and does not add local history entries. Expiry refers to the displayed
indicator: encrypted relay records still follow the normal delivery retention
window. Preserve ratchet/outbox atomicity even for short-lived signals; dropping
protocol state or reusing it is not an acceptable presence optimization.

The current local model maps each test person to one device. Account-wide
multi-device reaction identity must be designed with production enrollment;
do not infer that it has already been solved.

## Implementation and evidence

See [event parsing/projection](../../crates/local/src/events.rs),
[client history/transient processing](../../crates/client/src/history.rs),
[backend tests](../../crates/local/tests/reactions_typing.rs),
[receive tests](../../crates/client/tests/receive.rs), and
[reaction/typing browser tests](../../apps/ui/tests/reactions-typing.spec.ts).
