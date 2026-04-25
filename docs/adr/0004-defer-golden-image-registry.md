# ADR 0004: Defer the golden-image registry / drift-detection subsystem

**Status:** Accepted
**Date:** 2026-04-25
**Deciders:** Audit-driven decision on [epic #556](https://github.com/aegis-boot/aegis-boot/issues/556) — flagged as greenfield + speculative by all 6 agents in the higher-order consensus vote.
**Tracking issue:** [#566](https://github.com/aegis-boot/aegis-boot/issues/566)

## Context

The gpt5.5 plan proposed a "golden-image validation" subsystem comparing systems against expected baselines: OS / version, Secure Boot state, disk-encryption expectations, partition layout, installed bootloader, recovery partition, firmware expectations, baseline provisioning. Outputs would include pass/fail reports, drift reports, JSON for automation, Markdown for humans.

Audit of the codebase found:

- **One narrow precedent**: [`update.rs`](../../crates/aegis-cli/src/update.rs) computes a per-file ESP sha256 diff for in-place update eligibility. That's point-in-time comparison against a freshly-built reference, not a stored baseline.
- **Zero registry**, zero drift-detection state, zero "expected baseline" type.
- **No active operator workflow** has surfaced that needs full baseline tracking. ROADMAP.md doesn't mention it.

## Decision

**Defer the golden-image registry.** No baseline-store crate, no drift-report generator, no compliance-audit subcommand in this milestone.

The current `update --eligibility` path is sufficient for the v1.0 use case (in-place signed-chain rotation). The raw material for a future drift-detection layer already exists in `Manifest.esp_files[]` and `Attestation.target.image_sha256` — no schema change is needed to start that work later.

## Re-entry criteria

Reconsider when:

1. An operator files a concrete workflow describing the comparison they need: *which fields*, *against what reference*, *on what cadence*.
2. AND that workflow can't be served by `update --eligibility` plus `verify --stick`.

A future epic that re-opens this records the specific operator scenarios it serves.

## Rationale

- **YAGNI**: speculative scaffolding before a concrete operator requirement.
- **Existing primitives suffice**: the data we'd need to compute drift (ESP file hashes, attestation manifest, ISO sidecars) is already captured. A future drift layer is a *consumer* of existing data, not a producer of new data.
- **Scope discipline**: the gpt5.5 plan bundled this with 5 other large feature areas in one push; treating it as a separate epic preserves the option to design it well when the requirement is clear.

## What stays in scope

- `update --eligibility` (existing) — point-in-time ESP comparison for the in-place update path.
- `verify --stick` (existing) — re-hash all ISOs against their sidecars.
- `Attestation` JSON written per flash (existing) — captures the source-side baseline at flash time.
- This ADR.

## What stays out

- A `aegis-baseline` crate or registry.
- A `aegis-boot drift` subcommand.
- A "golden image" JSON Schema / wire format.
- Cron-style continuous-validation tooling.

## References

- [`crates/aegis-cli/src/update.rs`](../../crates/aegis-cli/src/update.rs) — the existing point-in-time ESP diff.
- Codebase audit: parallel-Explore subagents on [epic #556](https://github.com/aegis-boot/aegis-boot/issues/556).
- ADR 0002 (KEY_MANAGEMENT) for the trust-anchor model that any future drift layer would have to respect.
