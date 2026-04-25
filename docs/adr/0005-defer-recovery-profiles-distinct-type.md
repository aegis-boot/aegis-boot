# ADR 0005: Recovery profiles are not yet a distinct typed subsystem

**Status:** Accepted
**Date:** 2026-04-25
**Deciders:** Audit-driven decision on [epic #556](https://github.com/aegis-boot/aegis-boot/issues/556) — the "first-class profile type" framing is rejected; a future "fourth profile slug" within the existing `Profile` array is the cheap re-entry path.
**Tracking issue:** [#567](https://github.com/aegis-boot/aegis-boot/issues/567)

## Context

The gpt5.5 plan proposed recovery profiles as a first-class typed subsystem with submodes: disk triage, file recovery, malware triage, memory capture, evidence collection, network troubleshooting, secure wipe. Outputs would include `recovery-report.json`, `evidence-manifest.json`, `hashes.sha256`, `operator-notes.md`.

Audit of the codebase found:

- **Existing profile scaffolding**: [`crates/aegis-cli/src/init.rs`](../../crates/aegis-cli/src/init.rs) defines a `Profile` struct (name + description + slugs array) plus three profiles: `panic-room`, `minimal`, `server`. Recovery is conceptually a fourth slug, not a new type.
- **`bug-report` exists** but it's an operator-bug-draft generator with PII redaction, not an incident-response evidence-capture pipeline.
- **No specific operator workflow** has surfaced that requires the listed evidence-manifest fields. The list reads as a "what could a recovery toolkit emit?" enumeration, not a "what does this specific operator need to recover from?" answer.

## Decision

**Defer recovery profiles as a distinct typed subsystem.** No new `RecoveryProfile` type, no `aegis-boot recover` subcommand, no `evidence-manifest.json` schema in this milestone.

An operator who needs a recovery USB today can already use `aegis-boot init --profile minimal` (Alpine 3.20 with their tools of choice added) plus `aegis-boot bug-report` for the operator-evidence side.

## Re-entry criteria

Reconsider when ALL of:

1. An operator describes a specific incident-response scenario in a GitHub issue (not "could be useful" — a real incident or near-miss).
2. AND that scenario calls for evidence-manifest fields the existing `Attestation` + `bug-report` outputs don't already cover.
3. AND a maintainer commits to the schema review + ongoing evolution (incident-response output formats are notoriously hard to evolve; the schema lock has long-tail consequences).

The cheap path forward, when re-entry happens: add `recovery` as a fourth slug in the existing `Profile` array. Promote to its own type only if the data model genuinely needs it — `Profile { slugs: [...] }` may be enough.

## Rationale

- **YAGNI**: "what would a recovery toolkit emit?" is a poor design driver. Real incident-response workflows are specific.
- **Existing primitive is sufficient**: a fourth `Profile` covers 80% of the operator value (a curated stick of recovery ISOs).
- **Schema-lock cost**: an incident-response wire format is hard to revise once operators start scripting against it. Premature commitment is expensive.

## What stays in scope

- The existing `Profile` struct + 3 profiles.
- `bug-report` for operator-side evidence.
- `Attestation` for flash-time provenance.
- This ADR.

## What stays out

- `RecoveryProfile` as a distinct type.
- `evidence-manifest.json` wire format.
- `aegis-boot recover` subcommand.
- Disk-triage / memory-capture / malware-triage submodes as built-in workflows.

## References

- [`crates/aegis-cli/src/init.rs`](../../crates/aegis-cli/src/init.rs) — existing `Profile` struct + array.
- [`crates/aegis-cli/src/bug_report.rs`](../../crates/aegis-cli/src/bug_report.rs) — existing operator-evidence path.
- Codebase audit: parallel-Explore subagents on [epic #556](https://github.com/aegis-boot/aegis-boot/issues/556).
