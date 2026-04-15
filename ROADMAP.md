# Roadmap

A forward-looking sketch. The CHANGELOG tells what already shipped; this file says where we're going. Everything here is subject to change — file an issue if you think the priorities are wrong.

## Now (active)

- **#77** Repo cleanup — CONTRIBUTING, ROADMAP, scripts/README, docs/README *(this PR)*
- **#76** Project branding — logo, tagline, social preview, README hero
- **#78** Docs/content accuracy audit cadence (recurring; ran at v0.7.0)

## Next (toward v1.0.0)

- **#51** Real-hardware shakedown on Framework / ThinkPad / Dell. Required for v1.0.0. Needs physical access — can't run from CI.
- **crates.io publishing** ([#51](https://github.com/williamzujkowski/aegis-boot/issues/51) gate) — `iso-parser`, `iso-probe`, `kexec-loader` go up at v1.0.0-rc1. `rescue-tui` and `aegis-fitness` stay repo-local.
- **`scripts/release.sh`** — manual asset upload is fine for v0.x; for v1.0.0 we want one command.
- **Reproducibility extension** — currently only `rescue-tui` is verified reproducible under SOURCE_DATE_EPOCH. Stretch: include `initramfs.cpio.gz` once we can pin the busybox version.

## Later (post-1.0)

- **Architecture variants** — aarch64 build, riscv64 exploration. Separate epics, deferred until x86_64 is solid on real hardware.
- **Remote attestation** — beyond TPM PCR 12 measurement (which we already do), wire up a referenceable verifier. Probably a small companion crate.
- **Network boot / PXE** — explicitly out of scope for v1.0; reconsider if real users ask.
- **Custom signing chain** — let operators substitute their own shim → grub → kernel for environments that don't trust Microsoft's CA.

## Non-goals (probably forever)

- A full UEFI application (rejected in [ADR 0001](./docs/adr/0001-runtime-architecture.md), Option A)
- Linking `libtss2-esys` (we shell out to `tpm2_pcrextend`; see [`crates/rescue-tui/src/tpm.rs`](./crates/rescue-tui/src/tpm.rs))
- Native Windows ISO `kexec` (different boot protocol; `Quirk::NotKexecBootable` blocks it explicitly)
- A web UI / management console — this is a boot tool, not a server

## How items get on this roadmap

Open an issue. If three or more "yes" answers from [CONTRIBUTING.md's bar](./CONTRIBUTING.md), it lands here. The order within a section is rough — sequencing usually emerges from dependencies, not voting.
