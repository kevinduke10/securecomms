# 0021: Organize device security tools into responsive sections

- **Status:** Accepted
- **Date:** 2026-11-08
- **Area:** Interaction / client UI
- **Delivery:** Shared UI verified and installed in iOS Simulator; development cloud hosting subsequently removed
- **Source:** User reported poorly laid-out Devices & security dialogs and overflowing codes during sign-in/device setup. Tabs, bounded scrolling, public-value copy controls and grouped forms are engineering choices.
- **Related:** [0010](0010-account-login-and-replacement-device-enrollment.md), [0012](0012-functional-interface-copy.md), [0015](0015-independent-desktop-client.md), [0018](0018-ios-client-and-native-platform-bridge.md), [0020](0020-browser-client-and-web-hosting.md).

## Context

Native/browser security tools stacked account/device approvals, contact exchange,
conversation invitations and group decisions into one narrow dialog. Unstyled
packet textareas and long identifiers made the forms difficult to use on mobile.
The enrollment screen also displayed long relay URLs and account IDs without
consistent containment or copying controls.

## Decision

Use a wider Devices & security dialog with a fixed header/navigation and a
separately scrolling content body. Default to Devices; provide Contacts,
Invitations and Group changes tabs for independently held clients. Support keyboard
arrow/Home/End navigation, keep form drafts when switching tabs, and return each
section to its top when selected. Keep the local harness's device workflow available
without exposing native-only package exchange.

Separate sharing a package from importing one, with bounded, wrapping read-only
exports and clearly labeled input forms. Group device status, enrollment requests
and administrator lookup; collapse the less frequently used other-account lookup.
Keep the close action visible as content scrolls.

Display public fingerprints and account identifiers in wrapping containers with
copy controls. Apply the same controls on the sign-in/enrollment screen; wrap
relay URLs and stack actions on narrow screens. Clipboard access uses an explicit
user action. If clipboard access is unavailable, select the public value for the
platform's Copy command. Package exports can reveal membership metadata, even
though they contain no private keys; retain that explanation.

## Rationale and alternatives

Separate sections make common device tasks accessible without scrolling through
unrelated packet forms. A single scrolling dialog body keeps navigation predictable
on small screens. Wrapping preserves complete identifiers, while copying avoids
manual transcription. Do not truncate the fingerprint used for verification.

## Consequences

This changes layout and access to existing tools, not identity policy, trust,
protocols, persistence, encryption or enrollment authorization. Full fingerprint
matching remains mandatory for approval/import. Copying a server-displayed value
is not independent verification; users must still compare it with the new device
or an independently verified contact. Simpler first-device policy remains a
separate [todo](../todo.md).

Shared UI changes apply to the hosted browser on publication and native apps on
rebuild. Existing client keys, history and local workspaces are preserved.

## Implementation and evidence

- [Tabbed sections](../../apps/ui/src/DeviceSecurity.tsx),
  [responsive styles](../../apps/ui/src/device-security.css),
  [public-value controls](../../apps/ui/src/SecurityValue.tsx),
  [devices](../../apps/ui/src/Devices.tsx),
  [setup and package forms](../../apps/ui/src/DesktopSetup.tsx).
- [Layout, draft, keyboard and copy tests](../../apps/ui/tests/security-layout.spec.ts),
  [actual WASM client contracts](../../apps/ui/tests/browser-client.spec.ts), and
  [local enrollment contract](../../apps/ui/tests/login-enrollment.spec.ts).
- Four layout tests pass: desktop/mobile tab layout, keyboard navigation, retained
  drafts, persistent close action, full-fingerprint approval gating, 320/390 px
  enrollment wrapping and exact clipboard values. Screenshots were visually checked.
- All four local authentication/group-enrollment tests and all three real WASM
  client contracts pass. TypeScript checks and documentation consistency checks pass.
- The hosted bundle was published. An isolated browser using the live assets and
  synthetic public enrollment values verified no horizontal overflow at 320 and
  390 px, with no page errors. No live account was enrolled by that layout check.
- The iOS Simulator build succeeded and was installed over the existing app.
  The simulator relocated its data container during the update; existing
  installation/profile/database files remained, and the Administrator messaging
  screen reopened successfully. No app data was reset. Physical-device layout and
  platform clipboard behavior have not been manually verified.

The user subsequently requested cloud teardown; the web deployment is no longer
live. See [teardown evidence](0019-aws-development-deployment-preparation.md#teardown-evidence).
