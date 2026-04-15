# Scripts

Build / test / boot helpers for aegis-boot. All scripts are POSIX-ish bash, run from any directory (they self-locate the repo root via `${BASH_SOURCE[0]}`).

| Script | Purpose | Needs sudo? |
|---|---|---|
| [`build-initramfs.sh`](./build-initramfs.sh) | Assemble `out/initramfs.cpio.gz` (busybox + rescue-tui + storage modules + `/init`). Reproducible under `SOURCE_DATE_EPOCH`. | no |
| [`mkusb.sh`](./mkusb.sh) | Build the bootable GPT image at `out/aegis-boot.img` (ESP + AEGIS_ISOS partitions). | yes (kernel reads) |
| [`qemu-try.sh`](./qemu-try.sh) | Boot `out/aegis-boot.img` interactively under QEMU + OVMF Secure Boot. Empty AEGIS_ISOS by default. | no |
| [`qemu-loaded-stick.sh`](./qemu-loaded-stick.sh) | Build a fresh stick image, copy ISOs from `./test-isos/` onto AEGIS_ISOS, boot under QEMU. Supports `--attach {virtio,sata,usb}`. | yes (loop-mount) |
| [`qemu-smoke.sh`](./qemu-smoke.sh) | Headless OVMF boot, asserts rescue-tui startup banner. CI smoke test. | no |
| [`qemu-kexec-e2e.sh`](./qemu-kexec-e2e.sh) | Full kexec hand-off test using a fixture ISO mounted as `-cdrom`. Uses `AEGIS_AUTO_KEXEC` to skip the TUI. | yes (loop-mount + kexec) |
| [`ovmf-secboot-smoke.sh`](./ovmf-secboot-smoke.sh) | Boot the signed chain under OVMF SB enforcing; assert SB enforcement. | no |
| [`ovmf-secboot-e2e.sh`](./ovmf-secboot-e2e.sh) | Full OVMF SB E2E including MOK enrollment scenarios. | no |
| [`dev-test.sh`](./dev-test.sh) | Run the 8-stage local CI equivalent (fmt → clippy → test → initramfs → mkusb → qemu-try → kexec-e2e → fitness audit). | yes for stages 5+7 |

## Conventions

- All scripts `set -euo pipefail`.
- Output goes to stderr with a `[<script>]` prefix for grep-ability.
- Build artifacts land under `out/` (gitignored).
- Test ISOs live in `./test-isos/` (gitignored). Drop your own there.
- Sudo prompts are explicit — no scripts elevate without saying so.

## When to use which

| You want to... | Run |
|---|---|
| Iterate on Rust code | `cargo test --workspace` |
| Iterate on `/init` or initramfs layout | `./build-initramfs.sh` + boot under `qemu-loaded-stick.sh` |
| Iterate on the boot chain (shim/grub/kernel) | `./mkusb.sh` + `./qemu-try.sh` |
| Manually drive the rescue-tui with real ISOs | `./qemu-loaded-stick.sh -d ~/iso-stash -i` |
| Pre-push sanity check | `./dev-test.sh` |

## Prereqs

See [`docs/LOCAL_TESTING.md`](../docs/LOCAL_TESTING.md) for the apt install one-liner.
