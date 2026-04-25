# ADR 0003: Defer the full netboot daemon — `aegis-netbootd` is post-v1.0

**Status:** Accepted
**Date:** 2026-04-25
**Deciders:** 6-agent `consensus_vote` (`higher_order`) on [epic #556](https://github.com/aegis-boot/aegis-boot/issues/556) — Architect, Security, AI/ML, PM, DevEx, Contrarian all recommended deferring in their reasoning.
**Tracking issue:** [#565](https://github.com/aegis-boot/aegis-boot/issues/565)

## Context

A substantive plan from gpt5.5 proposed extending aegis-boot with a full netboot delivery path: an `aegis-netbootd` daemon serving generated iPXE menus, HTTP-served kernel/initrd/cmdline profiles, optional iPXE chainloading, a local mirrored asset cache, optional TFTP / proxy-DHCP adapters, and eventual UEFI HTTP boot endpoints. The plan framed this as the second of two delivery paths under an "Aegis Boot" umbrella.

Audit of the codebase found:

- **Zero existing scaffolding.** No iPXE / PXE / TFTP / DHCP / HTTP-boot mentions anywhere in `crates/`, no TODO comments, no design notes.
- **Explicit roadmap deferral.** [`ROADMAP.md`](../../ROADMAP.md) line 22: *"Network boot / PXE — explicitly out of scope for v1.0; reconsider if real users ask."*
- **No demand signal.** No GitHub issue from any operator describing a netboot need; no maintainer commitment to operate a netboot lab.

## Decision

**Defer the netboot daemon.** No `aegis-netbootd` binary, no `aegis-netboot` crate, no iPXE menu rendering function in this milestone.

The compatible foundation will be laid by [#557 (M1A)](https://github.com/aegis-boot/aegis-boot/issues/557): `BootEntryKind::Netboot` is reserved as a documented enum variant so a future netboot epic can attach without re-architecting the boot-entry model.

## Re-entry criteria

Reconsider netboot when ALL of:

1. **≥2 distinct user issues** mention a netboot need (not "would be nice"; a concrete deployment scenario).
2. **A maintainer commits** to operating a netboot lab in their own infrastructure for development and CI testing.
3. **OR** an enterprise commitment that includes both a deployment to test against and a maintainer to land + maintain the work.

Either path produces a new epic that cites this ADR and reopens the design.

## Rationale (from the consensus_vote)

- **Architect:** *"10-crate refactor compounded by inventing demand for netboot/iPXE/PXE that the ROADMAP explicitly defers… textbook DRY violation… YAGNI at scale."*
- **Security:** *"Implementing full netboot opens the system to network-based attacks such as rogue DHCP, PXE spoofing, and MITM on boot streams, which is unjustified given the ROADMAP explicitly defers this to post-v1.0."*
- **DevEx:** *"Substantial API and tooling surface without improving the primary user workflow enough to justify the learning and maintenance cost."*
- **PM:** *"No user signal, no build."*
- **Contrarian:** *"Textbook example of second-system syndrome."*

## What stays in scope

- The `BootEntryKind` enum reserves a `Netboot` variant for future use ([#557](https://github.com/aegis-boot/aegis-boot/issues/557)).
- The architecture doc ([#569](https://github.com/aegis-boot/aegis-boot/issues/569)) frames netboot as a deferred delivery, not a missing one.
- This ADR.

## What stays out

- iPXE menu rendering function (failed YAGNI test in the consensus vote — even as a "validate the direction" probe, the panel rejected it as speculative scaffolding).
- A `aegis-netboot` crate.
- A separate `aegis-netbootd` binary.
- TFTP / proxy-DHCP / UEFI HTTP boot adapters.
- Mirrored asset cache for netboot delivery.

## References

- Plan that proposed the daemon: provided to the maintainer 2026-04-25.
- Codebase audit: parallel-Explore subagents on [epic #556](https://github.com/aegis-boot/aegis-boot/issues/556).
- ROADMAP.md line 22.
