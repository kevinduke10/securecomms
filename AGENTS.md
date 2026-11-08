# Working on Cirra

## Read before changing the project

- Read [README.md](README.md) for the current implementation and local workflow.
- Read the [ADR index](docs/adr/README.md), [product direction](docs/adr/0001-product-and-commercial-scope.md), and records relevant to the task before choosing an approach.
- Consult [architecture.md](docs/architecture.md) for implementation details and security boundaries. ADRs explain why; source code and tests establish what actually works.
- Distinguish accepted product direction from implemented features. This repository currently contains a local test application and a serverless relay foundation, not a production-ready service.

## Maintain Architecture Decision Records

- Capture meaningful changes to architecture, security, protocols, persistence, deployment, or established interaction patterns in `docs/adr/` as part of the same change.
- Use the [template](docs/adr/template.md), the next four-digit number, and a descriptive filename. Update the ADR index and link relevant code/tests.
- Record context, the decision, rationale, tradeoffs, consequences, and implementation status. Identify user requirements separately from engineering choices and estimates.
- Keep accepted decisions as historical records. If an approach changes, add a new ADR, mark the old one superseded, and link both directions. Correct factual errors or update delivery evidence in place without erasing the original rationale.
- Routine fixes that preserve an existing decision do not require a new ADR. Update documentation when behavior or implementation status changes.
- Follow the user's current instructions when they change an earlier decision, and reconcile the ADRs in that change. ADRs do not introduce an extra approval step.
- Never put credentials, private keys, recovery secrets, local workspace manifests, or private message contents into ADRs or other tracked documentation.

## Important boundaries

- Keep message decryption and client keys outside the cloud relay. Use the existing MLS implementation; preserve pinned identity verification and tenant authorization.
- Persist cryptographic state atomically with outgoing ciphertext and incoming history/cursors. Retry saved envelopes; do not re-encrypt a send after an ambiguous response.
- `croaker-local` owns all demo accounts and their development keys on one machine. It must stay loopback-only and must not be deployed as the AWS service.
- Preserve existing workspace data during development. Use a separate `--data-dir` for fresh fixtures rather than resetting the user's workspace.
- Use direct, functional UI copy without taglines or promotional slogans; consult ADR-0012.
- Preserve the bubble long-press reaction pattern and searchable bundled emoji catalog; consult ADR-0006 before changing that interaction.

## Validation

Use the relevant checks documented in README.md. Run browser tests for UI behavior changes and Rust tests for core/backend changes. Documentation-only changes need link and consistency checks, not a full application rebuild. Report actual validation and material limitations; do not imply benchmarks, security audits, production deployment, or compliance work have been completed without evidence.
