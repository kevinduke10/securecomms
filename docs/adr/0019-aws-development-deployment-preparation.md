# 0019: Prepare an authenticated, low-idle-cost AWS development deployment

- **Status:** Accepted
- **Date:** 2026-11-08
- **Area:** Deployment / operator authentication / operations
- **Delivery:** Development relay deployed in us-east-2; live endpoint smoke checks pass; native login/enrollment acceptance pending
- **Source:** User requested AWS deployment and asked about costs without use. The user supplied an administrator/budget contact; that private deployment input remains outside tracked documentation.
- **Related:** [0002](0002-rust-core-and-serverless-aws.md), [0010](0010-account-login-and-replacement-device-enrollment.md), [0017](0017-client-encrypted-object-attachments.md).

## Context

The native clients require a deployed HTTPS relay and Cognito configuration.
Infrastructure and mock/emulator checks existed, but no cloud service had been
created. Saved operator credentials were invalid; a current browser-based AWS
login was needed without copying access keys into the app or chat.

## Decision

Retain the existing on-demand Lambda/API Gateway/DynamoDB/S3 design, bounded
logs/media retention, MFA, private media, deletion protection and continuous
backups. Do not remove data protection to shave small idle charges. Build only
the relay/protocol dependency graph for ARM64 Linux; never deploy `croaker-local`.

Use a new operator browser-login profile, preserving existing profiles. Supply
short-lived credentials to Terraform through the AWS CLI credential-process
bridge when needed. Keep generated deployment inputs, Terraform plans/state,
account details and artifacts ignored and private. Verify the target account and
region before applying, and regenerate saved plans after either changes.

Do not change or bypass organization Service Control Policies as part of app
deployment. Explicit authorization failures require an approved account/region
or action by the organization administrator before cloud creation continues.

## Rationale and tradeoffs

Browser login avoids introducing long-lived operator keys. A real Terraform plan
makes the intended cloud changes reviewable, but a successful plan does not prove
permission to create every resource. Local private state is adequate for this
single-operator development preparation; shared production state management is
separate work and must preserve locking/access protection.

Serverless removes always-on compute, not every charge. The nearly empty idle
estimate of roughly $0–$1/month assumes closed clients, little retained data and
account-dependent free allowances. Open clients poll, and storage/backups/logs/
monitoring can accrue costs without messages. This estimate is an engineering
planning assumption, not measured deployment billing. The existing budget alerts
are not a spending cap.

## Implementation and evidence

- [Deployment runbook and pricing assumptions](../aws-deployment.md).
- [Lambda packaging](../../scripts/build-lambda.sh) produced a ZIP with an
  executable, stripped ARM64 Linux bootstrap; no client/MLS decryption crate is
  linked into the relay.
- [Terraform](../../infra/terraform/main.tf) validates; all three mocked plans pass.
  A real read-only plan was also generated using authenticated operator identity.
- Initial live inventory/quota checks were rejected by an AWS organization policy.
  After the user reiterated the deployment instruction, a fresh plan and actual
  apply were attempted. Terraform recorded successful creation of the relay IAM
  role, two empty log groups, and the account-wide budget/alerts.
- Cognito CreateUserPool, DynamoDB CreateTable, S3 CreateBucket, and API Gateway
  CreateApi were explicitly denied by the organization's Service Control Policy.
  The current role also cannot read the denying policy, so an approved region
  cannot be inferred. No controls were changed or bypassed.
- The user subsequently selected `us-east-2`. Lambda quota and DynamoDB/Cognito
  inventory checks succeeded there. A fresh plan reused the global IAM role and
  budget, replaced the two verified-empty old-region log groups, and created the
  remaining regional resources. Explicit log-group regions prevent the provider
  retaining old-region state when changing the installation region.
- The Ohio apply completed. Live HTTPS checks passed for health and public login
  configuration; account access, conversation creation and sync without credentials
  returned 401. These checks do not establish authenticated messaging, MFA, native
  enrollment, S3 attachment acceptance, or production readiness.
- The initial Cognito administrator was created without sending an invitation.
  Its immutable subject was granted administrator access in a new tenant through
  the IAM-authenticated operator CLI. Its temporary password remains in an ignored
  owner-only local file. First login
  must change that password and enroll authenticator-app MFA; device fingerprint
  verification and explicit approval remain necessary.
- The old partial state backup and current deployment state are preserved privately.
  If deployment moves to another account, retain this state and use a separate
  state/backend there; never apply this account's state under another identity.

- Live first-administrator onboarding subsequently completed account sign-in and
  submitted a device enrollment from the iOS Simulator. The operator compared
  the complete fingerprint rendered in the simulator with the pending request,
  provisioned a temporary tenant-admin approval credential through the existing
  IAM-only CLI, and approved through the normal transactional enrollment API.
  The approved public binding was read back, the temporary credential was revoked,
  a subsequent authenticated request returned 401, and its local secret was
  removed. No client private keys or MLS state were transferred to the relay.
  This validates this simulator enrollment, not physical-device or messaging/media
  acceptance.
