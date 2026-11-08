# Cirra desktop client

The macOS desktop client runs its Rust device engine inside Tauri. It does not start `croaker-local` or use the local demo accounts. It needs a reachable relay, an account permitted in that relay's tenant, and explicit device approval. AWS deployment and live Cognito login are separate setup steps; this repository does not supply a running production service.

## Run on macOS

Install the repository's pinned Rust toolchain, Node.js, npm dependencies, and Xcode command line tools. From the repository root:

```sh
npm ci
./scripts/start-desktop.sh
```

The launcher builds the React interface, then runs `cargo run -p cirra-desktop`. Assets are bundled in the binary; it starts no Vite development port. Rebuild and restart after frontend edits. This is an unsigned development executable, not a signed or notarized installer. A release executable can be compiled with:

```sh
npm run build:ui
cargo build -p cirra-desktop --release
```

Current OS secure storage support is macOS Keychain. Windows and Linux builds must not substitute plaintext files or the keyring mock backend; they need an explicit native provider implementation and validation first.

## Configure login and enrollment

1. Configure the relay URL in the desktop setup panel. Use HTTPS. Plain HTTP is allowed only for loopback test fixtures. This installation cannot switch relays after enrollment.
2. Configure the relay's Cognito login provider. Its public `GET /v1/login/config` response supplies `issuer`, `client_id`, `domain`, and `scope`. Use a public app client with authorization-code grant and S256 PKCE. Never configure a Cognito client secret in the desktop client. Permit the required API scope (currently `croaker/access`) and `openid`.
3. Register this exact allowed Cognito callback URL:

   ```text
   http://localhost:38317/callback
   ```

   The native listener binds only `127.0.0.1:38317`, validates the callback Host, path, random state, and single-use authorization code, and expires after five minutes. That port must be available. The account login page opens in your system browser; the embedded UI cannot navigate to arbitrary websites.
4. Sign in. The native client exchanges the code and PKCE verifier directly with the provider. The renderer never receives OAuth access/refresh tokens or the verifier. The relay independently verifies the account token and its operator-managed tenant grant.
5. Request enrollment for this installation. Share the displayed account ID and full device fingerprint with an existing approved same-account device or authorized tenant administrator using a trusted channel.
6. The approver inspects the pending request, compares the full fingerprint, and approves it. Account login alone does not approve a new endpoint. Refresh the candidate's status after approval. Expired requests can be renewed without replacing its candidate keys.

For the first device in a tenant/account, an operator must provision the tenant account grant and an authorized approval/bootstrap route according to the existing enrollment documentation. The desktop client does not create a tenant administrator automatically. See [ADR-0010](adr/0010-account-login-and-replacement-device-enrollment.md) and the [README account login instructions](../README.md).

## Exchange contacts and conversation invitations

This first independent client uses manual public contact exchange. It does not include an unverified public directory or automatic identity trust.

Export your contact bundle from the desktop setup tools and share it with another approved participant. This bundle contains public device identity information and an MLS KeyPackage, not private keys. Compare the full fingerprint using a trusted channel, then import the contact with that fingerprint. Every participant must belong to the same tenant. An existing pinned identity cannot be silently changed by importing a new bundle.

Export a fresh contact package before each new conversation; MLS KeyPackages are consumed when joining. For a conversation, the owner creates it with verified contacts. Share the owner's conversation invitation with the other participants. Before importing, each participant must import and verify the owner's contact and every roster contact. The invitation is checked against those pinned identities and the intended roster. The MLS Welcome must also be signed by that exact owner and contain that exact roster. Exchange initial invitations before later group changes; delayed participants need the original invitation followed by verified decisions in order. Receiving an invitation does not bypass MLS Welcome validation or grant previous message history. Public contact and invitation bundles still reveal membership metadata; share them deliberately.

## Change group access

Open a group's conversation details and choose **Manage group members**. Only its current owner can add a verified approved device, transfer ownership to another member, or remove a member. Compare the target's full device fingerprint before submitting. Ownership transfer rotates the group epoch; removal rotates keys and removes that device's relay access. Previously received messages and cached media remain on the removed device.

After an accepted change, export its group decision packet in the desktop setup tools and distribute it to the remaining participants using a trusted channel. Each recipient separately verifies all involved contacts, compares the approving owner's full fingerprint, and imports that packet. Import preserves the existing owner and roster until that client's MLS engine authenticates the exact commit. The newly admitted device can import its admission packet to establish the new group and its delivery floor; it does not gain previous messages. Import pending changes in order and sync between them. Account administration and relay roster metadata cannot authorize these changes by themselves.

A lost owner cannot be replaced automatically: transfer requires the current owner's keys. A previously removed identity cannot rejoin using stale MLS state; enroll a fresh device identity and admit it explicitly. A removed independent device can retain cached history and show a sync access error because the relay excludes it before delivering its removal commit. See [ADR-0016](adr/0016-group-owner-transfer-and-removal.md).

## Save media and backup files

Desktop downloads open the native macOS save panel. Select a destination and confirm any overwrite there; canceling leaves the destination untouched. The renderer supplies a basename and bounded file contents, never a filesystem path. Newly created files use owner-only permissions (`0600`). Downloaded media is plaintext once you explicitly save it; exported `.crkbackup` files remain encrypted and still require their recovery code. The command permits at most 27 MiB decoded data (40 MiB encoded), covering current attachments and history backups.

## Device data and recovery

On macOS, the default application data directory is `~/Library/Application Support/app.cirra.desktop`. It is separate from the local demo workspace. The runtime restricts directory/file permissions and holds an exclusive installation lock while open. Starting another process on the same data directory fails.

The installation ID and relay connection settings are public metadata. The original relay URL is also pinned inside the encrypted device profile; changing the public file after enrollment fails closed. A failed profile write blocks further operations until storage access is corrected and the client restarts. The encrypted client database, encrypted device profile, and installation lock remain on disk; the random vault key stays in macOS Keychain under the `app.cirra.desktop` service. Device credentials and MLS private state never go into browser localStorage or a plaintext manifest. Keychain access may prompt. If access is denied or a key is missing for existing encrypted data, the client preserves the data and fails instead of generating a replacement key over it.

Do not copy a running device directory to create another device or synchronize it with a cloud drive. A replacement installation must generate a fresh identity and request approval. The installation lock prevents simultaneous use of one directory; it cannot detect arbitrary copied directories on another machine. User-held encrypted history backups restore as separate read-only archives, not live device identities or ratchets. Small inline attachments remain included. Larger encrypted object attachments retain only their name, MIME type, and size with an explicit omission indicator; object locators, keys, and media bytes are excluded from history exports. These archive extensions require an updated client to read. Keep their recovery codes separately from the backup files. See [ADR-0004](adr/0004-transactional-client-state-and-recovery.md).

## Checks and limits

Run desktop checks on macOS after `npm ci` and `npm run build:ui`; Tauri embeds
those assets at compile time. CI uses a separate macOS desktop job and excludes
the shell from Linux core checks. This does not add Linux desktop support or a
fallback credential store.

```sh
cargo check -p cirra-desktop
cargo test -p cirra-desktop --lib
cargo test -p cirra-device
```

The default shell tests exercise IPC and navigation boundaries without touching real Keychain entries. The explicit test below creates and deletes a uniquely named test credential, and may trigger a macOS Keychain prompt:

```sh
cargo test -p cirra-desktop --lib keychain_roundtrip -- --ignored
```

Native OAuth tests use a loopback provider fixture and verify state, Host validation, S256 PKCE exchange, and one-use behavior. These tests do not establish live Cognito operation, AWS deployment, signed distribution, security audit, or cross-platform support. See [ADR-0015](adr/0015-independent-desktop-client.md) for the security boundaries and remaining work.

Explicit native sign-out persists an encrypted device lock across restarts; reopening the app cannot bypass it. The original account must authenticate again to unlock the preserved device store.
