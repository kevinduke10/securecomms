# 0001: Build secure messaging for businesses

- **Status:** Accepted
- **Date:** 2026-11-07
- **Area:** Product and commercial scope
- **Delivery:** Product direction; partially implemented
- **Source:** Explicit user choices and the accepted implementation plan.

## Context

The user wants a secure messaging product with Signal-like features, a Rust core,
AWS serverless deployment, groups, media sharing, and private video calls later.
It must be commercially usable by businesses.

## Decision

- Support hosted multi-tenant SaaS and dedicated customer-owned AWS installations
  from the commercial launch. Dedicated messaging must work independently of our
  SaaS control plane; licensing and updates can initially be managed manually.
- Provide participant-only encryption. Business administrators manage access and
  devices without receiving message decryption access.
- Target small/mid-size businesses first, with eventual iOS, Android, Windows, and
  macOS clients sharing React/Tauri UI and a Rust core.
- Support restricted external guests within an inviting tenant. Communication
  between independent installations is outside the initial scope.
- Use email/invitations, MFA, and enterprise SSO rather than phone-number identity.
  Start with sales-led onboarding and manual invoices, not self-service billing.
- Aim for a full messaging pilot: direct/groups, multiple devices, offline
  delivery, media, reactions, replies, receipts, local search, disappearing
  messages, and user-key recovery backups. Private voice/video calls come later;
  large broadcasts are not the selected video direction.

## Rationale and alternatives

The user explicitly selected both deployment models, participant-only privacy,
mobile/desktop clients, external guests, enterprise SSO, and sales-led pilots.
This establishes the intended product even while implementation proceeds in
smaller validated milestones.

## Consequences

The same core and infrastructure should serve both deployment modes. Central
plaintext search, administrator recovery, and compliance exports do not follow
from participant-only encryption and must not be silently introduced.

Capacity goals are 1,000 seats per business and 500 users per group. The current
relay limit is 500 **devices**, and those product capacities are not benchmarked.
The user accepted a $200/month pilot AWS planning budget; the estimate of roughly
300 active users is an assumption to measure, not a capacity or billing guarantee.
AWS budget alerts do not cap spending. No regulated-sector certification is claimed.

## Implementation and evidence

The [project README](../../README.md) lists implemented behavior and remaining
milestones. The local app already demonstrates groups, guests, attachments,
reactions, typing, offline queueing, and encrypted history. Production enrollment,
SSO, mobile/native distribution, automatic cloud recovery, advanced messaging, and calling
remain future work. Portable history backup files are implemented in
[ADR-0009](0009-user-held-history-backup-files.md). See [ADR-0002](0002-rust-core-and-serverless-aws.md) for deployment
and [ADR-0003](0003-participant-only-encryption-and-trust.md) for security boundaries.
