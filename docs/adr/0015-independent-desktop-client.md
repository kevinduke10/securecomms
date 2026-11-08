# 0015: Independent desktop client and OS-held device secrets

- **Status:** Accepted
- **Date:** 2026-11-08
- **Area:** Architecture / security / persistence / deployment
- **Delivery:** Independent development client implemented; live provider acceptance, distribution, and platform UI validation pending
- **Source:** User requirement for an independently installed client; Tauri, platform scope, native IPC, and Keychain integration are engineering decisions.
- **Related:** [0003](0003-participant-only-encryption-and-trust.md), [0004](0004-transactional-client-state-and-recovery.md), [0010](0010-account-login-and-replacement-device-enrollment.md), [0013](0013-approved-group-admission.md).

## Context

The multi-account local test server owns every demo account and its keys on one machine. It cannot provide the endpoint isolation required by an installed messaging client. Each installation needs its own identity, MLS state, encrypted history, pending ciphertext, and device authorization. Account login authorizes requesting enrollment; it must not silently approve a replacement device or transfer another endpoint's live ratchet state.

## Decision

Use a Tauri 2 desktop shell around the existing bundled React interface and a separate Rust device runtime. The shell has no dependency on `croaker-local`, creates no demo users, and starts no UI HTTP server. The runtime uses the application data directory, owns its installation lock, and performs the existing transactional client-store operations. Existing local workspace files remain untouched.

Initially support macOS. Device secrets are stored in macOS Keychain through the `keyring` crate's `apple-native` backend. The provider implements the runtime's narrow `SecretStore` trait; it never returns secrets through Tauri commands. Missing credentials are distinguished from Keychain failure. Denied access and unsupported operating systems fail closed; there is no plaintext-file, environment-variable, or mock storage fallback. Keychain operations are serialized and raw platform errors are not returned to the UI.

The renderer receives message plaintext needed to display conversations, but no vault keys, MLS private state, device bearer credentials, OAuth tokens, or PKCE verifier. Native runtime responses must preserve that boundary. Renderer compromise can still operate the unlocked client and read visible plaintext; IPC isolation does not protect against a compromised endpoint.

The only shell commands are guarded `client_request`, `desktop_info`, argument-free `open_login`, and `save_download`. The save command accepts a validated basename and bounded base64 data, then opens an OS save panel for an explicit destination and overwrite decision. It accepts no filesystem path; new files use mode `0600`. Encoded payloads are limited to 40 MiB and decoded files to 27 MiB. Requests are bounded and confined to client API namespaces; the runtime applies its exact route and authorization allowlist. Tauri capabilities grant these commands only to the bundled main window. There are no general filesystem, process execution, HTTP, arbitrary URL opener, or JavaScript evaluation commands. Navigation stays on the bundled application origin. CSP blocks remote scripts, frames, forms, and direct renderer network access.

Account login opens the system browser using a native runtime-generated URL, after the runtime validates its configured Cognito origin and authorization path. The renderer cannot supply the URL. The runtime retains OAuth state and PKCE verifier and receives the callback through its fixed loopback callback listener. Configure `http://localhost:38317/callback` as an allowed Cognito callback URL for this desktop flow. This account login remains separate from the fingerprint approval required for device enrollment.

Do not clone a live installation database as a device transfer mechanism. A replacement installation generates a fresh identity and obtains explicit approval. User-held history backups remain encrypted, separate read-only archives, following [0004](0004-transactional-client-state-and-recovery.md).

## Rationale and alternatives

Tauri keeps the cryptographic client in Rust and uses the operating system's WebView for the interface. A browser-only application would require a separate decision about browser-held device secrets. Reusing the local server would preserve its shared ownership of demo accounts and keys. A plaintext desktop configuration file would weaken protection of the encrypted client database.

`keyring` 3.6.3 is pinned with only `apple-native` enabled. This deliberately avoids the crate's mock backend and the larger API change in version 4. Future platform support must explicitly select, implement, and test its native secure store before it is advertised. The subsequent iOS implementation uses its own native shell and Keychain adapter, recorded in [ADR-0018](0018-ios-client-and-native-platform-bridge.md).

## Consequences

The development launcher builds static assets before starting the native client. Frontend edits require rebuilding those assets. Each installation has an independent client data directory and Keychain credentials; another installation cannot obtain group history by copying a public configuration file.

macOS Keychain may prompt for access, and losing its credentials prevents opening the encrypted live client state. Backing up the database does not replace the user-held encrypted history export. Signing, notarization, installation packaging, application updates, Windows/Linux secure stores, platform access-control hardening, security review, and production distribution remain follow-up work. An unsigned developer build is not evidence of a production-ready endpoint.

## Implementation and evidence

- Native OAuth PKCE listener and loopback fixture tests: [oauth.rs](../../crates/device/src/oauth.rs).
- History-only export and read-only archive bridge: [backups.rs](../../crates/device/src/backups.rs).
- Local native workflow: [desktop.md](../desktop.md).
- Native shell and transport guards: [apps/desktop/src/lib.rs](../../apps/desktop/src/lib.rs).
- OS credential provider and opt-in real Keychain round-trip test: [secret_store.rs](../../apps/desktop/src/secret_store.rs).
- Bundled asset, CSP, and window configuration: [tauri.conf.json](../../apps/desktop/tauri.conf.json).
- Explicit native save panel and validation tests: [downloads.rs](../../apps/desktop/src/downloads.rs).
- Main window command capability: [main.json](../../apps/desktop/capabilities/main.json).
- Independent device runtime: [crates/device](../../crates/device).
- Development entry point: [start-desktop.sh](../../scripts/start-desktop.sh).
- Primary references used to verify integration APIs: [Tauri capabilities](https://v2.tauri.app/security/capabilities/) and [keyring 3.6.3 Entry](https://docs.rs/keyring/3.6.3/keyring/struct.Entry.html).

Unit tests reject unsupported request methods, arbitrary transport paths, traversal, untrusted navigation, invalid credential names, and empty credentials. Real Keychain testing is explicit opt-in because it may trigger a native prompt. Delivery evidence must distinguish successful compilation and tests from live Cognito/enrollment operation, AWS deployment, signing, notarization, and security audit; those require separate evidence.

The encrypted device profile also pins its original relay URL; editing the public connection file cannot redirect an enrolled device credential. A failed profile write puts the runtime into a fail-closed state until restart, preventing publication from an in-memory-only enrollment or group intent. Native fixture tests cover configuration tampering, failed-save recovery, exclusive stores, exact offline retries after restart, verified cross-store messaging/media, and imported owner transfer.

Initial native joins additionally require the MLS-authenticated Welcome author to equal the pinned invitation owner and the authenticated MLS roster to equal the exact verified invitation roster. This policy runs in the same receive transaction as state/history/cursor persistence. A different known contact cannot impersonate the owner merely by supplying the owner’s public fingerprint. The local multi-account harness retains its documented broader pin set for delayed Welcome fixtures. See [strict Welcome tests](../../crates/client/tests/welcome_authorization.rs).

Explicit native sign-out persists an encrypted device lock across restarts; reopening the app cannot bypass it. The original account must authenticate again to unlock the preserved device store.

CI checks the core/relay workspace on Linux with `cirra-desktop` excluded and
checks/tests the shell in a separate macOS job after building its UI assets; see
[workflow](../../.github/workflows/ci.yml). Native device checkpoint futures retain
an exclusive runtime borrow so they remain Send for Tauri's async executor without
requiring SQLite or the device runtime to be Sync. The
[native executor regression test](../../crates/device/src/tests.rs) exercises this
contract. Browser persistence still awaits its encrypted checkpoint before network
requests; no storage or identity policy is relaxed by this build fix.
