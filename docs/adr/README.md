# Architecture Decision Records

These ADRs preserve the reasoning behind Cirra's architecture and significant
product/interaction decisions so future engineers and AI contributors can continue
without reconstructing the chat history.

## Reading guide

Start with ADR-0001, then choose records relevant to the work. Read ADR-0003 and
ADR-0004 before changing encryption, authentication, delivery, or persistence.
Read ADR-0005 before working on the local app, and ADR-0006/0007 before changing
reactions or typing indicators. Read ADR-0008 before changing appearance themes, ADR-0009 before changing backups, and ADR-0010 before changing login/enrollment. Read ADR-0012 before adding interface copy.

| Record | Decision | Status | Delivery |
|---|---|---|---|
| [0001](0001-product-and-commercial-scope.md) | Business product, hosted/dedicated deployment, and launch scope | Accepted | Product direction; partially implemented |
| [0002](0002-rust-core-and-serverless-aws.md) | Shared Rust core and serverless AWS relay | Accepted | Foundation implemented; production rollout pending |
| [0003](0003-participant-only-encryption-and-trust.md) | Participant-only encryption, MLS, and identity trust | Accepted | MLS/enrollment foundation implemented; native clients/federation pending |
| [0004](0004-transactional-client-state-and-recovery.md) | Transactional cryptographic state, history, and safe recovery | Accepted | State/history and portable recovery implemented; cloud recovery pending |
| [0005](0005-local-ui-test-harness.md) | React UI with a loopback Rust testing harness | Accepted | Implemented development workflow |
| [0006](0006-message-reactions-and-emoji-interaction.md) | Bubble long press, quick reactions, and searchable emoji | Accepted | Implemented local UI |
| [0007](0007-encrypted-reactions-and-typing-events.md) | Encrypted reaction state and transient typing indicators | Accepted | Implemented local app/core support |
| [0008](0008-browser-appearance-themes.md) | Semantic appearance palettes and browser-local theme preferences | Accepted | Implemented local UI |
| [0009](0009-user-held-history-backup-files.md) | User-held encrypted history files and isolated read-only restores | Accepted | Implemented portable/local workflow; cloud backup automation pending |
| [0010](0010-account-login-and-replacement-device-enrollment.md) | Cognito login and independently approved replacement identities | Accepted | Backend/local workflow implemented; production clients pending |
| [0011](0011-cirra-product-name.md) | Cirra branding with stable data/protocol identifiers | Accepted | Implemented display rename |
| [0012](0012-functional-interface-copy.md) | Direct functional UI copy without taglines or slogans | Accepted | Implemented local UI |
| [0013](0013-approved-group-admission.md) | Explicit pinned group admission and atomic MLS commit/welcome delivery | Accepted | Implemented local/client/relay workflow |
| [0014](0014-encrypted-replies-and-receipts.md) | Authenticated replies and encrypted delivery/read receipt state | Accepted | Implemented local app/core flow |
| [0015](0015-independent-desktop-client.md) | Independent macOS Rust/Tauri client, Keychain, and guarded native transport | Accepted | Development client implemented; live deployment/distribution pending |
| [0016](0016-group-owner-transfer-and-removal.md) | Explicit owner transfer and member removal with MLS key rotation | Accepted | Local/native client and SQLite/DynamoDB relay flow implemented |
| [0017](0017-client-encrypted-object-attachments.md) | Client-encrypted object attachments and private S3 delivery | Accepted | Local/native flow and AWS infrastructure implemented; live S3 acceptance pending |
| [0018](0018-ios-client-and-native-platform-bridge.md) | iOS native shell, device-only Keychain, and shared Rust engine | Accepted | Implemented; iOS Simulator build and native bridge tests pass |
| [0019](0019-aws-development-deployment-preparation.md) | Browser-authenticated AWS deployment preparation and low idle-cost assumptions | Accepted | Deployed in us-east-2; live endpoint smoke checks pass; native acceptance pending |

| [0020](0020-browser-client-and-web-hosting.md) | Rust/WASM browser client, encrypted IndexedDB snapshots, and private S3/CloudFront hosting | Accepted | Published; browser crypto contracts and live sign-in-page checks pass |

| [0021](0021-responsive-device-security-tools.md) | Responsive tabbed security tools and wrapping/copyable enrollment identifiers | Accepted | Published web/iOS Simulator UI; responsive and approval checks pass |

“Accepted” describes a design decision, not a claim that its entire production
implementation is finished. Delivery labels and each record's evidence make that
distinction explicit. Current behavior is also documented in
[architecture.md](../architecture.md) and the [project README](../../README.md).

## Adding or changing a decision

1. Copy [template.md](template.md) to `NNNN-short-title.md` using the next number.
2. Record the requirement, decision, reasons, alternatives, consequences, delivery
   status, and relevant source/tests. Keep each record focused on one decision.
3. Add it to this index. Use `Proposed`, `Accepted`, or `Superseded` for status.
4. When replacing an accepted approach, preserve the old record, link the new
   record from it, and identify the superseded record in the replacement.

Initial records were reconstructed on 2026-11-07 from the user's instructions,
accepted implementation plan, and repository evidence. They are not a transcript
or proof of production readiness. Avoid putting secrets or private message data
in this directory. Future contributors should maintain these records as part of
their changes, as instructed by the root [AGENTS.md](../../AGENTS.md).
