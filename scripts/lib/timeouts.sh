# SPDX-License-Identifier: MIT OR Apache-2.0
# shellcheck shell=sh
# shellcheck disable=SC2034
# (Constants are consumed by sibling scripts that source this file;
# the linter cannot see those references from here, so SC2034
# "apparently unused" fires falsely. Suppression is correct.)
#
# Named QEMU / boot-smoke timeout constants — single source of truth
# for both shell scripts and CI workflows. Source this file from
# anywhere that previously hardcoded a `TIMEOUT_SECONDS=<N>` literal.
#
# Why:
#   The same QEMU boot was being timed out at 12 different places
#   across scripts/*.sh + .github/workflows/*.yml (issue #733). When
#   #727 raised the boot canary 90 → 180s, sibling overrides had to
#   move in lockstep by hand. This file is the lock.
#
# Usage from a shell script:
#   . "$(dirname "${BASH_SOURCE[0]}")/lib/timeouts.sh"
#   TIMEOUT_SECONDS="${TIMEOUT_SECONDS:-$QEMU_SMOKE_TIMEOUT_DEFAULT}"
#
# Usage from a GitHub Actions workflow step:
#   - run: |
#       . ./scripts/lib/timeouts.sh
#       echo "TIMEOUT_SECONDS=$QEMU_KEXEC_E2E_TIMEOUT_DEFAULT" >> "$GITHUB_ENV"
#
# Each constant has a rationale comment so a reader knows why the
# number is what it is — not just what.

# rescue-tui first-render canary under -nographic QEMU. The 54MB+
# initramfs takes ~80s to decompress on a shared GHA runner; +50%
# headroom for slow runners and post-#727 driver-tree size growth.
QEMU_SMOKE_TIMEOUT_DEFAULT=180

# Full kexec_file_load round-trip from rescue-tui auto-kexec into a
# fixture ISO + first dmesg line of the new kernel. Wider than
# qemu-smoke because we're booting TWO kernels back-to-back: the
# host kernel + the kexec'd target. Same +50% headroom rationale.
QEMU_KEXEC_E2E_TIMEOUT_DEFAULT=180

# OVMF + shim handshake only — no kernel boot, no rescue-tui. Just
# proves the signed chain loads under SecBoot enforcement. Tight bound
# because the test path is short and a regression would be fast.
OVMF_SECBOOT_SMOKE_TIMEOUT_DEFAULT=30

# OVMF + shim + grub + signed-kernel + rescue-tui first-render under
# SecBoot enforcement. Heaviest path; ~2x qemu-smoke because the
# signed boot chain has more handshake overhead than `-kernel … -initrd`.
OVMF_SECBOOT_E2E_TIMEOUT_DEFAULT=120

# Inline OVMF SecBoot boots embedded in e2e-suite.yml jobs (mkusb,
# direct-install, update-rotate). Same shape as
# OVMF_SECBOOT_E2E_TIMEOUT_DEFAULT; deliberately a separate name
# so a future divergence (e.g. update-rotate adding a TPM dance)
# can move independently without splitting the script default.
OVMF_INLINE_BOOT_TIMEOUT_DEFAULT=120

# Single test-mode dispatcher (`aegis.test=<NAME>`). Each dispatch
# is a one-shot — flip into the named test, emit a landmark line,
# exit. No full TUI, no kexec. Tight bound because failure should
# manifest fast or never.
TEST_MODE_DISPATCHER_TIMEOUT_DEFAULT=60
