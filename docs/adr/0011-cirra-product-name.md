# 0011: Adopt Cirra as the product name

- **Status:** Accepted
- **Date:** 2026-11-08
- **Area:** Product identity and compatibility
- **Delivery:** Implemented in the UI, current documentation, and visible development labels
- **Source:** User explicitly selected Cirra from the proposed names. Compatibility boundaries and the initial C speech-bubble mark are engineering choices.

## Context

The repository began with Croaker as a working name. The user selected Cirra as
the app's name. The existing workspace already contains encrypted state, pinned
device identities, backup files, browser preferences, and deployment configuration
whose identifiers use the old name.

## Decision

Use Cirra for the app title, login greeting, workspace wordmark, accessibility
labels, help copy, favicon, current documentation, development startup messages,
and the Cognito resource server's display name. Use an initial C speech-bubble
mark in the existing brand colors. Preserve the available appearance themes.
New backup downloads use `cirra-history.crkbackup`; both old and new filenames
remain valid imports because the filename does not determine the encrypted format.

Keep internal Rust/npm package names, commands, workspace directories, environment
variables, cookies, local-storage keys, API scopes, protocol identifiers, key
fingerprints, authenticated-encryption contexts, recovery-code prefixes, and backup
extensions/headers unchanged. Keep the old icon asset available for older cached
pages, while the current UI references `/cirra.svg`.

Retain earlier ADRs as historical records using the name current at the time.
Current documentation explains the retained internal names. The repository's
absolute folder name is not changed by product naming.

## Rationale and alternatives

The selected name is the user's product decision. A global text replacement would
change cryptographic derivation/authentication contexts and break saved data,
or rename infrastructure identifiers and risk resource replacement. A display
rename achieves the intended product identity without a data/protocol migration.
The initial vector mark removes the working name's frog association without
introducing a bitmap dependency or altering theme choices.

## Consequences

Contributors will encounter both Cirra branding and legacy `croaker` technical
identifiers. Treat those identifiers as compatibility contracts rather than stale
copy to replace indiscriminately. A future internal rename requires an explicit
migration plan. This change makes no claim about trademark or domain availability.

## Implementation and evidence

- [App shell](../../apps/ui/src/App.tsx), [login](../../apps/ui/src/Login.tsx),
  [page metadata](../../apps/ui/index.html), and [brand mark](../../apps/ui/public/cirra.svg).
- [Download label](../../apps/ui/src/Backups.tsx) and
  [local export response](../../crates/local/src/backups.rs).
- [README](../../README.md), [architecture](../architecture.md), and
  [Cognito display label](../../infra/terraform/login.tf).
- Existing [login/enrollment browser checks](../../apps/ui/tests/login-enrollment.spec.ts)
  use the Cirra greeting. [Backup checks](../../apps/ui/tests/backups.spec.ts) and
  [Rust backup tests](../../crates/local/tests/backups.rs) continue to import legacy
  `.crkbackup` files. No cryptographic or infrastructure identifier migration is performed.
