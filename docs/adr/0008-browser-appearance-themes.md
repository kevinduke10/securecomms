# 0008: Browser appearance themes with shared semantic colors

- **Status:** Accepted
- **Date:** 2026-11-08
- **Area:** Interaction and client preferences
- **Delivery:** Implemented local UI
- **Source:** User requested additional themes. Palette names, selection UI, and local persistence are engineering choices.

## Context

The local UI originally used a single green palette with colors embedded throughout
its styles. Additional themes must keep text, messages, forms, dialogs, reactions,
and typing indicators coherent on desktop and mobile without changing messaging
or the long-press behavior in [ADR-0006](0006-message-reactions-and-emoji-interaction.md).

## Decision

Offer Forest (the default), Ocean, Rose, Sand, and the dark Midnight palette through
an Appearance button in the workspace's top strip. A modal uses native radio
controls and small previews; selection applies immediately. Shared semantic CSS
variables describe surfaces, text, accents, message bubbles, and status colors.
Apply the active theme to the document root so body-mounted reaction portals and
native dialogs inherit it. Preview cards scope the same variables locally.

Save the validated theme identifier in browser local storage (`croaker:theme`).
Initialize it before React mounts, synchronize changes between same-origin tabs,
and keep it across test-account switches. Missing, invalid, or unreadable storage
falls back to Forest; selection still works for the current page if writes fail.
Set the native color scheme and browser theme-color metadata with the palette.

## Rationale and alternatives

Semantic roles keep new palettes consistent across existing components. Separate
component-by-component theme overrides would duplicate styling and make omissions
more likely. Browser persistence fits this presentation preference without adding
relay traffic or changing encrypted application events. Native radio controls
provide keyboard selection without a custom selection protocol.

## Consequences

The preference belongs to this browser and origin, shared by its local test
accounts. It does not synchronize across devices, use system dark-mode settings,
or configure business branding. Clearing browser storage resets the preference.
The brand artwork remains fixed. Theme changes affect presentation only; client
keys, message history, tenant authorization, and the loopback boundary are unchanged.
This change does not constitute a formal accessibility audit.

## Implementation and evidence

- [Theme selection and persistence](../../apps/ui/src/theme.ts),
  [startup initialization](../../apps/ui/src/main.tsx), and
  [Appearance UI](../../apps/ui/src/App.tsx).
- [Palettes](../../apps/ui/src/themes.css),
  [interface styles](../../apps/ui/src/style.css), and
  [reaction/typing styles](../../apps/ui/src/reactions.css).
- [Browser checks](../../apps/ui/tests/themes.spec.ts) cover every palette,
  refresh persistence, account tabs, storage synchronization/reset, keyboard and
  mobile selection, reaction portal colors/search, and storage failure fallback.
- [Local workflow](../../README.md). Production client branding and cross-device
  appearance preferences remain unimplemented.
