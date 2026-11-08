# Cirra

Rust foundation for end-to-end encrypted business messaging on serverless AWS.

Cirra is the product name (formerly the Croaker working name). Existing Rust crate
names, CLI commands, configuration identifiers, and backup formats retain their
`croaker` identifiers for compatibility. See [ADR-0011](docs/adr/0011-cirra-product-name.md).

**Status: browser/native development clients and a serverless relay prototype, not a production service.**
The hosted browser client runs the Rust engine through WebAssembly with encrypted
browser storage and independent device enrollment. The default localhost workflow
remains a three-account Rust test harness. Cognito and approved-device enrollment
are implemented; the development relay and web hosting are deployed with live
endpoint/sign-in-page checks. Initial iOS Simulator enrollment was verified.
Full live browser onboarding/messaging, enterprise federation, signed distribution
and production acceptance remain to be completed.
An independent macOS desktop client now uses its own Rust device store and Keychain
vault key; see [desktop setup](docs/desktop.md). It requires a configured relay,
Cognito account grant, and explicit device approval.

Contributors and AI agents: read the [Architecture Decision Records](docs/adr/README.md)
and [AGENTS.md](AGENTS.md) before changing established design choices. Maintain the
ADRs alongside meaningful architecture, security, or interaction changes.

## Open the messaging UI

From this directory, run:

```sh
./scripts/start-local.sh
```

This installs the locked JavaScript dependencies if needed, builds the React UI,
and starts the Rust app. It requires Node 22.12+ and Rust (the pinned toolchain will
be selected automatically). On Windows, use `npm ci` followed by `npm run dev`.

Open **http://127.0.0.1:3000** and choose a clearly labeled local development
sign-in (Alice, Bob, or Maya). To chat between accounts, use a separate browser
profile or private window, since tabs in one browser share the sign-in cookie.
Maya is the guest account. For real Cognito configuration and replacement-device
steps, see [login and enrollment](docs/login-and-enrollment.md).

- Open **Devices** to request a replacement with fresh keys, verify its fingerprint,
  approve it, and activate its separate client. It must be explicitly invited to
  conversations; backup restore does not grant group access. The group owner can
  use **Conversation details → Manage group members** to verify and invite an
  approved device. Only new messages are delivered to that device. The current
  owner can also transfer ownership or remove a member with MLS key rotation.
- Send messages between accounts, or create a group with the compose button.
- Press and hold a message for about half a second to react, like iMessage.
  The popup starts with common emoji; **+** opens a searchable library of over
  1,900 emoji. Right-click or focus a bubble and press Enter for desktop access.
  Matching reactions show a count; choose another emoji to replace yours or click
  your highlighted reaction to remove it. Reactions sync and queue offline.
- Select **Reply** from a message’s long-press or right-click popup. Quotes are
  resolved from authenticated history, including after backup restore.
- Outgoing messages show Sent, Delivered, or Read. Group counts are per device.
  Enable **Share read receipts for this device** in the account menu to share read
  state; it is off by default. Background tabs and obscured content do not mark
  messages read. Already queued/sent receipts are not withdrawn by disabling it.
- Type in one account to see a typing indicator in another. It clears on send,
  blur, or inactivity, with automatic expiry if a tab closes unexpectedly.
- Attach an image or file, up to **25 MiB**. Files up to 32 KiB remain inside MLS;
  larger files use client-side encryption and an immutable object store (local
  fixtures or private S3). Only the encrypted MLS message contains the file key
  and name. Choose Download or Preview to fetch and verify an uploaded file.
- Open the account menu at the bottom left and choose **Go offline**. Queue a
  message, choose **Reconnect**, and watch it arrive in the other tab.
- Search conversations/messages, open conversation details, or resize to try the
  mobile layout. `Ctrl+K` / `⌘K` focuses conversation search.
- Choose **Appearance** in the top bar for Forest, Ocean, Rose, Sand, or Midnight.
  Your theme is saved in this browser and shared across its account tabs.
- Open **Backups** in the top bar to generate a recovery code, confirm it, and
  download an encrypted `.crkbackup` history file (up to 16 MiB). Keep the code
  separately. **Restore backup** imports a searchable, read-only archive into the
  selected test account, including in a fresh workspace. Uploaded files retain an explicit name/size
  omission record; their contents are excluded. Keep those files separately. Live conversations and
  keys are unchanged; this does not log in or rejoin groups. Pending messages and
  typing are excluded. Preserve original backup files; restored archives are not
  included in later exports. Automatic S3 backup is not connected.
- Refresh or restart: the same local workspace and encrypted history are retained.

Stop with `Ctrl+C` in the terminal that started the app. To use a different port
or a separate fresh workspace without deleting anything:

```sh
npm run dev -- --port 3001 --data-dir data/ui-another
```

The default workspace lives in `data/ui/`, which is ignored by Git. **Use test
data only:** the local Rust process owns all three test accounts. Its private
`workspace.local.json` file contains development credentials and vault keys;
browser tabs are not isolated production devices. The relay stores ciphertext,
and client history/state are encrypted locally. This harness binds only to
loopback and must never be deployed as a cloud service.

Group changes retain one previous MLS epoch for delayed ciphertext. Older queued
updates can become undecryptable after multiple membership changes; clients skip
only well-framed old-epoch applications, record a durable warning, and continue
syncing. They never re-encrypt an ambiguous saved send or fabricate delivery.
See [ADR-0013](docs/adr/0013-approved-group-admission.md) for this retention tradeoff.

The UI source is in `apps/ui/`. After source edits, restart `npm run dev` to rebuild.
There is no Vite dev server or second port required.

## Open the macOS desktop client

```sh
./scripts/start-desktop.sh
```

This builds bundled assets and opens the native client. Configure your business
relay, sign in, and request device approval. It uses no demo accounts. Contacts,
invitations, and owner decisions currently require explicit public-package exchange
and fingerprint verification. See [desktop setup](docs/desktop.md) for prerequisites,
first-device bootstrap, Keychain behavior, and limitations. No AWS resources are
created by this launcher.

## Build the iOS app

```sh
./scripts/build-ios.sh simulator
open apps/ios/Cirra.xcodeproj
```

The iPhone/iPad app shares the Rust engine and React UI, with iOS Keychain,
system account sign-in, and a native Files picker. It requires full Xcode and
an iOS simulator runtime. Configure a business HTTPS relay and explicitly approve
the device. See [iOS setup](docs/ios.md) for signing, callbacks, and limitations.

## Try it locally

Rust is pinned to **1.99.1** in `rust-toolchain.toml`. No AWS account is needed.

```sh
cargo run -p croaker-cli -- demo
```

The demo starts a loopback HTTP relay backed by SQLite, creates a three-device MLS
group (including a guest), sends encrypted messages, exercises an idempotent retry,
restores an offline device from an encrypted vault, and verifies revocation and
group key rotation. Its relay data is saved to `data/demo.sqlite`; client keys are
ephemeral and the demo is not an interactive chat application.

For a separately running local relay:

```sh
cargo run -p croaker-cli -- init-demo
cargo run -p croaker-cli -- provision --credentials data/devices.local.json
cargo run -p croaker-cli -- serve
# In another terminal:
cargo run -p croaker-cli -- demo --url http://127.0.0.1:3000 --credentials data/devices.local.json
```

The demo revokes its second credential. Generate and provision fresh demo
credentials before running it again against the same relay. Credential files are
created with restricted permissions and ignored by Git. Do not paste them into logs.

## What works

- OpenMLS encryption with pinned device identities, direct/group membership,
  authenticated welcomes, replay/tamper rejection, and authorized member removal.
- Cognito access-token verification and invite/MFA infrastructure; tenant grants
  and atomically approved public device identities in SQLite/DynamoDB.
- Local sign-in sessions, account-scoped API access, replacement approval and
  activation, and same-account device revocation.
- Encrypted device-state snapshots and an atomic local send outbox: ciphertext and
  the advanced MLS ratchet commit together before network delivery.
- Encrypted local history with atomic inbound ratchet/history/cursor commits.
- Owner-approved group admission with atomic MLS commit/welcome delivery, pinned
  identities, durable retries, and no automatic transfer of prior history.
- Explicit owner transfer/removal with exact saved commits, atomic relay changes,
  and pinned recipient authorization.
- Encrypted 25 MiB object attachments with scoped uploads/downloads, quotas, and
  private S3 infrastructure; local fixtures require no AWS.
- Independent macOS Tauri client, Keychain-held vault key, native OAuth PKCE,
  verified contact exchange, and native file save dialogs.
- Encrypted replies and delivery/read receipts with per-device privacy controls.
- User-held recovery codes, encrypted portable history exports, and isolated
  read-only archive restore, with wrong-key/tamper rejection and encrypted storage.
- Responsive React UI with multi-account local testing, groups, file attachments,
  offline queueing, conversation search, emoji reactions, typing indicators, and
  guest restrictions.
- Tenant-scoped relay authorization, explicit guest access, device credential
  revocation, durable sequence ordering, pagination, and idempotent sends.
- Interchangeable SQLite and DynamoDB stores using the same behavioral tests.
- An AWS Lambda HTTP entry point and Terraform for API Gateway, DynamoDB,
  least-privilege function permissions, logs, an error alarm, and a $200 budget.

The relay stores ciphertext and routing metadata; it never receives chat keys.
Infrastructure encryption at rest is separate from the client-side MLS encryption.

## Workspace

| Crate | Responsibility |
|---|---|
| `croaker-protocol` | Versioned request/response types, identifiers, and limits |
| `croaker-crypto` | Client-only MLS, pinned identities, encrypted device vaults |
| `croaker-client` | Transactional encrypted state and outgoing message queue |
| `croaker-relay` | HTTP API, authorization, SQLite/DynamoDB storage, Lambda |
| `croaker-cli` | Local server, credential provisioning, end-to-end demonstration |
| `cirra-device` | Independent per-installation Rust client engine and verified relay transport |
| `cirra-ios-bridge` | Narrow C ABI for the iOS native shell and shared Rust engine |
| `apps/ios` | SwiftUI, restricted WKWebView, iOS Keychain, system sign-in, and Files export |
| `cirra-desktop` | Bundled macOS Tauri shell and Keychain/file-dialog adapters |
| `croaker-local` | Loopback-only UI bridge and persistent multi-account test workspace |
| `apps/ui` | React/TypeScript interface, local launcher, Playwright browser tests |

## Checks

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-features --locked
npm ci
npx playwright install chromium
npm run test:auth # Login, sessions, and device enrollment in an isolated workspace.
npm run test:ui # Starts an isolated temporary workspace on port 3107; requires no AWS.
bash scripts/test-dynamodb.sh # Requires Docker and AWS CLI; uses dummy local credentials.
terraform -chdir=infra/terraform init -backend=false
terraform -chdir=infra/terraform validate
terraform -chdir=infra/terraform test # Mock provider; creates no cloud resources.
```

## Hosted browser client

See [browser setup, storage boundaries, build and publication](docs/web.md). The
hosted build runs the shared Rust engine through WebAssembly; it does not deploy
the local test harness. Browser devices require their own verified enrollment.

## AWS development deployment

See the [operator login, deployment runbook, and idle-cost assumptions](docs/aws-deployment.md).

This deploys the prototype relay, not the complete product. Use a development
account. Provisioning uses your operator credentials; there is no public endpoint
that can mint an account, tenant, or device credential.

```sh
cargo install cargo-lambda --locked
bash scripts/build-lambda.sh
terraform -chdir=infra/terraform init
terraform -chdir=infra/terraform plan \
  -var="lambda_zip=$(pwd)/artifacts/croaker-lambda.zip" \
  -var="budget_email=you@example.com" \
  -out=deployment.tfplan
terraform -chdir=infra/terraform apply deployment.tfplan
```

`api_url` is the native client's Business relay URL; `table_name` is the operator
provisioning target. Create invited Cognito accounts and immutable subject/tenant
grants using [login and enrollment](docs/login-and-enrollment.md), then verify and
explicitly approve native devices. Keep the demo credentials and local workspace
out of the deployed installation.

The same Terraform supports a hosted installation with multiple tenant IDs or a
dedicated installation in a customer's AWS account. No central licensing service
is required. Store Terraform state securely before sharing deployment access.
The table has deletion protection; explicitly disable it only for intentional
teardown. The budget is **account-wide and alerts only; it is not a spending cap**.
Without `budget_email`, it records the budget but sends no email notifications.

## Next product milestones

Track actionable follow-ups in the [todo list](docs/todo.md), including simpler
first-device onboarding and QR-based additional-device approval.

1. Acceptance-test live Cognito login and verified enrollment, add enterprise
   federation, invitations/admin UI, seat limits, and account lifecycle management.
2. Signed/notarized native distribution, additional platform secure stores,
   automated verified contact/decision transport, and APNs/FCM delivery. macOS
   and iOS clients share isolated device stores and explicit enrollment.
3. Cloud backup automation/account recovery, indexed history search, and
   disappearing messages. Group admission, owner transfer/removal, encrypted
   object media, replies, and receipts have local/native development flows; live
   AWS/S3/Cognito acceptance remains pending.
4. Independent security review, platform distribution, measured load/cost tests,
   operational recovery drills, and commercial release.

Video calls follow through a separate WebRTC media plane with participant-only
encryption. This prototype does not implement calling or streaming.

See [architecture and security boundaries](docs/architecture.md) and the
[HTTP contract](docs/api.openapi.yaml).
