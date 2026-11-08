# 0006: Open reactions by holding a message, with searchable emoji behind plus

- **Status:** Accepted
- **Date:** 2026-11-07
- **Area:** Message interaction and shared emoji data
- **Delivery:** Implemented local UI
- **Source:** Explicit user requests for iMessage-like long hold and a plus/search picker.

## Context

The first reaction UI put smile-plus buttons below every message. The user
explicitly replaced that approach with holding the message itself, then requested
common default emoji and a plus button for a larger searchable suite.

## Decision

- Open the reaction picker after a stationary 500 ms press on a message bubble.
  Remove the per-message smile-plus trigger. Cancel the hold on early release,
  movement beyond 10 px, scrolling, pointer cancellation, or leaving the bubble.
- Preserve quick file downloads and ordinary taps. Support right-click and
  Enter/Space/context-menu keys for desktop and keyboard access, with focus
  restoration and visible focus styling.
- Start with six common emoji and a plus button. Plus opens the expanded picker
  with text search by names/keywords, a back control, and dismissal controls.
- Bundle the emoji catalog locally and load the expanded UI data on demand.
  Generate `shared/emoji.json` from pinned Unicode/keyword packages. Both UI and
  Rust validation use this same catalog; preserve its provenance and licenses.
- Group matching reactions into counts. A person can choose a replacement emoji
  or tap their highlighted reaction to remove it. Transport semantics are in
  [ADR-0007](0007-encrypted-reactions-and-typing-events.md).

## Rationale and alternatives

The user selected the familiar message-hold pattern over visible action buttons
under each bubble. Quick defaults keep frequent reactions close; an expandable
search library supports the broader selection without crowding the initial row.
Local data avoids sending search text to an external emoji service.

## Consequences

Do not reintroduce the smile-plus trigger as the default interaction without
reconciling this decision. Gesture work must preserve scrolling, file links,
touch behavior, keyboard access, and mobile viewport/keyboard positioning.
Expanded choices must be accepted and rendered by the backend, not just shown in
the picker. Catalog updates must regenerate the shared data, not split the UI and
server allowlists. Available glyph rendering depends on the platform's fonts.

## Implementation and evidence

See [message bubble](../../apps/ui/src/MessageBubble.tsx),
[gesture/search tests](../../apps/ui/tests/reaction-gestures.spec.ts),
[shared catalog notes](../../shared/README.md), and
[generation script](../../scripts/generate-emoji-data.mjs). At initial recording,
the catalog contains 1,914 emoji; that count is generated data, not a fixed limit.
