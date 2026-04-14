# Aegis-Boot

Reproducible UEFI Secure Boot orchestration infrastructure.

## Components

- **[BUILDING.md](./BUILDING.md)** — Reproducible build setup (Docker + Nix)
- **[THREAT_MODEL.md](./THREAT_MODEL.md)** — UEFI Secure Boot threat model (PK/KEK/MOK/SBAT)
- **[Dockerfile.locked](./Dockerfile.locked)** — Pinned base image (Ubuntu 22.04, Rust 1.75.0, EDK II stable202311)
- **[flake.nix](./flake.nix)** — Nix flake for declarative dev environments
- **[scripts/scaffold-aegis-boot.sh](./scripts/scaffold-aegis-boot.sh)** — Bootstraps Rust workspace
- **[crates/iso-parser](./crates/iso-parser)** — ISO installation media parser

## Status

Work in progress. See BUILDING.md for setup and THREAT_MODEL.md for security boundaries.

## License

Dual-licensed under [Apache-2.0](./LICENSE-APACHE) or [MIT](./LICENSE-MIT) at your option.

## Security

Report vulnerabilities privately — see [SECURITY.md](./SECURITY.md).
