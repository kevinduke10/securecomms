# 0012: Use direct, functional interface copy

- **Status:** Accepted
- **Date:** 2026-11-08
- **Area:** Product interaction and content
- **Delivery:** Implemented local UI
- **Source:** User explicitly requested removing “A space for your people,” “An open conversation,” and similar decorative language.

## Context

The interface included promotional taglines, conversational flourishes, and
repeated privacy slogans in navigation, login, the composer, empty states, dialogs,
and theme descriptions. The user wants straightforward messaging software.

## Decision

Remove promotional text instead of replacing it with new slogans. Use short,
literal headings, actions, status messages, placeholders, and instructions.
Navigation shows Cirra and functional controls. Login says “Sign in to Cirra,”
the composer placeholder says “Message,” and empty conversations say “No messages.”
Theme descriptions identify colors. The document title is simply “Cirra.”

Keep actionable security and development information: test-account limitations,
key ownership, encrypted delivery, recovery-code handling, fingerprint approval,
and group-membership requirements. State those facts directly and show them where
relevant rather than repeating reassurance throughout the interface.

Existing demo messages and the user's saved conversation contents are data, not
interface copy. Do not rewrite stored history to enforce this editorial decision.
Do not change names, branding, message protocols, keys, persistence, or interactions.

## Rationale and alternatives

A business messaging interface benefits from concise status and clear actions.
Deleting redundant text reduces clutter. Replacing the original taglines with new
marketing language would not meet the user's request. Removing necessary security
instructions would hide limitations or make recovery/enrollment harder to use.

## Consequences

Future UI work should follow this voice, including error and empty states. Do not
add casual slogans or claims like “always secure.” The composer retains normal
bottom spacing and mobile safe-area padding without a promotional footer.
Earlier ADR wording remains historical; this record establishes the current UI
copy direction.

## Implementation and evidence

- [App interface](../../apps/ui/src/App.tsx), [login](../../apps/ui/src/Login.tsx),
  [themes](../../apps/ui/src/theme.ts), [backup status](../../apps/ui/src/Backups.tsx),
  [page metadata](../../apps/ui/index.html), and [layout](../../apps/ui/src/style.css).
- Existing [messaging tests](../../apps/ui/tests/messaging.spec.ts),
  [login/enrollment tests](../../apps/ui/tests/login-enrollment.spec.ts),
  [reaction gesture tests](../../apps/ui/tests/reaction-gestures.spec.ts), and
  [theme tests](../../apps/ui/tests/themes.spec.ts) cover the affected flows.
