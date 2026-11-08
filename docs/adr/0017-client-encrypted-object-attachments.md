# 0017: Client-encrypted object attachments

- **Status:** Accepted
- **Date:** 2026-11-08
- **Area:** Security / protocols / persistence / deployment
- **Delivery:** Implemented foundation; AWS deployment and production security review remain outstanding
- **Source:** User accepted the next implementation milestone. Larger attachments, ciphertext storage and native device isolation follow accepted product direction. The format, limits, quotas and retention below are engineering choices.

## Context

The original application embeds files up to 32 KiB inside MLS messages. Larger media would exceed the relay's bounded message and serverless request sizes. A cloud object store must not learn the original filename, MIME type, encryption key or plaintext. A retry after an uncertain upload or send result must not produce a second MLS ratchet advance or silently change an attachment's ciphertext.

## Decision

Keep existing inline attachments compatible. Add a versioned media application event, carrying text, an optional reply reference and a secret object descriptor **inside MLS encryption**. The descriptor binds the conversation, random object ID, filename, MIME type, plaintext size, ciphertext size, random key and ciphertext SHA-256. UI snapshots contain only an explicit name/MIME/size/object-ID summary; native webviews never receive raw decryption keys.

The Rust client encrypts a whole file with the existing RustCrypto XChaCha20-Poly1305 implementation, a random 256-bit key and random 192-bit nonce. The file format is a fixed version header, nonce and authenticated ciphertext. Versioned additional authenticated data binds the conversation ID, object ID, exact plaintext length and length-prefixed filename/MIME. The initial plaintext maximum is **25 MiB**. This separate bound does not raise the existing 16 MiB device-vault or recovery-file bounds. Before exposing plaintext, the client checks descriptor validity, exact ciphertext length, SHA-256, authenticated decryption and exact plaintext length. This is not a new cryptographic primitive or streaming video format.

`ClientStore::prepare_media` durably saves the exact ciphertext and a vault-key-encrypted descriptor/source fingerprint under a stable request ID before any upload. Repeated requests must match the original source and metadata. The descriptor is then sent using the existing atomic MLS state/history/outbox transaction and stable message ID. Uploading alone does not advance MLS. A confirmed upload followed by an uncertain send retries the saved envelope. After a confirmed immutable upload, the client compacts the cached ciphertext while retaining the sealed descriptor/source fingerprint for idempotency. Retries of a compacted object reuse that descriptor and verify the object still exists. If remote storage requires a fresh PUT after compaction, the client reports that the upload expired and requires a new attachment request; it never re-encrypts under the old request. Pending ciphertext has a separate 512 MiB capacity guard, and retained request metadata has a 1,024-object guard with an explicit capacity error. Automatic metadata eviction is future work.

The relay only accepts an object ID, ciphertext size and digest. It authorizes the current device, tenant and conversation before issuing upload/download grants. Reservations are durable and immutable, scoped to tenant/conversation/object, and reject conflicting retries. Atomic reservation quotas charge retries once: 250 MiB per device per UTC day and 1 GiB per tenant per UTC day. Reservation expiration is 30 days. Quotas are an initial spend guard, not billing or comprehensive abuse prevention.

In AWS, ciphertext uploads/downloads go directly to private S3 through **60-second** signed URLs. PUT grants bind content length, SHA-256 checksum, octet-stream type and create-only `If-None-Match: *`; an existing matching object makes a repeated upload request succeed without another PUT. Native clients must use a transport without redirects, never attach a relay bearer to an S3 URL, validate transfer destinations and bound downloads. Current device membership is checked when granting access; an already issued S3 URL remains usable until its short expiration. Revocation does not erase plaintext or keys already held by a former member.

Terraform blocks public access/ACLs, requires TLS, enables S3 encryption at rest, limits signature age, configures 30-day object expiration, and grants the relay object GetObject/PutObject plus bucket ListBucket (needed for HeadObject to return 404 for a missing upload rather than 403) on this bucket. Raw keys and names do not enter S3 keys, relay metadata or signed URL responses. Object deletion by lifecycle is asynchronous; the registry denies expired references independently.

The loopback test workspace has a separate SQLite reservation registry and private ciphertext files. It enforces existing account/device/conversation authorization and decrypts in Rust for the authenticated local browser. As with the rest of `croaker-local`, this is explicitly a development exception: the loopback process hosts all demo clients. Local files are not automatically garbage-collected yet; expired reservations deny access, and operators must account for disk retention in test workspaces. Never deploy this workspace as the relay.

Only PNG/JPEG/WebP/GIF are eligible for explicit raster previews. Other MIME types, including HTML and SVG, download as octet-stream. Preview/download uses fully verified Rust plaintext, never an unverified remote file URL. User-visible filenames are not trusted as paths or HTML.

## Rationale and alternatives

Separate object storage preserves the serverless request limits and avoids base64 overhead inside every MLS message. Whole-file AEAD reuses an established library and is practical at the explicit 25 MiB bound. Chunked streaming encryption and video need a separately reviewed versioned format, congestion/backpressure controls and transport design. The encryption library's security properties do not constitute an audit of this application.

Atomic reservations and create-only object writes prevent mutable object IDs and uncharged replay uploads. A separate registry avoids changing the message relay's storage contract, while SQLite and DynamoDB implementations share the same public interface. S3 checksum headers provide transfer integrity; the client still performs independent hash and AEAD checks before rendering.

## Consequences

Attachments expire separately from encrypted message history: a historical message may remain while its remote media is unavailable. Object sizes and traffic timing remain visible to the relay and storage provider; no padding or metadata resistance is claimed. Uploads that never result in a message occupy quota/storage until expiration.

The current small recovery archive cannot embed a 25 MiB file. Recovery exports preserve an explicit omitted-media name/type/size record, without remote IDs or keys, rather than implying recovery of the binary. Existing live local history continues to retain the original encrypted descriptor. A future encrypted backup bundle must specify offline object inclusion, format versioning and capacity before claiming complete media recovery.

Full device replacement with attachments, streaming video, automatic metadata/disk cleanup, resumable multipart uploads, malware/content policy, billing and production operational limits remain follow-up work. No real AWS deployment or security audit was performed in this change.

## Implementation and evidence

- [Wire contracts](../../crates/protocol/src/media.rs), [authenticated file format](../../crates/crypto/src/media.rs), [durable client cache](../../crates/client/src/media.rs), [MLS application projection](../../crates/client/src/application.rs).
- [Authorized object service and local store](../../crates/relay/src/media.rs), [DynamoDB reservations](../../crates/relay/src/media/dynamodb.rs), [S3 grants](../../crates/relay/src/media/s3.rs), [Lambda wiring](../../crates/relay/src/bin/lambda.rs), [private S3 infrastructure](../../infra/terraform/media.tf).
- [Loopback bridge](../../crates/local/src/media.rs), [attachment UI](../../apps/ui/src/ObjectAttachment.tsx), [UI transport helper](../../apps/ui/src/media.ts).
- [Durable retry tests](../../crates/client/tests/media.rs), [immutable/scoped/quota tests](../../crates/relay/tests/media.rs), [local large-file round trip](../../crates/local/tests/media.rs), and crypto module tamper/scope/size tests exercise the implementation. The ignored `dynamodb_media_contract` test exercises immutable metadata, both byte quotas, transaction rollback and error classification against DynamoDB Local. AWS SDK compilation and mocked Terraform plans validate configuration shape, not live S3/IAM behavior.
- AWS references: [presigned URL integrity and expiration](https://docs.aws.amazon.com/AmazonS3/latest/userguide/using-presigned-url.html), [S3 checksum behavior](https://docs.aws.amazon.com/AmazonS3/latest/userguide/checking-object-integrity.html), [HeadObject permission behavior](https://docs.aws.amazon.com/AmazonS3/latest/API/API_HeadObject.html).
