# 0009: User-held encrypted history backup files and isolated restores

- **Status:** Accepted
- **Date:** 2026-11-08
- **Area:** Security, persistence, recovery, and interaction
- **Delivery:** Portable Rust backup/restore and local UI implemented; cloud automation and production enrollment pending
- **Source:** User requested implementing user-held recovery backups. File-first delivery and the format/limits below are engineering choices; production login and device approval must precede automatic cloud recovery.
- **Related:** Extends [ADR-0004](0004-transactional-client-state-and-recovery.md); preserves its separation between history recovery and live device state.

## Context

An encrypted live device vault is unsafe as a user recovery backup: restoring old
MLS ratchets or cloning an active device can compromise sending state. Users need
portable history recovery without giving the relay or business administrator a
decryption key. The current local harness has no production login, user identity
registry, OS secure-storage integration, or replacement-device enrollment.

## Decision

Implement manual `.crkbackup` file exports in the Rust client layer and expose
setup, confirmation, download, restore, search, attachment download, and removal
of local restored archives through the loopback UI. Do not add a public recovery
endpoint, automatic S3 storage, or a bypass for device approval.

### Secret and encryption format

Generate a 32-byte recovery secret with the OS cryptographic RNG. Encode it as
`CRK1`, eight groups of eight hexadecimal characters, and an eight-character
SHA-256 checksum. The checksum detects transcription errors; it is not a password
hash or an authentication factor. Do not derive this secret from a login password.
Require confirmation of the saved code before UI export. Keep codes only in memory;
never store them in the workspace manifest, browser storage, relay, URLs, or logs.
The local test bridge carries codes over loopback, like its other client operations.

Each file uses a fresh random 32-byte salt and a fresh random 32-byte archive data
key. Derive a wrapping key from the recovery secret with HKDF-SHA256 using that
salt and the domain `croaker-history-backup-wrapping-v1`. Reuse the existing
XChaCha20-Poly1305 authenticated record implementation, with independently random
24-byte nonces for the wrapped key and archive. Its existing context prefix adds
another domain boundary.

The binary file is the literal version marker `CROAKER-HISTORY-1` followed by a NUL,
the 32-byte salt, a 72-byte sealed data key (nonce + ciphertext + authentication
tag), and the sealed JSON archive (nonce + ciphertext + tag). The key wrapper
authenticates the marker and salt; the archive authenticates the complete header.
Version, salt, wrapped key, and ciphertext cannot be mixed between files without
failing authentication. Unknown versions, wrong keys, modifications, truncation,
and appended bytes are rejected. Only format, salt, encrypted key, and size are
visible without recovery material. No compression or bespoke streaming scheme is
introduced. Bounded whole-archive serialization caps plaintext at 16 MiB; larger
and streaming backups require a separately versioned format.

### History boundary and restore

Export a typed allowlist: archive ID/version/date, source tenant/device IDs,
participant display names/IDs, conversation names/membership, delivered message
text, attachments, and projected delivered reaction state. Exclude pending sends
and pending reaction changes, typing, credentials, signing keys, MLS secrets,
outbox envelopes, and sync cursors. This exports already saved history; it does
not force an offline account online or fetch history it has not received.

Authenticate the complete file and validate schema, references, duplicates,
content/attachment limits, and reaction support before writing. Import one encrypted
archive row atomically under the receiving client's vault key, using the archive
ID in authenticated context. Repeating an identical archive is idempotent;
conflicting content with the same ID is rejected. Live state, outbox, history,
and cursors are untouched. Separate accounts cannot read each other's imported
archive through the local bridge. A fresh workspace can restore an archive without
possessing the source device's keys.

Recovered history is read-only and separate from live chats. Display names and
historical sender labels are archive claims, not newly verified identities or MLS
membership. Possession of a code allows decryption (and fabrication) of its archive,
not account login, tenant access, or future-message access. Production account
recovery must authenticate and enroll a fresh identity, then obtain conversation
membership approval. Never copy identity keys or ratchets out of a recovery file.

## Rationale and alternatives

File exports make the recovery boundary usable and testable without inventing a
production user registry or conflating device credentials with account recovery.
They can be stored by the user wherever appropriate. Automatic cloud retrieval
needs a stable user-to-backup authorization model and verified replacement-device
enrollment first; a recovery code alone must not mint relay credentials.

Fresh per-file data keys isolate archive encryption from the recovery secret's
wrapping role. Whole-archive AEAD uses an existing primitive at the prototype's
bounded size and detects incomplete archives before any import. A raw SQLite copy,
live vault snapshot, password-derived recovery key, and administrator escrow were
not selected.

## Consequences

Losing both the code and any surviving decrypting device makes the corresponding
file unrecoverable. Anyone with both code and file can read the backed-up history;
backups intentionally retain that history beyond the live protocol's key lifecycle.
A new setup generates a new code; retain the correct code for each exported file.
No secret escrow, code reset, automatic key retention, or cloud schedule is added.

Backup dates do not prove freshness; a valid older file remains restorable. An
archive ID detects duplicate imports, not malicious rollback to an old backup.
Previously exported files cannot be revoked, and local archive deletion cannot
erase downloaded copies or guarantee physical erasure from SSDs/SQLite journals.
Deleted/disappearing message policy must be revisited before those features ship;
this implementation exports delivered history currently present on the client.
Recovered archives are not recursively included in live-history exports: preserve
their original encrypted files. No independent security audit is claimed.

## Implementation and evidence

- [Rust recovery format and archive persistence](../../crates/client/src/backup.rs)
  and [schema migration](../../crates/client/src/lib.rs).
- [Local export/restore and guarded routes](../../crates/local/src/backups.rs),
  [UI](../../apps/ui/src/Backups.tsx), and [local instructions](../../README.md).
- [Core tests](../../crates/client/tests/backups.rs): code checksum, randomized
  encryption, tamper/truncation/wrong-key/size rejection, strict schemas, encrypted
  persistence, idempotence/conflicts, restart, and separation from live state.
- [Harness tests](../../crates/local/tests/backups.rs): content/reactions/files,
  account isolation, cross-workspace restore, unchanged live identities/history,
  origin/confirmation/body limits, and no plaintext archive persistence.
- [Browser tests](../../apps/ui/tests/backups.spec.ts): confirmed download, restore,
  search, account isolation, refresh persistence, mobile errors, duplicate import,
  and local archive removal. Production S3 integration is unimplemented. [ADR-0010](0010-account-login-and-replacement-device-enrollment.md)
adds the login/enrollment backend and local approval flow; isolated production
clients and deployment acceptance remain pending.
