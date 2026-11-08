# Cirra for iPhone and iPad

The iOS app uses the same Rust device engine and bundled React messaging interface
as the independent desktop client. Swift supplies the iOS Keychain, system account
sign-in, a restricted WKWebView bridge, and a Files export picker. It does not run
the multi-account local demo or receive other devices' live keys.

## Build and run

Use macOS with full Xcode, an installed iOS SDK/simulator runtime, the repository's
pinned Rust toolchain, and Node 22.12+. From the repository root:

```sh
./scripts/build-ios.sh simulator
open apps/ios/Cirra.xcodeproj
```

The script builds the UI and compiles an ad-hoc signed simulator application
(needed for simulator Keychain access). Select the Cirra scheme and an iPhone/iPad
simulator in Xcode, then Run.

Xcode's first build phase automatically builds Rust for the selected destination:
Intel simulator (`x86_64-apple-ios`), Apple Silicon simulator
(`aarch64-apple-ios-sim`), or physical iPhone/iPad (`aarch64-apple-ios`). It installs
a missing Rust target and stages the resulting library in
`$(BUILT_PRODUCTS_DIR)/cirra-rust/`, which the linker searches. Switching between
simulators and devices no longer requires manually changing library paths or
prebuilding a different target. Multi-architecture simulator builds combine only
simulator slices.

Cargo's cache stays in `target/ios/<target>/release/`; the wrapper's Xcode products
stay in `target/ios/xcode/`. Rust edits rebuild automatically in Xcode. After UI
edits, run `npm run build:ui`; Xcode bundles `apps/ui/dist/` without starting a web
server. Xcode must be able to access Rust, normally installed in `~/.cargo/bin`.

For an unsigned physical-device build:

```sh
./scripts/build-ios.sh device
```

To install on a physical device, open the Xcode project and select your own signing
team and provisioning profile. Device signing, paid developer enrollment, App Store
metadata, export compliance, TestFlight and release distribution are separate work.
No developer team, certificate, provisioning secret or production account is
committed to this repository. This app is not an App Store release.

## Configure an account

Use a reachable HTTPS business relay. An iPhone cannot reach a relay using the
Mac's `localhost`; use a correctly configured HTTPS development endpoint. The
native engine permits HTTP only for literal loopback fixtures, and iOS sign-in
requires a provider HTTPS URL. Do not expose or deploy `croaker-local` to solve
mobile connectivity.

Register this exact Cognito allowed callback for the iOS client:

```text
app.cirra.ios:/oauth/callback
```

The relay supplies its public issuer/client/domain/scope configuration. **Sign in
with business account** opens `ASWebAuthenticationSession`, outside the messaging
webview. Rust holds the random OAuth state and S256 PKCE verifier. The native
session returns its custom-scheme callback directly to Rust, which checks its exact
scheme/path, one matching state/code, five-minute expiry, and single-use status
before exchanging it. Access/refresh tokens never enter JavaScript. The relay then
verifies the account access token and original-account identity independently.
Canceling sign-in or signing out discards the pending flow. Sign-out persists an
encrypted device lock across restarts. A successful sign-in to the original
account is required to unlock saved history.

Enroll the installation, compare its complete fingerprint with an approved
same-account device or tenant administrator through a trusted channel, and approve
it there. The first administrator still needs the operator bootstrap described in
[desktop setup](desktop.md) and [login setup](login-and-enrollment.md). Account
login alone cannot approve a fresh device or restore conversation access.

After approval, use **Devices & security** to exchange verified contact packages,
conversation invitations and owner decision packets as described in
[desktop setup](desktop.md). All participant fingerprints must be verified.
The shared UI retains message long press, quick reactions and searchable emoji,
replies, typing, receipt preferences, themes, groups and file attachments. The
initial iOS app uses foreground cursor sync; APNs delivery/background messaging
and calls are not implemented.

## Storage and exports

Each installation keeps one encrypted Rust SQLite store and encrypted private
profile under its Application Support directory. The directory is excluded from
iCloud/device backups: copying a live ratchet store is not supported recovery.
The random vault key is a non-synchronizing generic-password Keychain item using
`kSecAttrAccessibleWhenUnlockedThisDeviceOnly`. Missing or denied Keychain access
preserves existing data and fails closed. iOS Keychain does not migrate that key
to a replacement device. Each replacement must enroll a fresh identity.

A nonpersistent WKWebView serves only application-bundled files at
`cirra-app://bundle`. It cannot navigate to another origin, open arbitrary URLs,
make arbitrary network calls, or access arbitrary filesystem paths. Messages to
native code are accepted only from the bundled main frame. Theme/unread UI
preferences in web storage last for the current webview lifetime; message history,
crypto state, receipt preferences, pins and pending sends persist in Rust.
The app covers its UI while inactive to reduce app-switcher exposure. Native
foreground/overlay state also gates read receipts and stops typing while the app
is inactive or a native account/file picker obscures messages. The native bridge
also rechecks presented controllers before read/active-typing requests, including
WebKit’s built-in attachment chooser. Permission
usage strings cover the system attachment picker’s photo/video capture options;
they do not request access on startup or implement video calling.

Backups and verified attachments use the native Files export picker. The webview
supplies a bounded basename and bytes; it cannot choose an output path. Export
staging files use iOS complete file protection and are deleted after picker
completion/cancellation. A crash can leave a protected temporary staging file until
the system purges temporary storage. Keep user-held recovery codes separate from
the encrypted history file. Large uploaded file contents are explicitly omitted
from history backups; keep those files separately.

## Validation

```sh
cargo test -p cirra-ios-bridge -p cirra-device
./scripts/build-ios.sh simulator
# Choose an available simulator UUID from `xcrun simctl list devices available`.
xcodebuild -project apps/ios/Cirra.xcodeproj -scheme Cirra \
  -destination 'platform=iOS Simulator,id=YOUR_SIMULATOR_UUID' \
  -derivedDataPath "$PWD/target/ios/xcode" \
  CODE_SIGNING_ALLOWED=YES CODE_SIGN_IDENTITY=- test
```

The selected simulator determines the Rust architecture automatically. The scheme's
test action enables an isolated fresh fixture device directory; it never resets
your regular installation. Swift tests check origin/filename guards, actual
bundled React loading, and a native status request. Rust tests check narrow FFI
routing and both desktop/iOS OAuth PKCE/callback boundaries. These checks do not
establish live Cognito enrollment, physical-device Keychain behavior, background
notification delivery, App Store acceptance, a security audit, or production AWS
readiness. See [ADR-0018](adr/0018-ios-client-and-native-platform-bridge.md).
