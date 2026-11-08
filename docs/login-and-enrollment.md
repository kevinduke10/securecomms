# Login and replacement-device setup

The default local app now opens a sign-in screen. Choose a development account to
try it without AWS. The demo verifies no password and still hosts all private
client keys on one Mac. Do not use it for confidential data or deploy this harness.

## Try replacement enrollment locally

1. Sign in as Alice, Bob, or Maya. Open **Devices** in the top strip.
2. Name and request a replacement. A fresh identity/store is saved before the
   signed enrollment request is submitted. Repeating a failed request reuses it.
3. On the existing device, compare the full fingerprint using a trusted channel,
   enter that fingerprint, and approve. In this local demo both devices are on
   this Mac, so you can complete the exercise in the same dialog.
4. Choose **Use this new device**. It has no old conversation memberships or keys.
   The original conversation owner can open **Conversation details → Manage group
   members**, select the approved device, compare its fingerprint, and invite it.
   It receives new messages, never automatic copies of prior history. Use
   **Backups** for isolated read-only past history.
5. Open **Devices** on the replacement to revoke the old device's relay access.
   Remaining conversation members must still perform MLS removal/key rotation;
   revocation does not erase already received messages.

If the lost device was a conversation’s sole owner, owner transfer/recovery is
not implemented; account administration alone cannot supply that group’s keys.

If all devices are lost, a separately verified same-tenant administrator can
review an enrollment by its account enrollment ID. Administrators verify the new
fingerprint; they receive no message decryption keys. A recovery code restores
history only and cannot approve enrollment.

## Cognito deployment configuration

Terraform adds an invite-only email user pool, required authenticator-app MFA,
a public authorization-code client, and five-minute access tokens. It exposes
`cognito_user_pool_id`, `cognito_issuer`, `cognito_client_id`, and `cognito_domain`.
No AWS resources are created until an operator deploys the Terraform. Configure
production HTTPS callbacks explicitly with `login_callback_urls` and
`login_logout_urls`; the defaults are only for local client testing.

Invite users through Cognito's administrative tools. Use their immutable Cognito
`sub`, not email, when granting a tenant and role. Prepare an ignored
`account-grant.local.json` containing these fields (replace every placeholder):

```json
{
  "issuer": "https://cognito-idp.REGION.amazonaws.com/USER_POOL_ID",
  "subject": "COGNITO_SUBJECT",
  "tenant_id": "TENANT_UUID",
  "role": "member",
  "name": "Display name",
  "bootstrap_device": null
}
```

Register with IAM/operator authority, outside the public API:

```sh
cargo run -p croaker-cli --features aws -- grant-account-aws \
  --table YOUR_TABLE_NAME --grant account-grant.local.json
```

For SQLite use `grant-account --database PATH --grant FILE`. Optional
`bootstrap_device` is a public identity binding (`tenant_id`, `device_id`, and a
32-byte `signature_key` JSON array) verified by the operator, and must refer to an
already provisioned device with that tenant/role. Existing operator-only device
provisioning can bootstrap the first administrator. Public login cannot mint
credentials or automatically approve that first device.

The deployed relay reads the Cognito configuration Terraform injects. Account
endpoints accept a Cognito access token with `croaker/access`; message and approval
endpoints accept an approved device credential. There is no public account grant
or tenant assignment endpoint. See [HTTP contract](api.openapi.yaml).

## Connect the local client to Cognito sign-in

This tests hosted login while retaining the local harness's three test owners and
one-machine key-storage boundary. It does not turn the harness into a production
client or make its SQLite relay the deployed AWS relay.

Create an ignored mapping file containing the immutable subjects assigned to the
three local owners:

```json
{"alice":"ALICE_COGNITO_SUB","bob":"BOB_COGNITO_SUB","maya":"MAYA_COGNITO_SUB"}
```

Set the following environment before starting:

```sh
export CROAKER_LOGIN_MODE=cognito
export CROAKER_COGNITO_ISSUER=https://cognito-idp.REGION.amazonaws.com/USER_POOL_ID
export CROAKER_COGNITO_CLIENT_ID=APP_CLIENT_ID
export CROAKER_COGNITO_DOMAIN=https://DOMAIN.auth.REGION.amazoncognito.com
export CROAKER_COGNITO_SUBJECTS_FILE=/absolute/path/subjects.local.json
export CROAKER_LOGIN_BASE_URL=http://localhost:3000
./scripts/start-local.sh
```

Open **http://localhost:3000**. Configure that exact
`http://localhost:3000/local-auth/callback` in Cognito. The local Rust client uses
PKCE and browser-bound state; it does not persist access or refresh tokens.
Sessions end with the access token, and restarting clears local browser sessions
without deleting encrypted history. Production clients still need their own
secure key storage and endpoint-specific enrollment integration.

For the original unrestricted development playground only, set
`CROAKER_LOGIN_MODE=demo` and `CROAKER_DEMO_AUTOLOGIN=1`. This bypass is visibly
reported by the auth configuration, is used by the existing messaging test suite,
and is rejected in Cognito mode. It is not an authentication option for production.

## Validation

Run `npm run test:ui` for messaging/backups/themes in the explicit multi-account
test mode and `npm run test:auth` for sign-in/enrollment with real session gates.
Use the Rust and Terraform checks in the [README](../README.md). The DynamoDB Local
script now runs event-store, enrollment, and group-admission contracts against an emulator.
Live Cognito, real email delivery, real-device isolation, and AWS rollout need
separate configured-environment acceptance testing. This is not a security audit.
