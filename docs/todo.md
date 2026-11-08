# Cirra todo list

Planned work, not implemented behavior. See the [product milestones](../README.md#next-product-milestones)
and [ADRs](adr/README.md) for the broader direction and existing decisions.

## User invitations and device onboarding

- [ ] Add administrator invitations for new users.
- [ ] Separate first-device onboarding from additional/replacement-device approval.
- [ ] Let an invited user sign in with MFA and enroll their first device under
  an explicit organization policy, without manually comparing a long fingerprint
  in the normal welcome flow.
- [ ] Retain administrator approval of every first device as an optional stricter
  organization policy.
- [ ] Require approval for additional/replacement devices, with a verified QR-code
  flow from an existing approved device instead of manual fingerprint entry where
  possible.
- [ ] Define and test invitation expiry/replay handling, first-device enrollment
  races, QR-code identity/session binding, and recovery when no approved device
  remains. Record the policy and protocol design in an ADR before implementation.

Keep keys on clients and preserve signed proof of possession. Account login or
first-device enrollment must not automatically grant existing conversation keys,
membership or past history. The current explicit approval behavior remains in
place until the new policy is designed, implemented and tested. See
[ADR-0010](adr/0010-account-login-and-replacement-device-enrollment.md).

Source: user requested adding the proposed onboarding improvements to the todo
list on 2026-11-08; implementation was not requested in this step.
