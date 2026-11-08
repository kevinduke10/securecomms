# 0020: Run the Rust device engine in an encrypted browser installation

- **Status:** Accepted
- **Date:** 2026-11-08
- **Area:** Client architecture / persistence / security / deployment
- **Delivery:** Published development browser client; WASM messaging/media contracts and live hosted sign-in-page checks pass
- **Source:** User explicitly requested deploying the web UI and selected full browser messaging. WASM, Web Crypto, IndexedDB, Web Locks and CloudFront are engineering choices.
- **Related:** [0004](0004-transactional-client-state-and-recovery.md), [0005](0005-local-ui-test-harness.md), [0010](0010-account-login-and-replacement-device-enrollment.md), [0015](0015-independent-desktop-client.md), [0019](0019-aws-development-deployment-preparation.md).

## Context

The React UI originally calls a loopback Rust test harness or a native device
bridge. Publishing those assets alone cannot provide browser messaging. Deploying
the test harness would place all participants' keys on the server, violating the
participant-only encryption boundary.

## Decision

Reuse the React UI and compile the existing Rust device, crypto and transactional
client-store code to wasm32-unknown-unknown. Run it in a dedicated Web Worker.
Keep the relay's dependency graph free of client keys and decryption. Browser
installations generate distinct identities and require the same signed enrollment,
complete fingerprint verification and explicit approval as native installations.
Manual verified contact/invitation/owner-decision exchange remains applicable.

Use an in-memory SQLite database through rusqlite's WASM implementation. Preserve
its transactional operations. Before any authenticated relay request or encrypted
object transfer, export the complete database plus device profile and await a
strict IndexedDB transaction containing an AES-GCM-encrypted snapshot. Also commit
after each renderer command before returning success. This makes ratchet state,
exact pending ciphertext, group intents, history/cursors and profile metadata
recover together on reload. Persistence failure blocks subsequent operations
until reload; never send a newly encrypted envelope after a failed checkpoint.

An origin-wide exclusive Web Lock is held for the worker lifetime. A second tab
fails closed; different browser profiles are independent devices. Browser storage
contains a non-extractable Web Crypto AES-GCM key and a wrapped random Rust vault
key. The wrapping key encrypts snapshots with fresh random IVs and distinct AAD.
No plaintext keys, access tokens, device credentials, MLS state or message history
are stored in localStorage. The short-lived PKCE verifier/state survive only the
redirect in sessionStorage and are consumed before token exchange. OAuth access
credentials stay in memory and are independently verified by the relay.

Publish only built frontend assets to a dedicated private S3 bucket, served through
CloudFront origin access control and HTTPS. Proxy /v1 through the same distribution
without caching or stripping authorization/query parameters. Keep private media
separate, with CORS limited to the web origin. Add exact Cognito web callback/logout
origins while retaining existing native callbacks. Apply CSP, no-referrer, no-sniff,
frame denial and HTTPS headers. Do not add third-party scripts, analytics, remote
fonts or service-worker caches. Manage hosting in the existing Terraform stack.

## Rationale and alternatives

This shares the existing MLS and application behavior rather than reimplementing
cryptography in JavaScript. Full-database snapshots favor correctness and reuse
over incremental persistence performance. The SQLite wrapper update preserves
fallible unsigned integer conversions used by existing client/relay queries.
Browser storage is bounded at 128 MiB per encrypted snapshot; large histories and
pending attachments can reach it. An incremental browser store is follow-up work.

A non-extractable browser key is not a hardware Keychain guarantee. Same-origin
malicious JavaScript can use the key and operate an unlocked client. A compromised
hosting account or frontend release can deliver such code. CSP and a worker reduce
accidental exposure, not the inherent trust in downloaded code. Clearing site data,
private browsing, browser eviction or losing the key loses that device; never
silently replace a missing key over existing state. History backups do not restore
a live identity or automatically rejoin groups.

## Consequences

Use a supported HTTPS browser with Web Crypto, IndexedDB and Web Locks. One open
tab per browser profile is deliberate. Browser and native devices have different
secure-storage guarantees and separate stores. Native clients retain their
Keychain and filesystem adapters. Existing local test data is untouched. Hosting
adds request/transfer/storage charges; no extra always-on server is introduced.

Browser compatibility, end-to-end interoperability, failed persistence, reloads,
ambiguous sends, and live hosted configuration require explicit validation.
Compilation and Terraform apply alone do not establish those outcomes or a
security audit.

## Implementation and evidence

- [Rust browser adapter](../../crates/device/src/browser.rs),
  [SQLite snapshots](../../crates/client/src/lib.rs),
  [browser worker/storage/login](../../apps/ui/src/browser/), and
  [shared UI transport](../../apps/ui/src/transport.ts).
- [Browser build](../../scripts/build-browser.sh),
  [hosting](../../infra/terraform/web.tf),
  [browser contracts](../../apps/ui/tests/browser-client.spec.ts), and
  [disposable loopback fixture](../../crates/device/examples/browser_fixture.rs).
- Three browser contracts pass using actual Rust/WASM clients: signed enrollment,
  pinned contact/group exchange, MLS messaging, encrypted object upload/download,
  exact retries after a response is deliberately lost, reload recovery, failed
  IndexedDB persistence preventing sends, exclusive-tab behavior, fresh device
  review, durable sign-out and PKCE redirect/state/verifier binding.
- All 19 existing UI regressions pass. Forty relevant native Rust tests passed;
  the final device-only rerun passed all 12 tests. Relevant native Clippy checks
  passed with warnings denied. Four mocked Terraform plans pass.
- Terraform created private S3/CloudFront hosting and added the web callback to
  the existing app client, preserving native callbacks. Publication uploads only
  built static assets, index last, and keeps previous hashed assets. Hosted output
  is separate from the local/native build directory.
- A real HTTPS browser loaded the published worker and WASM under CSP without
  browser errors. Same-origin login configuration returned 200, unauthenticated
  account access returned 401, and the real Cognito password form loaded for the
  web callback. Actual user MFA/browser approval and authenticated AWS messaging/
  S3 file acceptance remain pending; local fixture coverage is not a live-provider
  or security-audit claim.
