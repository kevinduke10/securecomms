# Browser client

The hosted browser client reuses the React UI and Rust device engine through
WebAssembly. It does not run the local multi-account harness in AWS. Each browser
profile owns a separate identity, encryption state, encrypted history and outbox.

## Sign in

Open the deployed `web_url` Terraform output in a current HTTPS browser supporting
Web Crypto, IndexedDB and Web Locks. Select **Sign in with business account**, use
your invited Cognito account, then select **Enroll this device**. Existing native
approval does not approve a browser automatically. Compare the complete browser
fingerprint on an approved device and explicitly approve it. After approval, the
browser opens the messaging screen.

The **Devices & security** dialog separates Devices, Contacts, Invitations and
Group changes into tabs. Public identifiers and exported packages have copy
controls; approval still requires independently comparing the complete fingerprint.
Contacts, invitations and group decisions use the explicit verified public
package exchange documented in
[desktop setup](desktop.md). Private keys do not enter those packages. Recovery
files restore read-only history, not a live identity or conversation membership.

Keep only one Cirra tab open per browser profile. Different profiles are separate
devices and require separate approvals. A second tab fails closed instead of
racing the device's ratchet. Reload preserves saved state and pending ciphertext;
requests whose responses were lost retry their exact saved envelopes. Signing out
persists a lock and requires the original account to sign in again.

## Storage and trust

The Rust engine runs in a dedicated worker. Its transactional SQLite state and
private device profile are saved as one encrypted IndexedDB snapshot before relay
requests and after UI commands. A failed save blocks further operations until
reload; the previously committed snapshot remains available. Snapshots currently
have a 128 MiB bound. Large histories or pending files can reach that limit.

A non-extractable Web Crypto key encrypts snapshots and wraps the Rust vault key.
This is browser storage, not a hardware Keychain or protection against malicious
same-origin code. The hosting account and delivered frontend code remain trusted.
Clearing site data, private-browser cleanup, eviction or a missing wrapping key
can lose this device. Existing saved data is preserved on decryption/key errors;
there is no automatic reset. Keep separate user-held history backups.

CloudFront serves bundled code over HTTPS with CSP and no external scripts,
fonts or analytics. The dedicated frontend bucket remains private and separate
from encrypted attachment storage. Authenticated relay requests are proxied under
`/v1` without caching. Cognito callback query strings are not access-logged by the
frontend distribution. There is no service-worker cache.

## Build and publish

Install the pinned Rust toolchain, the wasm32-unknown-unknown target, Zig on macOS,
and `wasm-bindgen-cli` at the version resolved by Cargo.lock. Use an official
release or `cargo install wasm-bindgen-cli --version VERSION --locked`.

```sh
npm ci
bash scripts/build-browser.sh
```

The build script produces Rust/WASM bindings into an ignored generated directory.
Normal local/native UI builds do not require those bindings or a WASM toolchain.
Hosted assets are built separately under `target/web-dist/`, so local/native
builds cannot overwrite a publication in progress. After applying Terraform with `enable_web_ui=true`, publish with:

```sh
bash scripts/deploy-web.sh
```

The publisher reads the existing stack outputs, builds the hosted UI with that
web origin, uploads hashed assets first, and publishes index.html last. It retains
previous assets for already open clients. Defaults use the current `cirra-deploy`
CLI profile and us-east-2; override `CIRRA_AWS_PROFILE`, `CIRRA_AWS_REGION`, or
`CIRRA_AWS_CLI` for another approved installation. Preserve the correct stack state
and verify account identity before applying or publishing.

## Validation

```sh
npm run test:browser
terraform -chdir=infra/terraform test
cargo test -p cirra-device -p croaker-client -p croaker-relay --locked
```

Browser contracts use disposable profiles, real Rust/WASM crypto and a loopback
relay fixture with fake test account tokens. They do not validate Cognito JWTs or
constitute a live-provider security audit. The fixture is an example executable,
never a Lambda dependency or deployment artifact. Actual hosted login/MFA,
physical devices, browser compatibility and S3 acceptance need separate evidence.

## Current deployment evidence

The development frontend is published. Three browser contract tests and all 19
existing UI regression tests pass. A real browser initialized the hosted worker
and WASM with no errors, verified the uncached same-origin relay configuration
and rejected unauthenticated access, and opened the real Cognito sign-in form.
This verifies frontend delivery and provider configuration, not a completed user
MFA/browser approval or live AWS messaging/S3 acceptance test.

## Development test accounts

Three operator-created Cognito test accounts are available in the development
tenant: two members and one restricted guest. Their sign-in names and temporary
passwords are kept in the ignored owner-only file
`data/aws-deployment/mock-accounts.local.json`. No invitation emails were sent;
the reserved example addresses do not receive mail. First sign-in requires a
password change and authenticator-app MFA. Each browser profile must generate
and enroll its own device, with explicit fingerprint approval. These accounts
have no seeded client keys, conversation memberships or message history.

Use separate browser profiles to test simultaneous participants. Additional tabs
in the same profile share one installation and are deliberately blocked. Contacts
and conversation invitations must be exchanged and verified after approval.
