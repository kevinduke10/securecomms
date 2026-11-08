# 0010: Separate account login from verified replacement-device enrollment

- **Status:** Accepted
- **Date:** 2026-11-08
- **Area:** Identity, security, persistence, deployment, interaction
- **Delivery:** Cognito infrastructure/verifier, durable enrollment API, and local browser workflow implemented; AWS rollout and production native clients pending
- **Source:** User requested login followed by replacement-device enrollment. Cognito follows the existing product plan; the protocol, limits, and local test adapter are engineering choices.
- **Related:** Extends [ADR-0003](0003-participant-only-encryption-and-trust.md), [ADR-0004](0004-transactional-client-state-and-recovery.md), and [ADR-0009](0009-user-held-history-backup-files.md).

## Context

The original registry only knew devices with operator-provisioned bearer tokens.
The browser selected any local test account. An account login must establish a
stable user/tenant authorization boundary without recovering private MLS state or
silently giving new devices existing conversation keys.

## Decision

### Account login

Provision a Cognito user pool per installation, with administrator invitations,
email usernames, required software-token MFA, deletion protection, and a public
client supporting only authorization-code flow. Use S256 PKCE and single-use,
browser-bound state for the local client's hosted sign-in. Passwords are handled
by Cognito, not collected or stored by Croaker. Access tokens last five minutes.
Enterprise federation configuration remains separate from this Cognito-user flow.

The Rust relay validates RS256 signatures against the configured issuer's HTTPS
JWKS, issuer, client ID, access-token use, expiration/issued-at bounds, and the
`croaker/access` scope. ID tokens and device bearer credentials cannot substitute
for account access tokens, or vice versa. JWKS responses and token sizes are
bounded; keys cache for one hour, with unknown-key refresh throttled to 30 seconds.
Token-controlled key URLs are never followed. Access tokens may remain valid until
expiry after upstream revocation; offline JWT validation is not instantaneous
Cognito session-revocation detection.

An IAM/operator-only account grant associates `(issuer, subject)` with a tenant,
role, and display name. The account identifier hashes the domain-separated,
length-prefixed issuer and subject. Never assign membership from email, client
parameters, or unverified tenant claims. Grants are create-only and idempotent;
identity reassignment/role editing and account suspension UI need later work.
An optional operator-verified initial public binding must correspond to an already
registered device with the same tenant and role. Existing IAM provisioning remains
an explicit administrator bootstrap path, not a public signup endpoint.

### Replacement enrollment

The client creates a fresh MLS signing identity, device UUID, random device bearer
credential, and encrypted local store. Save these before publishing the enrollment
request. A request carries a stable ID, public binding, label, credential SHA-256
digest, and Ed25519 proof of private-key possession. The proof binds the account ID,
request ID, tenant/device IDs, signing key, label hash, and credential digest using
a versioned domain-separated binary encoding. Private keys and raw credentials
never enter the account/enrollment records or public enrollment responses.

Pending requests expire after ten minutes, with at most five pending requests and
ten recorded approved identities per account. Retries reuse the saved request and
keys. A pending device has no relay credential. An active device already assigned
to that account, or an active administrator in the same tenant, may explicitly
approve or reject it. Require the complete expected fingerprint at approval, and
compare it through a trusted channel. Existing fingerprint computation is shared
with the device-binding format. Approval is not self-service account recovery.

Atomically persist the decision, approved public binding, and credential registry
entries in SQLite or DynamoDB. Recheck approver registration in that transaction;
DynamoDB also uses conditional account-document updates. Duplicate identical
requests and repeated matching approvals are safe; modified requests conflict.
An approved replacement can revoke another device belonging to its own account.
Same-tenant administrators retain their explicit management authority. Relay
revocation is separate from MLS member removal and key rotation.

Approval grants device authorization, not conversation membership. The new client
starts without old ratchets, cursors, outbox, or live history. Existing conversation
allowlists and MLS groups remain unchanged. Restore a user-held backup as a
separate read-only archive, and obtain new conversation invitations independently.
This initial enrollment change did not implement active-group addition.
[ADR-0013](0013-approved-group-admission.md) subsequently adds explicit owner-approved
single-device group admission; general reconciliation and owner transfer remain
unimplemented. Native clients must still pin verified public identities before MLS admission.

### Local browser adapter

Default local startup now requires sign-in. The clearly labeled demo provider
selects Alice, Bob, or Maya without verifying a password; it is not production
authentication. Sessions use random opaque cookies, HttpOnly, SameSite=Strict,
server-side hashed lookup, expiry, and logout invalidation. Demo sessions last
at most eight hours; Cognito-backed local sessions end with the access token and
require reauthentication rather than persisting refresh tokens. The loopback HTTP
cookie exception must never be deployed as a cloud service. Same-origin/local
header checks and Host validation remain in force. OAuth callbacks alone accept
cross-site navigation, checking the canonical Host, state, PKCE, and a short-lived
HttpOnly SameSite=Lax flow cookie. Session state is in local-process memory, so
restarting requires sign-in again; encrypted workspaces persist.

The browser can access only local devices owned by its signed-in account. Demo
owners are stable aliases, including migrated existing profiles. Cognito mode uses
an explicit operator alias-to-subject mapping, never email matching, and disables
the demo login endpoint. An explicitly named test mode can retain the original
unrestricted multi-account playground; it cannot be combined with Cognito mode.

The local replacement UI creates a separate Rust client store and private candidate
manifest, persists the proof before submission, verifies approval before activation,
and atomically adds the approved public binding to the local workspace's trusted
roster. This remains one Mac hosting both devices, not endpoint isolation. Sessions
select only owned, active local clients; another account cannot activate or read
those stores. No production keychain/native packaging is added in this change.

## Rationale and alternatives

Managed login avoids implementing password storage, MFA, or account-password reset
in Croaker. Immutable subject grants prevent email changes or arbitrary login from
creating tenant privileges. Separate account and device credentials prevent stolen
login or history-recovery material alone from decrypting conversations. Explicit
fingerprint approval and signed requests avoid silently trusting a substituted
server-provided signing key. Atomic enrollment prevents an approved UI state from
racing a missing credential, and blocks approval by revoked credentials.

An automatic first-device approval, restoring old device keys from backups,
server-generated client identities, and treating SSO as MLS admission were rejected.
The local process generates keys only in its existing role as the client harness;
the Lambda relay never depends on the client or MLS crates.

## Consequences

Cloud resources are defined but not deployed by this change. Cognito identities,
explicit grants, callback URLs, and verified initial administrators must be
configured for a live installation. Production native clients, platform secure
storage, general group rejoining, enterprise identity-provider federation, global
session invalidation, account lifecycle UI, and automatic S3 recovery remain work.
Demo candidate keys retain the documented private-development-manifest exception.
No independent security audit, compliance claim, or production-readiness claim is
made. Preserve existing client databases during upgrades and replacement tests.

## Implementation and evidence

- [Public proof format](../../crates/protocol/src/enrollment.rs),
  [client signing](../../crates/crypto/src/lib.rs), and
  [account policy](../../crates/relay/src/identity.rs).
- [Cognito verification/API](../../crates/relay/src/login.rs),
  [SQLite](../../crates/relay/src/sqlite.rs), [DynamoDB](../../crates/relay/src/dynamodb.rs),
  [operator CLI](../../crates/cli/src/main.rs), and [Terraform](../../infra/terraform/login.tf).
- [Local login](../../crates/local/src/auth.rs), [replacement client](../../crates/local/src/enrollment.rs),
  [login UI](../../apps/ui/src/Login.tsx), and [device UI](../../apps/ui/src/Devices.tsx).
- [JWT boundary tests](../../crates/relay/tests/login.rs),
  [shared enrollment contract](../../crates/relay/tests/enrollment_contract.rs),
  [local session/PKCE/enrollment tests](../../crates/local/tests/login_enrollment.rs),
  [browser checks](../../apps/ui/tests/login-enrollment.spec.ts), and
  [Terraform safety assertions](../../infra/terraform/main.tftest.hcl).

Subsequent [ADR-0015](0015-independent-desktop-client.md) and [ADR-0018](0018-ios-client-and-native-platform-bridge.md) add native account sign-in, Keychain-backed independent devices, and approved enrollment. Live Cognito acceptance remains pending.
