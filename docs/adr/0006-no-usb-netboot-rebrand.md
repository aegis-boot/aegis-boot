# ADR 0006: aegis-boot stays one product (no "Aegis Boot USB" + "Aegis Netboot" rebrand)

**Status:** Accepted
**Date:** 2026-04-25
**Deciders:** 6-agent `consensus_vote` on [epic #556](https://github.com/aegis-boot/aegis-boot/issues/556). The Developer Experience role rejected the rebrand most strongly; the rest of the panel concurred in their reasoning.
**Tracking issue:** [#568](https://github.com/aegis-boot/aegis-boot/issues/568)

## Context

The gpt5.5 plan proposed a rebrand: "Aegis Boot" as an umbrella platform, with two delivery sub-products:

- **Aegis Boot USB** — portable trusted USB boot workflows
- **Aegis Netboot** — trusted PXE / iPXE / UEFI HTTP boot workflows

Implementation: split into ~10 crates including `aegis-boot-usb`, `aegis-netboot`, `aegis-netbootd`. CLI surface to remain unified via `aegis-boot {usb, netboot, verify, doctor, pack}`.

## Decision

**Keep aegis-boot as one product, one CLI binary, one umbrella name.** No "Aegis Boot USB" / "Aegis Netboot" rebrand.

The umbrella concept itself is correct and worth landing in [`docs/ARCHITECTURE.md`](../ARCHITECTURE.md) ([#569](https://github.com/aegis-boot/aegis-boot/issues/569)) — but as one product with one current delivery (USB) and one deferred delivery (netboot, per ADR 0003), not as two parallel sub-products.

## Re-entry criteria

Reconsider the rebrand when:

1. ADR 0003 re-enters AND netboot ships to ≥1 real operator. At that point a sub-brand may help disambiguate the two delivery paths.
2. OR a future product expansion (e.g., a SaaS attestation receiver, a hardened-kernel build service) needs naming separation.

Until then, the rebrand is documentation churn with no operator value.

## Rationale (from the consensus_vote)

- **DevEx (most strongly):** *"From a developer-experience perspective, the rebrand and crate split add substantial API and tooling surface without improving the primary user workflow enough to justify the learning and maintenance cost. The audit shows most of the plan overlaps existing crates and concepts, so the proposal mostly redistributes logic developers already need to understand instead of reducing complexity."*
- **PM:** *"Operator value: rebrand to 'Aegis Boot USB + Aegis Netboot' helps no current user (ROADMAP confirms no netboot demand) and creates documentation/marketing churn."*
- **Architect:** *"10-crate refactor compounded by a 10-crate release-coordination tax. Reversibility favors B decisively — additive changes can always be extended; premature crate splits are expensive to undo."*

A sub-brand implies a maturity level the netboot path doesn't have (it's deferred entirely per ADR 0003). Adopting the sub-brand before the underlying product exists is marketing in advance of substance.

## What stays in scope

- One product: `aegis-boot`.
- One CLI binary: `aegis-boot`.
- One umbrella concept (per [#569](https://github.com/aegis-boot/aegis-boot/issues/569)): trusted boot, recovery, and provisioning tooling, with USB as the current delivery.
- This ADR.

## What stays out

- "Aegis Boot USB" / "Aegis Netboot" as parallel sub-product names.
- `aegis-boot-usb` and `aegis-netboot` as separate crates (audit found `aegis-boot-usb` would be an extraction of existing flash + detect logic with no obvious win; `aegis-netboot` is greenfield and ADR 0003 defers it).
- `aegis-netbootd` as a separate binary (ADR 0003).
- A "platform" framing in marketing copy (we're a tool; let it grow into a platform if real demand arrives).

## References

- [ADR 0003](./0003-defer-netboot-daemon.md) — defers the netboot daemon, which is the prerequisite for ever needing a sub-brand.
- [#569](https://github.com/aegis-boot/aegis-boot/issues/569) — the umbrella-concept architecture-doc section that lands the framing without the rebrand.
