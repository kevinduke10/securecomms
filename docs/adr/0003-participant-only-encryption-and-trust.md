# 0003: Use participant-only MLS encryption with verified device identities

- **Status:** Accepted
- **Date:** 2026-11-07
- **Area:** Security and identity
- **Delivery:** MLS and enrollment backend/local workflow implemented; production clients/federation pending
- **Source:** Explicit user privacy choice and the accepted security design.

## Context

The user selected a model in which only conversation participants can read
contents, including when an organization controls its own AWS account.

## Decision

Use OpenMLS in the Rust client core. Validate key packages and compare credentials
and signing keys with pinned device bindings. A server-supplied name or key is not
by itself verified identity. Keep private message keys and decryption outside the
cloud relay.

Treat account authentication, relay authorization, and cryptographic membership
as separate concerns. Enterprise SSO is planned for account authentication; an
SSO login alone must not silently acquire an existing conversation's keys.
Production enrollment must establish device approval and identity trust.

Administrators can manage tenant access and revoke device credentials, but do not
receive message decryption access. Relay revocation and MLS removal/key rotation
are distinct operations. Clients must explicitly approve relevant membership
changes rather than merging arbitrary commits automatically.

## Rationale and alternatives

OpenMLS provides an existing Rust implementation of a standard group protocol.
The Signal library was considered during planning, but its third-party support and
licensing characteristics made it a poor default for this project. This is not
Signal interoperability or a claim of matching all Signal security properties.
Do not replace the protocol with bespoke messaging cryptography.

## Consequences

The service still observes routing metadata, membership, timing, and approximate
sizes. Verified identities and endpoint security remain necessary. Previously
received content cannot be revoked. The implemented ciphersuite is classical;
post-quantum equivalence, a completed audit, and compliance certification are not
claimed.

The local harness is a documented testing exception to production key isolation;
see [ADR-0005](0005-local-ui-test-harness.md). Real clients still require OS secure
storage and authenticated enrollment.

## Implementation and evidence

See the [Rust crypto core](../../crates/crypto/src/lib.rs), its identity/tamper/
replay/removal tests, and [security boundaries](../architecture.md). [ADR-0010](0010-account-login-and-replacement-device-enrollment.md) adds Cognito
login and approved fresh-device enrollment. Production native key verification,
enterprise federation, deployment acceptance, and complete membership workflows
remain incomplete.
