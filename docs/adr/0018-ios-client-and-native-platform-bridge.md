# 0018: iOS client with shared Rust engine and a native platform bridge

- **Status:** Accepted
- **Date:** 2026-11-08
- **Area:** Architecture / security / persistence / platform interaction
- **Delivery:** Implemented iOS development client; simulator validation below; physical-device/provider/distribution acceptance pending
- **Source:** User explicitly requested an iOS app implemented by a subagent. Swift shell, C ABI, callback scheme, platform permissions and minimum iOS version are engineering choices.
- **Related:** [0003](0003-participant-only-encryption-and-trust.md), [0004](0004-transactional-client-state-and-recovery.md), [0006](0006-message-reactions-and-emoji-interaction.md), [0010](0010-account-login-and-replacement-device-enrollment.md), [0015](0015-independent-desktop-client.md), [0016](0016-group-owner-transfer-and-removal.md), [0017](0017-client-encrypted-object-attachments.md).

## Context

An independently installed phone needs its own private device identity, encrypted
store, enrollment and verified group membership. Wrapping the local development
server would still put every account's private keys on one Mac. The desktop's
fixed loopback OAuth callback and macOS save dialog cannot be assumed to work on
an iPhone, whose app is suspended during browser navigation.

## Decision

Use a small SwiftUI/UIKit platform shell around the existing bundled React UI and
`cirra-device` Rust engine. A versioned, narrow C ABI owns one Rust runtime/device
handle, confined to a serial native queue. Keep MLS, encrypted persistence, exact
outbox retries, verified identities, receipt/reaction projection, media decryption,
and history recovery in the shared Rust implementation. The initial platform
minimum is iOS 16. This extends the shared-core product direction without claiming
that the macOS Tauri platform adapter supplies iOS behavior.

Serve immutable application assets with a WKURLSchemeHandler at
`cirra-app://bundle`. Accept only main-frame messages whose native frame request
and webview URL belong to that bundle origin. Native commands are limited to
whitelisted device requests, argument-free system login, platform information and
explicit Files exports. Block external navigation, frames, remote scripts and
arbitrary renderer network/filesystem/process access. The renderer receives
plaintext display history and verified file bytes but no vault key, device bearer
credential, OAuth verifier/token, MLS private state or secret media descriptor.
Preserve the established message-hold/searchable-emoji interaction in the shared UI.

Store the random vault key using non-synchronizing iOS Keychain items with
`kSecAttrAccessibleWhenUnlockedThisDeviceOnly`, with no plaintext/mock fallback.
Exclude the live client directory from device/iCloud backup and preserve it when
Keychain access fails. A replacement installation enrolls a new identity; user-held
history files remain separate read-only archives. Cover the app while inactive.
Signal native foreground/occlusion changes to the UI so inactive apps and native
authentication/file pickers cannot create read receipts or leave typing active.
Recheck presented native controllers at the bridge before read/active-typing
claims, covering WebKit’s built-in upload chooser as well as explicit exports.

Use ASWebAuthenticationSession and the registered callback
`app.cirra.ios:/oauth/callback`. Rust retains state/PKCE, validates the exact callback
scheme/path and a single matching state/code within five minutes, consumes the
flow once, and performs bounded token exchange. Swift passes the callback directly
to Rust; it is not a JavaScript message or an app-supplied arbitrary opener. The
relay still validates the account and explicit device enrollment. The existing
macOS loopback login path remains supported and tested.

Exports accept a bounded safe basename and up to 27 MiB of bytes. Stage a protected
temporary file and present UIDocumentPicker for an explicit user-selected copy.
The webview never supplies an arbitrary destination path. Delete staging files on
completion/cancellation. No automatic cloud backup is introduced.

## Rationale and alternatives

A small Swift adapter exposes Apple's Keychain, authentication-session and document
picker boundaries directly while retaining one Rust implementation and one UI.
A second messaging/crypto implementation in Swift would duplicate ratchet and
persistence logic. A browser-only remote shell would weaken installed device
isolation. A loopback callback cannot rely on the app remaining active during
mobile browser sign-in. Automatic directory trust or silent key migration would
bypass the existing fingerprint/enrollment model.

## Consequences

Native project builds require full Xcode and a target-specific Rust static library.
Simulator builds use ad-hoc signing and the application’s own Keychain entitlement;
an unsigned simulator binary correctly fails Keychain access rather than falling
back to plaintext. The build sets the Rust C dependencies’ iOS deployment minimum
explicitly to 16.0. The app icon derives from the existing Cirra SVG.
Physical devices require the developer's own team/signing configuration. The app
currently syncs while foregrounded; APNs/background delivery, calls, dedicated camera UI,
biometrics policy, signed distribution, App Store/privacy/export review and
production acceptance remain future work. A nonpersistent webview resets theme and
unread UI preferences when recreated; durable messaging data stays in Rust.
Manual contact/invitation/group-decision exchange follows ADR-0015/0016. Keychain
protection and an app-switcher cover do not protect a compromised unlocked endpoint.
A crash during export can leave protected temporary data until system cleanup.

## Implementation and evidence

- [Swift platform shell and origin guard](../../apps/ios/Cirra/CirraViewController.swift), [bundled assets](../../apps/ios/Cirra/BundledAssets.swift), [Keychain adapter](../../apps/ios/Cirra/KeychainStore.swift), and [native queue](../../apps/ios/Cirra/DeviceEngine.swift).
- [Restricted C ABI](../../crates/ios-bridge/src/lib.rs), [platform login hooks](../../crates/device/src/ios.rs), and [shared OAuth](../../crates/device/src/oauth.rs).
- [Xcode destination-aware Rust build phase](../../scripts/build-ios-rust.sh), [Xcode project](../../apps/ios/Cirra.xcodeproj/project.pbxproj), [Swift checks](../../apps/ios/CirraTests/BoundaryTests.swift), [build script](../../scripts/build-ios.sh), and [iOS instructions](../ios.md).
- Primary API references: [Apple authentication sessions](https://developer.apple.com/documentation/authenticationservices/authenticating-a-user-through-a-web-service), [Keychain accessibility](https://developer.apple.com/documentation/security/restricting-keychain-item-accessibility), and [native frame information](https://developer.apple.com/documentation/webkit/wkframeinfo).

The x86_64 iOS simulator Rust static library compiles with the iOS 16 deployment
minimum. Three Swift checks passed on an isolated iPhone 16 Pro simulator running
iOS 18.3.1: bundled React setup renders, real Keychain-backed Rust status IPC
responds, and renderer OAuth access/history/origin/filename guards behave as
expected. Native inactive/active notifications also toggle the renderer activity
flag and its read/typing gate; native controllers are rechecked independently at
the request bridge. Rust host fixtures cover both desktop and iOS S256 exchanges and callback
validation. Compilation, simulator checks and actual-provider tests are distinct. No live AWS deployment, signed physical-device build,
App Store submission or independent security audit has been completed here.

Build integration correction: Xcode now builds Rust for its actual SDK and architecture list before linking, staging a library in the current Xcode products directory. Simulator and device libraries remain separate; universal simulator builds combine simulator slices only. This removes the previous requirement that a manually prebuilt target already exist when opening Xcode or switching destinations.

Destination-aware build validation: clean Xcode product directories successfully
built and linked the x86_64 simulator app and unsigned arm64 iPhone app without a
Rust-target override. The staged archives were checked for their requested CPU
architecture. Device signing and physical-device runtime acceptance remain separate.
