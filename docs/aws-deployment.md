# AWS development deployment

This deploys the Rust cloud relay, not the loopback multi-account test harness.
Use the native clients for independently held keys. First deployment is a
working development milestone; it is not production or security acceptance.

## Operator login

A recent AWS CLI supports browser-based console login. AWS CLI 2.32 or newer is
required. On this Mac the current user-local installation is `~/.local/bin/aws`;
the older system installation remains untouched. A fresh profile avoids expired
static credentials overriding the browser session:

```sh
~/.local/bin/aws login --profile cirra-deploy --region us-east-2
~/.local/bin/aws sts get-caller-identity --profile cirra-deploy
```

IAM Identity Center users instead configure `aws configure sso` and use
`aws sso login` with their organization-provided start URL and region.
Never paste credentials into chat or commit AWS configuration/cache files.

For tools without support for the new console-login credential format, configure
a separate AWS profile with `credential_process` pointing to the current AWS CLI:

```ini
[profile cirra-terraform]
region = us-east-2
credential_process = /absolute/path/to/current/aws configure export-credentials --profile cirra-deploy --format process
```

The CLI supplies short-lived credentials directly to Terraform. It does not
copy them into the application, Lambda environment, Terraform variables, or repo.
See [AWS browser login](https://docs.aws.amazon.com/cli/latest/userguide/cli-configure-sign-in.html).

## Build, plan, and apply

Install Cargo Lambda and Zig using the [official instructions](https://www.cargo-lambda.info/guide/installation.html), then build:

```sh
bash scripts/build-lambda.sh
terraform -chdir=infra/terraform init -backend=false
```

The artifact is `artifacts/croaker-lambda.zip`, containing an executable ARM64
Linux `bootstrap` for `provided.al2023`. Artifacts, local variable files and
Terraform plans/state must remain ignored and private. Use `umask 077` before
creating a plan or state file. Preserve the state; do not create a second stack
against the same resources by losing or switching state files.

Prepare private Terraform variables containing `deployment_name`, `region`,
`lambda_zip` (absolute artifact path), and `budget_email`. Before applying, verify
the authenticated account, approved region, and plan. If the account or region
changes, regenerate the plan. An organization Service Control Policy can deny
operations even to a role named FullAccess or AdministratorAccess. Resolve that
with the account/organization administrator; do not weaken organization controls
or attempt a partial deployment around the denial.

After a successful apply, `terraform output -raw api_url` is the native client's
Business relay URL. Verify `/healthz`, `/v1/login/config`, rejected unauthenticated
messaging, and invited account/device enrollment. Create the initial account grant
and independently approved device through the operator workflow in
[login and enrollment](login-and-enrollment.md). Do not provision demo identities
or upload the local test workspace as production onboarding.

## Idle cost assumptions

For a nearly empty deployment with all clients closed, a planning estimate is
roughly $0–$1/month of incremental stack cost, depending on account free-tier usage
and retained data. It is not a quote, cap, or whole-account bill estimate.

Lambda/API Gateway are request/compute based; inactive Cognito users are not
monthly active users. Retained S3/DynamoDB data, continuous backups, logs and the
CloudWatch alarm can still cost money without user traffic. Reserved concurrency
is a limit, not paid provisioned warm capacity. No NAT gateway, EC2, RDS, custom
domain or provisioned Lambda concurrency is configured. Open clients poll for
messages, so an open but quiet app is not zero usage; unsolicited traffic counts
too. The budget alerts about spending and does not stop it.

Pricing checked 2026-11-08: [API Gateway](https://aws.amazon.com/api-gateway/pricing/),
[Lambda](https://aws.amazon.com/lambda/pricing/), [Cognito](https://aws.amazon.com/cognito/pricing/),
[DynamoDB](https://aws.amazon.com/dynamodb/pricing/), [S3](https://aws.amazon.com/s3/pricing/),
and [CloudWatch](https://aws.amazon.com/cloudwatch/pricing/).

## Current development rollout status

The development relay is deployed in **us-east-2**. The initial us-east-1 attempt
created supporting resources but core creation was denied by an organization
policy. After the user selected Ohio, a new plan reused the global IAM role and
budget and replaced the two verified-empty old-region log groups. The Ohio apply
completed, and live health/login-configuration checks pass. Unauthenticated
account access, conversation creation and sync return 401.

The initial Cognito administrator exists. Private first-login details are in
`data/aws-deployment/initial-administrator.local.json` (owner-only, ignored).
No invitation email was sent. Change the temporary password and enroll an
authenticator app at first sign-in. The administrator tenant grant was provisioned
with the IAM-authenticated operator CLI; account login does not approve a device. Verify the new
device's complete fingerprint and explicitly approve enrollment before messaging.
The initial administrator signed in from the iOS Simulator and its device was
explicitly approved after comparing the complete on-screen fingerprint. The
temporary operator approval credential was revoked and checked for rejection.
Physical-device onboarding, authenticated messaging and attachment transfer still
require live acceptance testing.

Preserve the private Terraform state and the old partial-state backup. If another
account is chosen, use a separate state/backend and retain this account's state.
A successful login or plan does not override organization restrictions.

## Browser publication

The same stack now includes a separate private frontend bucket, CloudFront HTTPS
hosting, uncached same-origin relay routing, and the exact browser Cognito
callback. `terraform output -raw web_url` is the webpage to open; `api_url`
remains the native relay address. See [browser setup and publishing](web.md).
The browser is an independent Rust/WASM device and needs its own approval, even
when its account is already signed in on a native device.
