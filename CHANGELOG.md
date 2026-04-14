# Changelog

All notable changes to aegis-boot are recorded here. Format: [Keep a Changelog](https://keepachangelog.com/en/1.1.0/). This project adheres to [Semantic Versioning](https://semver.org/).

## [0.1.0] — 2026-04-14

First release. The rescue runtime boots end-to-end in CI: a real kernel unpacks a reproducible `initramfs.cpio.gz`, PID 1 runs, `rescue-tui` reaches first render, and the whole chain is verified on every PR.

### Architecture

- **ADR 0001** — signed Linux rescue + ratatui TUI + `kexec_file_load(2)` runtime. Decided by 5-agent consensus vote (higher-order, supermajority, 4–1) preserved in [`docs/adr/0001-runtime-architecture.md`](./docs/adr/0001-runtime-architecture.md).

### Crates

- **`iso-parser`** (existing, preserved) — ISO9660 / El Torito / UDF discovery, `cargo-fuzz`-covered.
- **`iso-probe`** (new, v0.1.0) — sync facade + RAII `PreparedIso` for kexec handoff. Real loop-mount integration test (#16).
- **`kexec-loader`** (new, v0.1.0) — audited `unsafe` FFI over `kexec_file_load(2)` only. Classifies `EKEYREJECTED` / `EPERM` / `ENOEXEC`. `kexec_load(2)` and `KEXEC_FILE_UNSAFE` deliberately not exposed.
- **`rescue-tui`** (new, v0.1.0) — ratatui binary. Pure state-machine + renderer split; stderr startup banner for serial consoles.

### Build + ship

- `Dockerfile.locked` — Ubuntu 22.04 (digest-pinned) + Rust 1.85, no EDK II (dropped per ADR 0001). `rescue-tui` binary is byte-reproducible under `SOURCE_DATE_EPOCH`.
- `scripts/build-initramfs.sh` — produces `out/initramfs.cpio.gz` (3.6 MB, byte-reproducible: sha256 `d82acb9e170b9750a40c23470dad45d15cd0a7cc48234f11b36e9d41a31bbb95`).
- `scripts/qemu-smoke.sh` — boots the initramfs under QEMU and asserts the TUI starts.

### CI (11 checks per PR)

Test (1.85) · Test (stable) · SAST (semgrep) · cargo-deny · gitleaks · CycloneDX SBOM · Nix smoke · reproducible-build · initramfs build · loop-mount integration · QEMU smoke boot.

### Documentation

- [`THREAT_MODEL.md`](./THREAT_MODEL.md) rewritten for the Option B chain.
- [`BUILDING.md`](./BUILDING.md) — reproducible build + initramfs assembly recipe.
- [`docs/adr/0001-runtime-architecture.md`](./docs/adr/0001-runtime-architecture.md) — decision record incl. preserved security dissent + revisit triggers.

### Known limits

- **Secure Boot chain** is demonstrated by design but not yet CI-verified. `aegis-boot` trusts shim + a distro-signed kernel; the initramfs rides that kernel's signature. Real MOK + SB enforcement verification is a separate follow-up.
- **`iso_probe::lookup_quirks()`** returns an empty list for every distribution. Real population tracked in [#6](https://github.com/williamzujkowski/aegis-boot/issues/6). Callers must not treat empty as "safe."
- **kexec handoff** is unit-tested via errno classification but not yet end-to-end exercised with a signed target ISO.
