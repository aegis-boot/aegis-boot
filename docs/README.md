# Documentation index

| Audience | Doc | What's in it |
|---|---|---|
| **Operator** (deploying aegis-boot) | [`USB_LAYOUT.md`](./USB_LAYOUT.md) | GPT + ESP + AEGIS_ISOS scheme; how to dd to a stick; how to drop ISOs onto the data partition |
| **Operator** | [`compatibility/iso-matrix.md`](./compatibility/iso-matrix.md) | Per-distro kexec compatibility — what works, what surfaces a quirk |
| **Developer** | [`LOCAL_TESTING.md`](./LOCAL_TESTING.md) | 8-stage local CI equivalent; `qemu-loaded-stick.sh --attach` modes; iteration recipes |
| **Developer** | [`../BUILDING.md`](../BUILDING.md) | Reproducible build setup (Docker `Dockerfile.locked` + Nix `flake.nix`) |
| **Developer** | [`../scripts/README.md`](../scripts/README.md) | What each script does and when to run it |
| **Architect** | [`adr/`](./adr/) | Architecture Decision Records |
| **Architect** | [`adr/0001-runtime-architecture.md`](./adr/0001-runtime-architecture.md) | Why "signed Linux rescue + ratatui + kexec" (Option B) over EDK II / dracut |
| **Security reviewer** | [`../SECURITY.md`](../SECURITY.md) | Vulnerability reporting (private path) |
| **Security reviewer** | [`../THREAT_MODEL.md`](../THREAT_MODEL.md) | UEFI SB threat model — PK/KEK/MOK/SBAT, kexec chain of trust |
| **Maintainer** | [`content-audit.md`](./content-audit.md) | Log of doc-accuracy audits + cadence |
| **Contributor** | [`../CONTRIBUTING.md`](../CONTRIBUTING.md) | Workflow, commit style, PR checklist |
| **Contributor** | [`../CODE_OF_CONDUCT.md`](../CODE_OF_CONDUCT.md) | Contributor Covenant 2.1 |
| Everyone | [`../CHANGELOG.md`](../CHANGELOG.md) | Per-release notes, what shipped and when |
| Everyone | [`../README.md`](../README.md) | Project overview, quickstart, status |

## Status pages

- **Releases:** https://github.com/williamzujkowski/aegis-boot/releases
- **Roadmap:** [`../ROADMAP.md`](../ROADMAP.md)
- **Open epics / issues:** https://github.com/williamzujkowski/aegis-boot/issues
