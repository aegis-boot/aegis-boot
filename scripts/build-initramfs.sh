#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Build a reproducible initramfs.cpio.gz that wraps rescue-tui.
#
# The resulting archive is designed to be appended (or concatenated) to a
# signed distro rescue kernel's own initramfs so that, once the kernel
# unpacks it, /usr/bin/rescue-tui runs as the boot-time UI.
#
# Reproducibility is achieved by:
#   - Sorted cpio input (stable file order)
#   - `cpio -o -H newc` (fixed on-disk layout; no timestamps baked into
#     the traversal itself beyond file mtimes)
#   - `find ... -exec touch -d @$SOURCE_DATE_EPOCH` before archiving
#     (flatten every mtime to the same deterministic value)
#   - `gzip -n --no-name` (strip filename + mtime from the gzip header)
#
# See: ADR 0001, issue #14, BUILDING.md.

set -euo pipefail

SOURCE_DATE_EPOCH="${SOURCE_DATE_EPOCH:-1700000000}"
export SOURCE_DATE_EPOCH

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT_DIR="${OUT_DIR:-$ROOT_DIR/out}"
STAGE_DIR="${STAGE_DIR:-$(mktemp -d -t aegis-initramfs-XXXXXX)}"
RESCUE_TUI_BIN="${RESCUE_TUI_BIN:-$ROOT_DIR/target/release/rescue-tui}"

cleanup() { rm -rf -- "$STAGE_DIR"; }
trap cleanup EXIT

log() { printf '[initramfs] %s\n' "$*" >&2; }

require() {
    command -v "$1" >/dev/null 2>&1 || {
        echo "missing required tool: $1" >&2
        exit 1
    }
}

require cpio
require gzip
require find
require sort
require install
require ldd
require sha256sum

if [[ ! -x "$RESCUE_TUI_BIN" ]]; then
    echo "rescue-tui binary not found or not executable: $RESCUE_TUI_BIN" >&2
    echo "build it first: cargo build --release -p rescue-tui" >&2
    exit 1
fi

mkdir -p "$OUT_DIR"

log "staging rootfs layout in $STAGE_DIR"
# POSIX-minimal directory skeleton.
install -d -m 0755 \
    "$STAGE_DIR/bin" \
    "$STAGE_DIR/sbin" \
    "$STAGE_DIR/usr/bin" \
    "$STAGE_DIR/usr/sbin" \
    "$STAGE_DIR/usr/lib" \
    "$STAGE_DIR/lib" \
    "$STAGE_DIR/lib64" \
    "$STAGE_DIR/etc" \
    "$STAGE_DIR/proc" \
    "$STAGE_DIR/sys" \
    "$STAGE_DIR/dev" \
    "$STAGE_DIR/run" \
    "$STAGE_DIR/tmp" \
    "$STAGE_DIR/mnt" \
    "$STAGE_DIR/run/media"

# --- rescue-tui --------------------------------------------------------------
install -m 0755 "$RESCUE_TUI_BIN" "$STAGE_DIR/usr/bin/rescue-tui"

# --- busybox (single static binary provides everything we need) --------------
BUSYBOX_PATH="$(command -v busybox || true)"
if [[ -z "$BUSYBOX_PATH" ]]; then
    echo "busybox not found on PATH; install busybox-static or busybox" >&2
    exit 1
fi
install -m 0755 "$BUSYBOX_PATH" "$STAGE_DIR/bin/busybox"

# --- tpm2_pcrextend (optional — PCR attestation before kexec) ----------------
# If present on the build host, ship it so rescue-tui's TPM measurement path
# can extend PCR 12 before handoff. Without this, the measurement is skipped
# with a logged warning — fine for TPM-less hardware but removes the
# attestation story.
TPM2_PCREXTEND="$(command -v tpm2_pcrextend || true)"
if [[ -n "$TPM2_PCREXTEND" && -f "$TPM2_PCREXTEND" ]]; then
    install -m 0755 "$TPM2_PCREXTEND" "$STAGE_DIR/usr/bin/tpm2_pcrextend"
    log "shipping tpm2_pcrextend for TPM PCR attestation"
else
    log "tpm2_pcrextend not on PATH — TPM measurement will be skipped at runtime"
fi

# --- util-linux losetup (proper loop-device handling) ------------------------
# Busybox's losetup applet doesn't accept `--show` and its behavior for
# loop-device allocation on modern kernels (loop-control, on-demand node
# creation) is inconsistent. Ship util-linux's real losetup if available;
# iso-parser prefers it automatically when present.
UTIL_LOSETUP="$(command -v losetup || true)"
if [[ -n "$UTIL_LOSETUP" && -f "$UTIL_LOSETUP" ]]; then
    # Find the actual binary, not a busybox symlink.
    resolved=$(readlink -f "$UTIL_LOSETUP")
    if ! [[ "$resolved" =~ busybox ]]; then
        install -m 0755 "$resolved" "$STAGE_DIR/sbin/losetup.util-linux"
        copy_libs_placeholder="$STAGE_DIR/sbin/losetup.util-linux"
    fi
fi

# --- Kernel modules (isofs, loop, udf) ---------------------------------------
# Modern Ubuntu distro kernels ship iso9660 support as a MODULE, not
# built-in. Without loading it, `mount -t iso9660 /dev/loop0 /mnt` fails
# even though the loop device exists. Ship the module tree so /init can
# modprobe isofs before attempting ISO mounts.
#
# If AEGIS_KMOD_SRC is set, copy modules from there. Otherwise, copy from
# the currently-running kernel's /lib/modules/$(uname -r)/. When the
# target kernel in the deployment doesn't match the build host's kernel,
# operators must override AEGIS_KMOD_SRC — we warn loudly.
KMOD_SRC="${AEGIS_KMOD_SRC:-}"
if [[ -z "$KMOD_SRC" ]]; then
    # Prefer a kernel whose version matches /boot/vmlinuz-* — that's the
    # kernel the operator actually installed for deployment/testing,
    # not the build host's running kernel. This matters on CI runners
    # where the host kernel (e.g. azure) differs from the installed
    # -generic kernel.
    for vmlinuz in /boot/vmlinuz-*; do
        [[ -e "$vmlinuz" && ! -L "$vmlinuz" ]] || continue
        ver=$(basename "$vmlinuz" | sed 's/^vmlinuz-//')
        candidate="/lib/modules/$ver"
        if [[ -d "$candidate/kernel/fs" ]]; then
            KMOD_SRC="$candidate"
            break
        fi
    done
fi
# Fallback: the running kernel's modules (may be wrong if deployment
# uses a different kernel).
if [[ -z "$KMOD_SRC" ]]; then
    for candidate in /lib/modules/*/kernel/fs; do
        [[ -d "$candidate" ]] || continue
        KMOD_SRC="${candidate%/kernel/fs}"
    done
fi
if [[ -n "$KMOD_SRC" && -d "$KMOD_SRC" ]]; then
    KVER=$(basename "$KMOD_SRC")
    log "shipping kernel modules from $KMOD_SRC (kernel $KVER)"
    MOD_DEST="$STAGE_DIR/lib/modules/$KVER"
    install -d "$MOD_DEST/kernel/fs/isofs"
    install -d "$MOD_DEST/kernel/fs/udf"
    install -d "$MOD_DEST/kernel/drivers/block"
    # Each module may be .ko or .ko.zst depending on compression. Ship
    # whatever the source kernel has.
    # Each module may be .ko, .ko.zst, .ko.xz, or .ko.gz depending on the
    # kernel's CONFIG_MODULE_COMPRESS_* setting. Busybox's modprobe applet
    # handles .ko.gz natively but NOT .ko.zst — Ubuntu's stock kernel
    # compiles as zstd. Decompress on the fly at build time so the shipped
    # module is always plain .ko (works with every known module loader).
    copy_module() {
        local rel_path="$1" dest_dir="$2"
        local src_dir="$KMOD_SRC/$(dirname "$rel_path" | sed 's|^\./||')"
        local base
        base="$(basename "$rel_path")"
        for ext in ko ko.zst ko.xz ko.gz; do
            local src="$src_dir/$base.$ext"
            [[ -f "$src" ]] || continue
            local dest="$dest_dir/$base.ko"
            mkdir -p "$(dirname "$dest")"
            case "$ext" in
                ko)     install -m 0644 "$src" "$dest" ;;
                ko.zst) zstd -d -q -c "$src" > "$dest" && chmod 0644 "$dest" ;;
                ko.xz)  xz -d -c "$src" > "$dest" && chmod 0644 "$dest" ;;
                ko.gz)  gzip -d -c "$src" > "$dest" && chmod 0644 "$dest" ;;
            esac
            return 0
        done
        return 1
    }
    # Distinguish "shipped as a module but we couldn't find it" (real
    # warning) from "compiled into the kernel image" (no .ko exists, no
    # action needed). Reads CONFIG_* from /boot/config-$KVER. Kernels
    # 6.14+ ship loop as built-in (CONFIG_BLK_DEV_LOOP=y), so the
    # previous "WARNING: loop module not found" was a false alarm. (#69)
    KCONFIG="/boot/config-$KVER"
    is_builtin() {
        [[ -r "$KCONFIG" ]] && grep -q "^${1}=y$" "$KCONFIG"
    }
    try_module() {
        local rel="$1" dest="$2" name="$3" kconfig_key="$4"
        if copy_module "$rel" "$dest"; then
            return 0
        fi
        if is_builtin "$kconfig_key"; then
            log "$name is built-in to kernel $KVER (no module to ship)"
        else
            log "WARNING: $name module not found (CONFIG_$kconfig_key not set?)"
        fi
    }
    # Bulk-copy every .ko under a category subtree (e.g. all of
    # drivers/net/phy/). Used for "ship every driver in this
    # hardware family" decisions where cherry-picking each module by
    # name would be brittle — a new vendor PHY chip lands in the
    # kernel + we'd have to update the build script. Operator-
    # reported gaps drove this approach (real-hardware boot 2026-05-03
    # left a Realtek PHY-less stick because we shipped r8169 but
    # missed realtek.ko).
    #
    # depmod runs against the staged tree at the bottom of this
    # block, so dep resolution still works for the bulk-copied
    # modules — modprobe pulls in transitive deps automatically.
    copy_module_tree() {
        local rel_subtree="$1"  # e.g. "kernel/drivers/net/phy"
        local label="$2"        # human-readable for logs
        local src_root="$KMOD_SRC/$rel_subtree"
        if [[ ! -d "$src_root" ]]; then
            log "module tree '$rel_subtree' not present in $KMOD_SRC — skipping"
            return 0
        fi
        local count=0
        local total_kb=0
        while IFS= read -r -d '' src; do
            local rel="${src#"$KMOD_SRC"/}"
            local dir_part
            dir_part=$(dirname "$rel")
            local base
            base=$(basename "$src")
            # Strip extension(s) — copy_module accepts the bare module name.
            local mod="${base%.zst}"
            mod="${mod%.xz}"
            mod="${mod%.gz}"
            mod="${mod%.ko}"
            if copy_module "$dir_part/$mod" "$MOD_DEST/$dir_part"; then
                count=$((count + 1))
                total_kb=$((total_kb + $(stat -c %s "$MOD_DEST/$dir_part/$mod.ko" 2>/dev/null || echo 0) / 1024))
            fi
        done < <(find "$src_root" -type f \
            \( -name '*.ko' -o -name '*.ko.zst' -o -name '*.ko.xz' -o -name '*.ko.gz' \) -print0)
        log "shipped module tree: $label ($count modules, ~${total_kb} KiB)"
    }
    try_module "kernel/fs/isofs/isofs" "$MOD_DEST/kernel/fs/isofs" \
        "isofs" "CONFIG_ISO9660_FS"
    try_module "kernel/fs/udf/udf" "$MOD_DEST/kernel/fs/udf" \
        "udf" "CONFIG_UDF_FS"
    # exfat (Linux 5.7+, mainlined in fs/exfat). Now the default for
    # AEGIS_ISOS as of #243; without this module the rescue-tui's
    # exfat-mount fallback in the AEGIS_ISOS auto-mount loop would
    # silently fail on every device manufactured after that change.
    try_module "kernel/fs/exfat/exfat" "$MOD_DEST/kernel/fs/exfat" \
        "exfat" "CONFIG_EXFAT_FS"
    try_module "kernel/drivers/block/loop" "$MOD_DEST/kernel/drivers/block" \
        "loop" "CONFIG_BLK_DEV_LOOP"

    # --- storage controller modules (#72) ------------------------------
    # Without these, /dev/sd* / /dev/nvme* never appear on real hardware
    # because Ubuntu generic kernels compile most storage drivers as
    # modules. Modules are copied BY RELATIVE PATH so copy_module's
    # src_dir resolution works regardless of where the module actually
    # lives under /lib/modules/<ver>. Each call is best-effort — any
    # missing module logs a warning but doesn't fail the build.

    # SCSI core — prerequisite for sd_mod and usb-storage.
    try_module "kernel/drivers/scsi/scsi_mod" \
        "$MOD_DEST/kernel/drivers/scsi" \
        "scsi_mod" "CONFIG_SCSI"
    try_module "kernel/drivers/scsi/sd_mod" \
        "$MOD_DEST/kernel/drivers/scsi" \
        "sd_mod" "CONFIG_BLK_DEV_SD"

    # SATA / AHCI — most modern desktops and laptops.
    try_module "kernel/drivers/ata/libata" \
        "$MOD_DEST/kernel/drivers/ata" \
        "libata" "CONFIG_ATA"
    try_module "kernel/drivers/ata/libahci" \
        "$MOD_DEST/kernel/drivers/ata" \
        "libahci" "CONFIG_SATA_AHCI"
    try_module "kernel/drivers/ata/ahci" \
        "$MOD_DEST/kernel/drivers/ata" \
        "ahci" "CONFIG_SATA_AHCI"

    # NVMe — modern laptops and workstations.
    try_module "kernel/drivers/nvme/host/nvme-core" \
        "$MOD_DEST/kernel/drivers/nvme/host" \
        "nvme-core" "CONFIG_NVME_CORE"
    try_module "kernel/drivers/nvme/host/nvme" \
        "$MOD_DEST/kernel/drivers/nvme/host" \
        "nvme" "CONFIG_BLK_DEV_NVME"

    # USB core + host controllers — THE deployment path (USB stick).
    try_module "kernel/drivers/usb/core/usbcore" \
        "$MOD_DEST/kernel/drivers/usb/core" \
        "usbcore" "CONFIG_USB"
    try_module "kernel/drivers/usb/common/usb-common" \
        "$MOD_DEST/kernel/drivers/usb/common" \
        "usb-common" "CONFIG_USB_COMMON"
    try_module "kernel/drivers/usb/host/xhci-hcd" \
        "$MOD_DEST/kernel/drivers/usb/host" \
        "xhci-hcd" "CONFIG_USB_XHCI_HCD"
    try_module "kernel/drivers/usb/host/xhci-pci" \
        "$MOD_DEST/kernel/drivers/usb/host" \
        "xhci-pci" "CONFIG_USB_XHCI_PCI"
    try_module "kernel/drivers/usb/host/ehci-hcd" \
        "$MOD_DEST/kernel/drivers/usb/host" \
        "ehci-hcd" "CONFIG_USB_EHCI_HCD"
    try_module "kernel/drivers/usb/host/ehci-pci" \
        "$MOD_DEST/kernel/drivers/usb/host" \
        "ehci-pci" "CONFIG_USB_EHCI_PCI"

    # USB mass storage — both classic (usb-storage) and UAS (USB 3.x).
    try_module "kernel/drivers/usb/storage/usb-storage" \
        "$MOD_DEST/kernel/drivers/usb/storage" \
        "usb-storage" "CONFIG_USB_STORAGE"
    try_module "kernel/drivers/usb/storage/uas" \
        "$MOD_DEST/kernel/drivers/usb/storage" \
        "uas" "CONFIG_USB_UAS"

    # USB HID — keyboards + mice. Without these, the rescue-tui boots
    # but operator input is dead silence; the only way out is power-
    # cycling. Reported on real hardware against an AMD micro-PC with
    # a USB keyboard. Three modules suffice for ~95% of USB input
    # devices: `hid` (core HID layer), `usbhid` (USB transport for
    # HID reports), `hid-generic` (catch-all driver for any HID device
    # without a vendor-specific quirk module). Apple / Logitech-
    # wireless / multi-touch quirk drivers are not shipped here —
    # they're tracked as a follow-up since each adds size and most
    # rescue scenarios use a plain USB keyboard.
    try_module "kernel/drivers/hid/hid" \
        "$MOD_DEST/kernel/drivers/hid" \
        "hid" "CONFIG_HID"
    try_module "kernel/drivers/hid/usbhid/usbhid" \
        "$MOD_DEST/kernel/drivers/hid/usbhid" \
        "usbhid" "CONFIG_USB_HID"
    try_module "kernel/drivers/hid/hid-generic" \
        "$MOD_DEST/kernel/drivers/hid" \
        "hid-generic" "CONFIG_HID_GENERIC"
    # HID vendor-specific quirk drivers — `hid-generic` covers the
    # plain-vanilla-USB-keyboard case but several common families need
    # a quirk driver to expose all keys / scroll behavior / etc:
    #   * Logitech Unifying / Bolt receivers (hid-logitech +
    #     hid-logitech-hidpp + hid-logitech-dj — the hidpp variant
    #     handles HID++ protocol used by Master/MX series; -dj drives
    #     the receiver bus enumeration)
    #   * Microsoft keyboards (Sculpt, All-in-One, etc.) — the
    #     -microsoft quirks expose the F-key media keys correctly
    #   * Apple keyboards (operators bring these to PC rescue work
    #     more often than you'd expect)
    #   * Multi-touch (touchscreens on modern all-in-ones / convertible
    #     laptops booted into rescue mode)
    try_module "kernel/drivers/hid/hid-logitech" \
        "$MOD_DEST/kernel/drivers/hid" \
        "hid-logitech" "CONFIG_HID_LOGITECH"
    try_module "kernel/drivers/hid/hid-logitech-hidpp" \
        "$MOD_DEST/kernel/drivers/hid" \
        "hid-logitech-hidpp" "CONFIG_HID_LOGITECH_HIDPP"
    try_module "kernel/drivers/hid/hid-logitech-dj" \
        "$MOD_DEST/kernel/drivers/hid" \
        "hid-logitech-dj" "CONFIG_HID_LOGITECH_DJ"
    try_module "kernel/drivers/hid/hid-microsoft" \
        "$MOD_DEST/kernel/drivers/hid" \
        "hid-microsoft" "CONFIG_HID_MICROSOFT"
    try_module "kernel/drivers/hid/hid-apple" \
        "$MOD_DEST/kernel/drivers/hid" \
        "hid-apple" "CONFIG_HID_APPLE"
    try_module "kernel/drivers/hid/hid-multitouch" \
        "$MOD_DEST/kernel/drivers/hid" \
        "hid-multitouch" "CONFIG_HID_MULTITOUCH"

    # SD/MMC card readers — many laptops + dev boards use these as a
    # secondary boot/rescue path. sdhci-pci covers the standard host
    # controller; sdhci-acpi covers the ACPI-enumerated variant on
    # newer Intel platforms. mmc_core + mmc_block are usually built
    # into stock distro kernels (CONFIG_MMC=y) but we ship the host
    # controllers anyway since they're loadable.
    # SDHCI core + Command Queue HCI — PARENT modules referenced by
    # sdhci-pci and sdhci-acpi. Without these, modprobe fails the
    # children with `Unknown symbol sdhci_*` (real-hardware report
    # 2026-05-03 dmesg: ~50 unresolved symbols on a single boot).
    try_module "kernel/drivers/mmc/host/sdhci" \
        "$MOD_DEST/kernel/drivers/mmc/host" \
        "sdhci" "CONFIG_MMC_SDHCI"
    try_module "kernel/drivers/mmc/host/cqhci" \
        "$MOD_DEST/kernel/drivers/mmc/host" \
        "cqhci" "CONFIG_MMC_CQHCI"
    try_module "kernel/drivers/mmc/host/sdhci-pci" \
        "$MOD_DEST/kernel/drivers/mmc/host" \
        "sdhci-pci" "CONFIG_MMC_SDHCI_PCI"
    try_module "kernel/drivers/mmc/host/sdhci-acpi" \
        "$MOD_DEST/kernel/drivers/mmc/host" \
        "sdhci-acpi" "CONFIG_MMC_SDHCI_ACPI"

    # NTFS read support — operators sometimes flash the .img onto
    # an existing NTFS-formatted thumb drive or have ISOs on an
    # NTFS-mounted secondary drive. ntfs3 is the in-tree driver
    # (kernel ≥ 5.15); read-only mount is enough for our use case.
    try_module "kernel/fs/ntfs3/ntfs3" \
        "$MOD_DEST/kernel/fs/ntfs3" \
        "ntfs3" "CONFIG_NTFS3_FS"

    # --- network drivers (#655 Phase 1A) -------------------------------
    # Wired Ethernet only — Phase 4 of #655 will add Wi-Fi (firmware
    # blobs make it a separate epic). These cover the QEMU smoke
    # paths (e1000, virtio_net) and the most common USB / PCI NICs
    # operators see in the field. Each is best-effort: if a target
    # kernel doesn't ship the driver as a module, try_module logs a
    # warning and the rescue env just won't see that NIC.
    #
    # PCI Ethernet
    try_module "kernel/drivers/net/ethernet/intel/e1000/e1000" \
        "$MOD_DEST/kernel/drivers/net/ethernet/intel/e1000" \
        "e1000" "CONFIG_E1000"
    try_module "kernel/drivers/net/ethernet/intel/e1000e/e1000e" \
        "$MOD_DEST/kernel/drivers/net/ethernet/intel/e1000e" \
        "e1000e" "CONFIG_E1000E"
    try_module "kernel/drivers/net/ethernet/intel/igb/igb" \
        "$MOD_DEST/kernel/drivers/net/ethernet/intel/igb" \
        "igb" "CONFIG_IGB"
    # Intel NIC dep modules — igb references `dca_*` and
    # `i2c-algo-bit` symbols. Real-hardware dmesg report 2026-05-03
    # showed ~6 unresolved symbols loading igb without these. Both
    # tiny (< 30 KiB); shipping unconditionally is cheap insurance.
    try_module "kernel/drivers/dca/dca" \
        "$MOD_DEST/kernel/drivers/dca" \
        "dca" "CONFIG_DCA"
    try_module "kernel/drivers/i2c/algos/i2c-algo-bit" \
        "$MOD_DEST/kernel/drivers/i2c/algos" \
        "i2c-algo-bit" "CONFIG_I2C_ALGOBIT"
    # Intel I225/I226 2.5GbE — common on modern motherboards (most
    # AM5/LGA1700 boards 2022+). Distinct from igb (1GbE family);
    # operators on newer hardware were silently NIC-less without
    # this.
    try_module "kernel/drivers/net/ethernet/intel/igc/igc" \
        "$MOD_DEST/kernel/drivers/net/ethernet/intel/igc" \
        "igc" "CONFIG_IGC"
    try_module "kernel/drivers/net/ethernet/realtek/r8169" \
        "$MOD_DEST/kernel/drivers/net/ethernet/realtek" \
        "r8169" "CONFIG_R8169"
    # Realtek PHY driver — r8169 (above) loads but cannot bring up
    # the link without `realtek.ko` for PHY ID 0x001cc800
    # (RTL8169-family PHYs). Real-hardware dmesg report 2026-05-03:
    #     r8169 0000:02:00.0: no dedicated PHY driver found for
    #     PHY ID 0x001cc800, maybe realtek.ko needs to be added to
    #     initramfs?
    # Kernel literally tells us what to ship. PHYLIB itself is
    # built-in (CONFIG_PHYLIB=y) so we don't need a libphy module.
    try_module "kernel/drivers/net/phy/realtek/realtek" \
        "$MOD_DEST/kernel/drivers/net/phy/realtek" \
        "realtek" "CONFIG_REALTEK_PHY"
    try_module "kernel/drivers/net/ethernet/realtek/8139too" \
        "$MOD_DEST/kernel/drivers/net/ethernet/realtek" \
        "8139too" "CONFIG_8139TOO"
    try_module "kernel/drivers/net/ethernet/broadcom/tg3" \
        "$MOD_DEST/kernel/drivers/net/ethernet/broadcom" \
        "tg3" "CONFIG_TIGON3"

    # Virtio (QEMU smoke + cloud)
    try_module "kernel/drivers/net/virtio_net" \
        "$MOD_DEST/kernel/drivers/net" \
        "virtio_net" "CONFIG_VIRTIO_NET"

    # USB Ethernet (USB-NIC dongles common in the field — laptops
    # without on-board Ethernet ports often use these).
    try_module "kernel/drivers/net/usb/asix" \
        "$MOD_DEST/kernel/drivers/net/usb" \
        "asix" "CONFIG_USB_NET_AX8817X"
    try_module "kernel/drivers/net/usb/ax88179_178a" \
        "$MOD_DEST/kernel/drivers/net/usb" \
        "ax88179_178a" "CONFIG_USB_NET_AX88179_178A"
    try_module "kernel/drivers/net/usb/cdc_ether" \
        "$MOD_DEST/kernel/drivers/net/usb" \
        "cdc_ether" "CONFIG_USB_NET_CDCETHER"
    try_module "kernel/drivers/net/usb/r8152" \
        "$MOD_DEST/kernel/drivers/net/usb" \
        "r8152" "CONFIG_USB_RTL8152"
    # USB-Net core dependencies — without `usbnet` and `mii` the
    # vendor drivers above won't load even if their .ko exists.
    try_module "kernel/drivers/net/usb/usbnet" \
        "$MOD_DEST/kernel/drivers/net/usb" \
        "usbnet" "CONFIG_USB_USBNET"
    try_module "kernel/drivers/net/mii" \
        "$MOD_DEST/kernel/drivers/net" \
        "mii" "CONFIG_MII"

    # --- vfat NLS fallback (#68, #109) ----------------------------------
    # CONFIG_NLS_DEFAULT="utf8" on Ubuntu but NLS_UTF8 is a module, and
    # the kernel's vfat default `iocharset=iso8859-1` needs the
    # `nls_iso8859-1` module which Ubuntu also keeps loadable. We ship
    # `nls_utf8` and /init mounts vfat with `iocharset=utf8` explicitly.
    # (Earlier comments referenced cp437 as an iocharset — wrong; cp437
    # is a codepage. iocharset must be one of utf8/iso8859-*/koi8/etc.)
    try_module "kernel/fs/nls/nls_utf8" \
        "$MOD_DEST/kernel/fs/nls" \
        "nls_utf8" "CONFIG_NLS_UTF8"
    # Also ship nls_cp437 + nls_iso8859-1 so the kernel's hot-plug
    # automount path doesn't log "IO charset iso8859-1 not found"
    # on real hardware. Our /init uses iocharset=utf8 explicitly,
    # but udev hot-plug mounts that fire before /init gets to the
    # vfat partition are the source of the dmesg noise.
    try_module "kernel/fs/nls/nls_cp437" \
        "$MOD_DEST/kernel/fs/nls" \
        "nls_cp437" "CONFIG_NLS_CODEPAGE_437"
    try_module "kernel/fs/nls/nls_iso8859-1" \
        "$MOD_DEST/kernel/fs/nls" \
        "nls_iso8859-1" "CONFIG_NLS_ISO8859_1"

    # --- bulk-copy resilient driver subtrees (operator request 2026-05-03)
    # The cherry-picked try_module list above covers the most common
    # devices but inevitably misses the long tail (a vendor PHY chip,
    # a HID quirk for a niche keyboard, a USB Ethernet dongle nobody
    # owned at design time). For a rescue environment we'd rather
    # spend 10-15 MiB of initramfs to absorb that tail than ship a
    # stick that's silent on someone's hardware.
    #
    # Subtrees chosen for size/value ratio:
    #   * net/phy        — every PHY chip driver (~1-2 MiB), pulled
    #                      automatically by NIC drivers via PHYLIB
    #   * net/usb        — every USB Ethernet dongle driver (~2-3 MiB)
    #   * hid            — every HID quirk driver — keyboards, mice,
    #                      tablets, gamepads (~3-5 MiB)
    #   * dca, i2c/algos — Intel NIC dep modules (< 100 KiB)
    #   * mmc/host       — every SDHCI variant (~1 MiB)
    #
    # Server-grade NICs (mlx5, bnxt_en, ice, i40e — each 5-10 MiB)
    # stay out per the size budget; tracked in #716. Wi-Fi stays out
    # per #714 (firmware-blob complications).
    copy_module_tree "kernel/drivers/net/phy"   "PHY drivers (Realtek/Broadcom/Marvell/Intel/etc.)"
    copy_module_tree "kernel/drivers/net/usb"   "USB Ethernet (asix/r8152/cdc_ether/dm9601/etc.)"
    copy_module_tree "kernel/drivers/hid"       "HID quirk drivers (Apple/Logitech/MS/Wooting/etc.)"
    copy_module_tree "kernel/drivers/dca"       "Intel DCA (igb/igc dep)"
    copy_module_tree "kernel/drivers/i2c/algos" "I2C algorithms (igb dep)"
    copy_module_tree "kernel/drivers/mmc/host"  "MMC/SDHCI host controllers"

    # Regenerate modules.dep so it references our decompressed .ko paths
    # (source kernel's modules.dep points at .ko.zst). depmod -b rebuilds
    # into the staged tree; no runtime kernel match needed.
    #
    # Fail hard on depmod failure (#138): a silent warning here was
    # producing silent boot-time failures (storage modules missing at
    # the busybox modprobe call in /init because modules.dep still
    # points at the original .ko.zst paths). If depmod is genuinely
    # missing on the build host, the operator should know before
    # producing an image they'll then have to debug under OVMF. Set
    # AEGIS_ALLOW_MISSING_DEPMOD=1 to bypass.
    if command -v depmod >/dev/null 2>&1; then
        depmod_stderr=$(depmod -b "$STAGE_DIR" "$KVER" 2>&1 >/dev/null) || {
            log "FATAL: depmod -b '$STAGE_DIR' '$KVER' failed"
            log "  stderr: $depmod_stderr"
            log "  busybox modprobe would miss dependencies at boot time; aborting."
            log "  (set AEGIS_ALLOW_MISSING_DEPMOD=1 to bypass — not recommended)"
            [ -n "${AEGIS_ALLOW_MISSING_DEPMOD:-}" ] || exit 1
            log "WARNING: proceeding despite depmod failure — AEGIS_ALLOW_MISSING_DEPMOD set."
        }
    else
        log "FATAL: depmod not on PATH; cannot regenerate modules.dep for staged modules."
        log "  install kmod (e.g. 'apt-get install kmod' / 'dnf install kmod') and retry."
        log "  (set AEGIS_ALLOW_MISSING_DEPMOD=1 to bypass — not recommended)"
        [ -n "${AEGIS_ALLOW_MISSING_DEPMOD:-}" ] || exit 1
        log "WARNING: proceeding without depmod — AEGIS_ALLOW_MISSING_DEPMOD set."
    fi
else
    log "WARNING: no kernel modules source found; iso9660 mounts will fail"
    log "  set AEGIS_KMOD_SRC=/lib/modules/<kver> if your target kernel needs modules"
fi
# Applets. Covered: mount, umount, mkdir, ls, sh, cat, mdev.
# rescue-tui doesn't call these directly — they exist for the init script
# below and for emergency shell fallback.
#
# Network applets (udhcpc, ip, kill, route, nslookup, hostname) ship for
# #655 Phase 1: the initramfs has no network stack today; these primitives
# are baked in but NOT auto-invoked at boot — operator triggers DHCP from
# rescue-tui (Phase 1B) or the emergency shell.
for applet in sh mount umount mkdir ls cat dmesg switch_root losetup \
              mdev blkid lsblk modprobe sleep echo ln readlink rmdir \
              findfs uname grep sed cp rm tee date \
              tail head sort basename dd mkfifo wait \
              udhcpc ip kill route nslookup hostname; do
    ln -sf /bin/busybox "$STAGE_DIR/bin/$applet"
done

# udhcpc requires a callback script — busybox calls it to apply DHCP
# leases (set IP, default route, /etc/resolv.conf). Distros usually
# ship one at /usr/share/udhcpc/default.script, but the path is not
# universal and ours is cpio-bundled — write our own minimal version
# so the rescue env doesn't depend on whatever the build host has.
# (#655 Phase 1A.)
install -d "$STAGE_DIR/usr/share/udhcpc"
cat > "$STAGE_DIR/usr/share/udhcpc/default.script" <<'UDHCPC_SH'
#!/bin/sh
# /usr/share/udhcpc/default.script — udhcpc lease-event handler.
#
# Called by busybox udhcpc on lease state changes:
#   bound       new lease acquired
#   renew       lease renewed (same params expected)
#   deconfig    interface should be deconfigured (link down etc.)
#   leasefail   couldn't get a lease
#   nak         server NAK'd a request
#
# We only implement the minimum needed for DNS-resolvable HTTPS to work:
# set IP+netmask+broadcast on the interface, install the default route,
# and write nameservers to /etc/resolv.conf. Anything fancier (search
# domains chained from multiple sources, hostname propagation) is
# deferred until #655 Phase 2 has real callers needing them.

[ -z "$1" ] && { echo "udhcpc-script: missing event arg" >&2; exit 1; }

case "$1" in
    deconfig)
        /bin/ip link set dev "$interface" up 2>/dev/null
        /bin/ip -4 addr flush dev "$interface" 2>/dev/null
        ;;
    bound|renew)
        /bin/ip link set dev "$interface" up 2>/dev/null
        /bin/ip -4 addr flush dev "$interface" 2>/dev/null
        if [ -n "$broadcast" ]; then
            /bin/ip -4 addr add "$ip/$mask" broadcast "$broadcast" dev "$interface" 2>/dev/null
        else
            /bin/ip -4 addr add "$ip/$mask" dev "$interface" 2>/dev/null
        fi
        if [ -n "$router" ]; then
            /bin/ip -4 route flush default 2>/dev/null
            for r in $router; do
                /bin/ip -4 route add default via "$r" dev "$interface" 2>/dev/null
                break  # first router wins (one default route)
            done
        fi
        : > /etc/resolv.conf
        [ -n "$domain" ] && echo "search $domain" >> /etc/resolv.conf
        for ns in $dns; do
            echo "nameserver $ns" >> /etc/resolv.conf
        done
        ;;
    leasefail|nak)
        echo "udhcpc-script: $1 on $interface" >&2
        ;;
esac
exit 0
UDHCPC_SH
chmod 0755 "$STAGE_DIR/usr/share/udhcpc/default.script"

# --- shared library deps of rescue-tui --------------------------------------
# busybox is typically static; rescue-tui links libc + libgcc_s + libm + libpthread.
log "copying shared library dependencies"
copy_libs() {
    local bin="$1"
    # `ldd` output: parse lines like "libc.so.6 => /lib/x86_64-linux-gnu/libc.so.6 (0x...)"
    # and plain "/lib64/ld-linux-x86-64.so.2 (0x...)" for the dynamic linker.
    # Mode 0755 because the dynamic linker is itself an ELF interpreter that
    # the kernel execve's — without the exec bit, every dynamically-linked
    # binary in the initramfs fails with "Permission denied".
    ldd "$bin" 2>/dev/null | awk '
        /=>/ { if ($3 ~ /^\//) print $3 }
        /^\t\// { print $1 }
    ' | sort -u | while IFS= read -r lib; do
        [[ -f "$lib" ]] || continue
        # Follow symlinks so we put the real file at the expected path; this
        # flattens /lib64/ld-linux-* -> /lib/x86_64-linux-gnu/ld-linux-* style
        # distro layouts into a self-contained initramfs.
        local resolved
        resolved="$(readlink -f "$lib")"
        install -D -m 0755 "$resolved" "$STAGE_DIR$lib"
    done
}
copy_libs "$STAGE_DIR/usr/bin/rescue-tui"
# If distro busybox is dynamically linked, ldd would error; ignore silently.
copy_libs "$STAGE_DIR/bin/busybox" 2>/dev/null || true
# util-linux losetup is dynamically linked.
if [[ -f "$STAGE_DIR/sbin/losetup.util-linux" ]]; then
    copy_libs "$STAGE_DIR/sbin/losetup.util-linux"
fi
# tpm2_pcrextend pulls in a bunch of libtss2 — copy them all.
if [[ -f "$STAGE_DIR/usr/bin/tpm2_pcrextend" ]]; then
    copy_libs "$STAGE_DIR/usr/bin/tpm2_pcrextend"
fi

# --- PID 1 init script -------------------------------------------------------
cat > "$STAGE_DIR/init" <<'INIT_SH'
#!/bin/sh
# aegis-boot PID 1 — minimal init that sets up the rescue environment and
# hands control to /usr/bin/rescue-tui.
#
# Deliberately does NOT use `set -e`. Rescue-environment commands routinely
# return non-zero (mount failures on absent filesystems, missing optional
# devices, etc.); aborting PID 1 on any of them triggers a kernel panic and
# reboot loop. Each command handles its own errors explicitly. (#68)

/bin/echo "init: aegis-boot /init starting (PID 1)"

/bin/mount -t proc  proc  /proc
/bin/mount -t sysfs sys   /sys
if /bin/mount -t devtmpfs dev /dev; then
    /bin/echo "init: mounted devtmpfs at /dev"
else
    /bin/echo "init: WARNING devtmpfs mount failed — falling back to tmpfs (no devices visible)"
    /bin/mount -t tmpfs tmpfs /dev
fi
/bin/mount -t tmpfs  run   /run

# Enable kernel SysRq for emergency escape hatches that rescue-tui's
# Help overlay advertises (Alt+SysRq+b reboot, +s sync, +e SIGTERM).
# Without this, those keybind cheatsheets lie — kernel.sysrq=0 is the
# common distro default. Write 1 (all SysRq functions enabled) since
# this is a rescue environment an operator explicitly booted. (#93)
if /bin/echo 1 > /proc/sys/kernel/sysrq 2>/dev/null; then
    /bin/echo "init: SysRq enabled (kernel.sysrq=1) — operator escape hatches active"
else
    /bin/echo "init: WARNING could not enable SysRq (kernel built without CONFIG_MAGIC_SYSRQ?)"
fi

# #109 shakedown: every /bin/echo "init: ..." below is ALSO captured
# to /run/aegis-init.log via a simple helper. After AEGIS_ISOS
# mounts, the file is copied onto the data partition so the
# diagnostics survive a reboot.
INIT_LOG=/run/aegis-init.log
: > "$INIT_LOG" 2>/dev/null

# Load storage controller modules so /dev/sd* / /dev/nvme* appear on
# real hardware. Order matters: bus cores before hosts before class
# drivers. Ignore failures (modules may be built-in on some kernels
# — modprobe logs a no-op and returns 0, or errors out if truly
# absent which is fine). (#72)
/bin/echo "init: loading storage modules"
# filesystem modules: isofs for mounted ISOs, udf for DVD-style
# isos, exfat for the AEGIS_ISOS data partition (default since
# #243), nls_* for FAT character-set translation tables (the ESP is
# vfat; without nls_iso8859-1 + nls_cp437 the kernel hot-plug
# automount path logs "IO charset iso8859-1 not found").
#
# Real-hardware validation of #132 caught the exfat omission: the
# module shipped in the initramfs but was never modprobed, so
# `mount -t exfat /dev/sda2` returned "No such device" and
# rescue-tui discovered 0 ISOs on a fresh direct-install stick.
for m in scsi_mod sd_mod \
         libata libahci ahci \
         nvme-core nvme \
         usbcore usb-common xhci-hcd xhci-pci ehci-hcd ehci-pci \
         usb-storage uas \
         hid usbhid hid-generic \
         hid-logitech hid-logitech-hidpp hid-logitech-dj \
         hid-microsoft hid-apple hid-multitouch \
         sdhci-pci sdhci-acpi ntfs3 \
         nls_utf8 nls_cp437 nls_iso8859-1 \
         loop isofs udf exfat; do
    /bin/modprobe "$m" 2>/dev/null || true
done

# (#655 Phase 1A) Network drivers — modprobe at boot but DON'T trigger
# DHCP. Operator opts in via rescue-tui (Phase 1B) or the emergency
# shell. mii / usbnet load before the vendor drivers that depend on
# them. virtio_net first since it's the QEMU happy path.
for m in mii usbnet \
         virtio_net \
         e1000 e1000e igb igc r8169 8139too tg3 \
         asix ax88179_178a cdc_ether r8152; do
    /bin/modprobe "$m" 2>/dev/null || true
done

# Give the kernel a moment to enumerate USB/NVMe devices before we look.
# USB bus probe can take a second or two on real hardware (hub reset
# sequence, UAS enumeration). 3s is conservative.
/bin/sleep 3
/bin/echo "init: kernel cmdline: $(/bin/cat /proc/cmdline 2>/dev/null || echo '?')"
/bin/echo "init: mounts active:"
/bin/cat /proc/mounts 2>/dev/null | /bin/sed 's/^/init:   /' || /bin/echo "init:   (cat /proc/mounts failed)"

# Prefer the stick's dedicated AEGIS_ISOS data partition if present.
# Resolve LABEL=AEGIS_ISOS via three fallback strategies because busybox's
# findfs does not always recognize FAT32 labels (#68 — observed silently
# returning empty on Ubuntu busybox 1.30 against a FAT32 partition with
# label AEGIS_ISOS, leading to "0 ISOs discovered" on otherwise-loaded
# sticks):
#   1. /bin/findfs LABEL=...           (works for ext*, sometimes FAT)
#   2. /bin/blkid -L AEGIS_ISOS        (label cache, broader fs support)
#   3. /dev/disk/by-label/AEGIS_ISOS   (udev/devtmpfs symlink, most reliable)
/bin/mkdir -p /run/media/aegis-isos
AEGIS_DEV=""
for resolver in \
    "/bin/findfs LABEL=AEGIS_ISOS" \
    "/bin/blkid -L AEGIS_ISOS" \
    "/bin/readlink -f /dev/disk/by-label/AEGIS_ISOS"; do
    candidate=$($resolver 2>/dev/null || true)
    if [ -n "$candidate" ] && [ -b "$candidate" ]; then
        AEGIS_DEV="$candidate"
        break
    fi
done
if [ -n "$AEGIS_DEV" ]; then
    # busybox mount type-autodetect is unreliable; explicit types in
    # fallback order. vfat needs `codepage=437,iocharset=utf8` because
    # the default `iocharset=iso8859-1` is a module (`nls_iso8859-1`)
    # we don't ship — without overriding it the mount fails with
    # "FAT-fs: IO charset iso8859-1 not found". `iocharset=cp437` is
    # NOT a valid value (cp437 is only a codepage / CCS); we ship
    # `nls_utf8` and use that instead. ext4 is the right pick for
    # >4 GiB ISOs and needs no nls. (#68, #109)
    # rw so /init can write aegis-boot-<ts>.log and rescue-tui can
    # tee F10 save-log evidence to the partition. ISO bytes
    # themselves are never modified — iso-probe opens .iso files
    # read-only via loop-mount.
    mount_ok=0
    # Try exfat first since it's the default for AEGIS_ISOS as of #243.
    # ext4 second (the Linux-only DATA_FS=ext4 opt-in path). vfat last
    # — legacy DATA_FS=fat32 sticks; the explicit codepage/iocharset
    # variants are kept because the kernel default `iocharset=iso8859-1`
    # is a module we don't ship (#68, #109).
    for spec in \
        "exfat:rw" \
        "ext4:rw" \
        "vfat:rw,codepage=437,iocharset=utf8" \
        "vfat:rw"; do
        fstype="${spec%%:*}"
        opts="${spec#*:}"
        mount_err=$(/bin/mount -t "$fstype" -o "$opts" "$AEGIS_DEV" /run/media/aegis-isos 2>&1)
        if [ -z "$mount_err" ]; then
            /bin/echo "init: mounted $AEGIS_DEV (LABEL=AEGIS_ISOS, fs=$fstype, rw) -> /run/media/aegis-isos"
            mount_ok=1
            break
        fi
        /bin/echo "init:   tried fs=$fstype: $mount_err"
    done
    [ "$mount_ok" = 0 ] && /bin/echo "init: WARNING: found $AEGIS_DEV but all mount attempts failed (see above)"
else
    /bin/echo "init: AEGIS_ISOS label not resolved by findfs/blkid/by-label — secondary auto-mount loop will try /dev/sd*"
fi

# Diagnostic — dump what we see in /dev so we can debug "0 ISOs found"
# on real hardware. The output goes to the serial console BEFORE
# rescue-tui takes the alternate screen, so it's grep-able from a
# QEMU run log. (#68)
/bin/echo "init: block devices visible:"
for dev in /dev/sd* /dev/nvme* /dev/vd* /dev/mmcblk* /dev/disk/by-label/*; do
    [ -e "$dev" ] && /bin/echo "init:   $dev"
done
/bin/echo "init: end of block-device listing"

# Also auto-mount any other block device that looks like it has a
# filesystem. Covers the case where the user attaches an ISO on a
# secondary stick or USB drive alongside the boot media.
# (#113) Iterate PARTITIONS, not whole disks — /dev/sda doesn't have
# a filesystem and mount attempts print noisy "Can't open blockdev"
# errors. The name pattern requires a trailing digit (partition
# suffix): sd*[0-9] matches sda1/sdb2/... but not sda/sdb.
for dev in /dev/sd*[0-9] /dev/nvme*n*p* /dev/vd*[0-9] /dev/mmcblk*p*; do
    [ -b "$dev" ] || continue
    # Skip the AEGIS_ISOS partition we already mounted.
    [ "$dev" = "${AEGIS_DEV:-}" ] && continue
    name=$(echo "$dev" | /bin/sed 's|.*/||')
    mp="/run/media/$name"
    /bin/mkdir -p "$mp"
    # (#113, #109) Explicit vfat options when auto-mount without a
    # type fails — Linux vfat defaults to iocharset=iso8859-1 which
    # is a module (`nls_iso8859-1`) we don't ship. We ship `nls_utf8`
    # so iocharset=utf8 works. (cp437 is a codepage, not an iocharset
    # — using it as iocharset silently falls back to the default and
    # fails the same way.) Try auto first (ext4/ntfs/etc work fine),
    # fall back to explicit vfat on failure.
    if /bin/mount -o ro "$dev" "$mp" 2>/dev/null; then
        /bin/echo "init: secondary-mount $dev -> $mp"
    elif /bin/mount -t vfat -o ro,codepage=437,iocharset=utf8 \
            "$dev" "$mp" 2>/dev/null; then
        /bin/echo "init: secondary-mount $dev -> $mp (fs=vfat)"
    else
        /bin/rmdir "$mp" 2>/dev/null
    fi
done

export AEGIS_ISO_ROOTS=/run/media/aegis-isos:/run/media
# Prefer util-linux losetup over busybox applet — iso-parser's
# loop-mount path works reliably with real losetup semantics.
if [ -x /sbin/losetup.util-linux ]; then
    /bin/ln -sf /sbin/losetup.util-linux /usr/sbin/losetup
    export PATH=/usr/sbin:/usr/bin:/sbin:/bin
else
    export PATH=/usr/bin:/usr/sbin:/bin:/sbin
fi

# (loop / isofs / udf already modprobed in the early bulk load above.)

# #109 shakedown: snapshot diagnostics into /run/aegis-init.log
# before rescue-tui takes the alternate screen. Everything here is
# readable post-reboot (copied to AEGIS_ISOS just below) and
# post-TUI-exit (still on tmpfs when the shell drops).
{
    /bin/echo "=== /proc/cmdline ==="
    /bin/cat /proc/cmdline 2>/dev/null
    /bin/echo ""
    /bin/echo "=== /proc/mounts ==="
    /bin/cat /proc/mounts 2>/dev/null
    /bin/echo ""
    /bin/echo "=== /dev (block devices) ==="
    /bin/ls -la /dev/sd* /dev/nvme* /dev/vd* /dev/mmcblk* 2>/dev/null
    /bin/ls -la /dev/disk/by-label/ 2>/dev/null
    /bin/echo ""
    /bin/echo "=== /lib/modules ==="
    /bin/ls /lib/modules/ 2>/dev/null
    /bin/echo ""
    /bin/echo "=== dmesg tail ==="
    # Try the dmesg cmd first (works most places). Fall back to a
    # nonblocking dd of /dev/kmsg if dmesg returns nothing — some
    # kernels CAP_SYSLOG-restrict dmesg even from PID 1, but
    # /dev/kmsg stays readable. Capture each separately so the log
    # tells us WHICH path produced output.
    _dmesg_out=$(/bin/dmesg 2>&1)
    if [ -n "$_dmesg_out" ]; then
        /bin/echo "$_dmesg_out" | /bin/tail -200
    else
        /bin/echo "(dmesg cmd produced no output — falling back to /dev/kmsg)"
        /bin/dd if=/dev/kmsg of=/dev/stdout iflag=nonblock 2>/dev/null | /bin/head -200 \
            || /bin/echo "(/dev/kmsg also unreadable)"
    fi
    /bin/echo ""
    # (#717 follow-up: USB/HID enumeration audit) — what the kernel
    # actually saw + what we modprobed + which input devices showed up.
    # Without these the operator-facing "keyboard frozen in TUI" report
    # has no way to distinguish "module didn't load" / "device didn't
    # enumerate" / "input event never reached the TUI".
    /bin/echo "=== lsmod (post-modprobe) ==="
    if [ -x /sbin/lsmod ] || [ -x /usr/sbin/lsmod ]; then
        /sbin/lsmod 2>/dev/null || /usr/sbin/lsmod 2>/dev/null
    else
        # Busybox doesn't ship lsmod by default; /proc/modules has the
        # raw form `<name> <size> <refcount> <deps> <state> <addr>`.
        /bin/cat /proc/modules 2>/dev/null | /bin/sort
    fi
    /bin/echo ""
    /bin/echo "=== /dev/input (input devices the kernel exposed) ==="
    /bin/ls -la /dev/input/ 2>/dev/null || /bin/echo "(no /dev/input)"
    /bin/echo ""
    /bin/echo "=== /sys/class/input/ (with names) ==="
    for _d in /sys/class/input/event* /sys/class/input/mouse* /sys/class/input/input*; do
        [ -e "$_d" ] || continue
        _basename=$(/bin/basename "$_d")
        # Per-input device name comes from `device/name` for event
        # nodes, `name` for input nodes. Try both; missing is fine.
        _name=$(/bin/cat "$_d/device/name" 2>/dev/null || /bin/cat "$_d/name" 2>/dev/null)
        /bin/echo "  $_basename: ${_name:-?}"
    done
    /bin/echo ""
    /bin/echo "=== /sys/bus/usb/devices (USB topology) ==="
    for _d in /sys/bus/usb/devices/*; do
        [ -e "$_d/idVendor" ] || continue
        _vid=$(/bin/cat "$_d/idVendor" 2>/dev/null)
        _pid=$(/bin/cat "$_d/idProduct" 2>/dev/null)
        _mfg=$(/bin/cat "$_d/manufacturer" 2>/dev/null)
        _prod=$(/bin/cat "$_d/product" 2>/dev/null)
        /bin/echo "  ${_d##*/}: ${_vid}:${_pid} ${_mfg:-?} ${_prod:-?}"
    done
} >> "$INIT_LOG" 2>&1

# Copy the snapshot to AEGIS_ISOS so it survives a reboot. Best-
# effort — cp fails silently if the partition isn't mounted or is
# read-only. /run/aegis-init.log stays on tmpfs regardless.
#
# `_ts` becomes the per-boot identifier shared by every log file
# this script writes. Set OUTSIDE the cp branch so the rescue-tui
# stderr capture below uses the same timestamp even when AEGIS_ISOS
# isn't mounted.
_ts=$(/bin/date +%Y%m%d-%H%M%S 2>/dev/null || /bin/echo "boot")
if [ -d /run/media/aegis-isos ]; then
    if /bin/cp "$INIT_LOG" "/run/media/aegis-isos/aegis-boot-${_ts}.log" 2>/dev/null; then
        # Force the cp + the snapshot's data blocks out to disk
        # before the next phase runs. exfat doesn't auto-sync; on
        # the AMD micro-PC report (2026-05-03 04:37) the file got
        # created but data never flushed before the operator's
        # power-button reset, leaving a 0-byte log on AEGIS_ISOS.
        # An explicit sync trades ~10ms for a guaranteed forensic
        # trail.
        /bin/sync 2>/dev/null
        /bin/echo "init: wrote init log to AEGIS_ISOS/aegis-boot-${_ts}.log"
    fi
fi

# Stage-checkpoint helper. Each call appends one line to the
# per-boot stages log + sync's it. If the next boot shows
# `aegis-boot-<ts>-stages.log` ending mid-pipeline, we know exactly
# which phase the operator's reset interrupted. Tiny + cheap; runs
# every boot so we don't have to add diagnostic-mode branching.
checkpoint() {
    if [ -d /run/media/aegis-isos ] && [ -w /run/media/aegis-isos ]; then
        /bin/echo "$(/bin/date '+%H:%M:%S' 2>/dev/null) [stage=$1]" \
            >> "/run/media/aegis-isos/aegis-boot-${_ts}-stages.log" 2>/dev/null
        /bin/sync 2>/dev/null
    fi
}
checkpoint "snapshot-flushed"

# Verbose-pause: if the kernel cmdline has aegis.verbose=1, pause
# for 30s (or until Enter) so the operator can read the pre-TUI
# diagnostics before the alt-screen takes over. (#109)
if /bin/grep -q "aegis.verbose=1" /proc/cmdline 2>/dev/null; then
    /bin/echo ""
    /bin/echo "init: aegis.verbose=1 — pausing 30s before rescue-tui."
    /bin/echo "init: press Enter to continue sooner."
    /bin/echo ""
    /bin/echo "Full init log: $INIT_LOG"
    /bin/echo ""
    /bin/sleep 30 &
    _pid=$!
    read -r _line 2>/dev/null || true
    /bin/kill "$_pid" 2>/dev/null
fi

export TERM=linux

# (#675) Harness-driven test modes. Cmdline `aegis.test=<name>` sets
# AEGIS_TEST=<name> and rescue-tui short-circuits its TUI to run the
# named scripted check. See docs/rescue-tui-serial-format.md for the
# serial-output contract aegis-hwsim grep-pins against. Quoted to
# accept future modes without re-editing /init.
for arg in $(/bin/cat /proc/cmdline 2>/dev/null); do
    case "$arg" in
        aegis.test=*)
            export AEGIS_TEST="${arg#aegis.test=}"
            /bin/echo "init: AEGIS_TEST=$AEGIS_TEST (cmdline-driven test mode)"
            ;;
    esac
done

# Verbose-by-default rescue-tui logging while we work through the
# real-hardware diagnostic gap (USB-HID input + render artifacting on
# AMD micro-PC, reported 2026-05-03). RUST_LOG honors any operator-
# set value first; otherwise we crank to trace on the rescue-tui
# crates so the persisted stderr capture below carries the full
# event stream. Once the diagnostic gap closes we'll dial this back
# to the original `rescue_tui=info` default.
if [ -z "${RUST_LOG:-}" ]; then
    export RUST_LOG="rescue_tui=trace,iso_probe=debug,kexec_loader=debug,aegis_fetch=info,info"
fi

# Persist rescue-tui stderr to AEGIS_ISOS so the operator-reported
# "screen full of artifacts, dialogs sort of worked, typing 'boot'
# did nothing" symptoms (2026-05-03) leave a forensic trail. Without
# this redirect, rescue-tui's tracing output goes to the framebuffer
# console — visible during the boot but lost the moment ratatui
# takes the alternate screen + the operator power-cycles.
#
# Best-effort: if AEGIS_ISOS isn't writable we fall back to /tmp so
# the file at least exists in tmpfs (gone after reboot, but readable
# via the emergency shell).
RESCUE_TUI_LOG=""
if [ -d /run/media/aegis-isos ] && [ -w /run/media/aegis-isos ]; then
    RESCUE_TUI_LOG="/run/media/aegis-isos/aegis-boot-${_ts:-boot}-rescue-tui.log"
elif [ -d /tmp ]; then
    RESCUE_TUI_LOG="/tmp/aegis-boot-rescue-tui.log"
fi
if [ -n "$RESCUE_TUI_LOG" ]; then
    /bin/echo "init: rescue-tui stderr → $RESCUE_TUI_LOG (RUST_LOG=$RUST_LOG)"
fi
checkpoint "pre-rescue-tui-exec"

# Hand off. Exit code semantics (#90):
#   0        — operator chose Quit → drop to emergency shell (old default)
#   42       — operator chose the rescue-shell entry explicitly
#   anything — crash / unclean exit → emergency shell
# All paths land in /bin/sh; the different branches only differ in the
# banner so an operator reading the serial log can tell which happened.
#
# Tee discipline: rescue-tui's stderr must land in BOTH the persistent
# log file AND on the original console (where the e2e QEMU canaries
# grep for "aegis-boot rescue-tui starting"). A naive
# `2>"$RESCUE_TUI_LOG"` redirect lands it in the file only — the
# canaries time out waiting for the marker. Busybox sh doesn't have
# process substitution (`>(...)`) so we do it via a named pipe +
# background tee. The tee inherits fd 2 (= console) from the script,
# so writing to file + fd 2 forks the stream.
TUI_FIFO=""
TEE_PID=""
if [ -n "$RESCUE_TUI_LOG" ]; then
    TUI_FIFO="/tmp/aegis-tui-stderr-fifo"
    /bin/mkfifo "$TUI_FIFO" 2>/dev/null || TUI_FIFO=""
fi
if [ -n "$TUI_FIFO" ]; then
    # Background a periodic-sync loop so the captured stderr file
    # gets flushed to AEGIS_ISOS every 2s. Without this, the
    # tee-written file lives in the kernel page cache + the exfat
    # write-back queue; an operator power-button reset (observed
    # 2026-05-03) leaves the file at 0 bytes on disk even though
    # tee has buffered megabytes into it. Loop terminates when
    # rescue-tui's parent script does (loop exit on EOF read).
    (
        while [ -f "$RESCUE_TUI_LOG" ]; do
            /bin/sleep 2 2>/dev/null
            /bin/sync 2>/dev/null
        done
    ) &
    SYNC_PID=$!
    /bin/tee "$RESCUE_TUI_LOG" < "$TUI_FIFO" >&2 &
    TEE_PID=$!
    /usr/bin/rescue-tui 2>"$TUI_FIFO"
    rc=$?
    /bin/wait "$TEE_PID" 2>/dev/null
    /bin/kill "$SYNC_PID" 2>/dev/null
    /bin/rm -f "$TUI_FIFO"
    /bin/sync 2>/dev/null
else
    # Fallback: no fifo (mkfifo unavailable, or RESCUE_TUI_LOG
    # unset). Run rescue-tui without persisting stderr — the
    # console-only path matches v0.18 behavior.
    /usr/bin/rescue-tui
    rc=$?
fi
case "$rc" in
    0)   /bin/echo "init: rescue-tui quit cleanly; dropping to emergency shell" ;;
    42)  /bin/echo "init: rescue shell requested by operator (#90)" ;;
    *)   /bin/echo "init: rescue-tui exited unexpectedly (rc=$rc); dropping to emergency shell" ;;
esac
# Persist rc + a tail of the captured stderr so the post-boot operator
# (or me, on a future stick inspection) can see the exit cause without
# re-rebooting. Best-effort, doesn't fail the init.
if [ -n "$RESCUE_TUI_LOG" ] && [ -d /run/media/aegis-isos ] && [ -w /run/media/aegis-isos ]; then
    _exit_log="/run/media/aegis-isos/aegis-boot-${_ts:-boot}-rescue-tui-exit.log"
    {
        /bin/echo "rescue-tui exited rc=$rc"
        /bin/echo "stderr capture path: $RESCUE_TUI_LOG"
        /bin/echo ""
        /bin/echo "=== last 80 lines of stderr ==="
        /bin/tail -80 "$RESCUE_TUI_LOG" 2>/dev/null || /bin/echo "(stderr file unreadable)"
    } > "$_exit_log" 2>/dev/null && \
        /bin/echo "init: rescue-tui exit summary → $_exit_log"
    /bin/sync 2>/dev/null
fi
checkpoint "rescue-tui-exited rc=$rc"
exec /bin/sh
INIT_SH
chmod 0755 "$STAGE_DIR/init"

# --- deterministic mtime flattening -----------------------------------------
log "flattening mtimes to SOURCE_DATE_EPOCH=$SOURCE_DATE_EPOCH"
find "$STAGE_DIR" -exec touch -h -d "@$SOURCE_DATE_EPOCH" {} +

# --- cpio + gzip assembly ---------------------------------------------------
OUT_CPIO="$OUT_DIR/initramfs.cpio"
OUT_GZ="$OUT_DIR/initramfs.cpio.gz"

log "creating cpio archive (newc, sorted)"
( cd "$STAGE_DIR" && find . -mindepth 1 -print0 | LC_ALL=C sort -z \
    | cpio --null --create --format=newc --quiet --reproducible \
  ) > "$OUT_CPIO"

log "compressing with deterministic gzip"
gzip --no-name --best --stdout "$OUT_CPIO" > "$OUT_GZ"
rm -f "$OUT_CPIO"

( cd "$OUT_DIR" && sha256sum initramfs.cpio.gz > initramfs.cpio.gz.sha256 )

size=$(stat -c '%s' "$OUT_GZ")
hash=$(awk '{print $1}' "$OUT_DIR/initramfs.cpio.gz.sha256")
log "wrote $OUT_GZ ($size bytes)"
log "sha256: $hash"

if [[ "$size" -gt 25165824 ]]; then
    echo "initramfs exceeds 24 MB size budget ($size bytes); investigate" >&2
    exit 1
fi
# Phase 1A networking primitives self-check: every applet we link
# against busybox must resolve to the busybox binary. Catches the
# build-host-busybox-without-applet-X case before it bites at runtime
# in QEMU. (Best-effort; busybox embedded applet list isn't queryable
# without a working binary, so we just confirm the symlinks exist.)
for required in udhcpc ip kill route; do
    if [[ ! -L "$STAGE_DIR/bin/$required" ]]; then
        echo "build-initramfs: WARNING: $required symlink missing — Phase 1A networking will be degraded" >&2
    fi
done
[[ -x "$STAGE_DIR/usr/share/udhcpc/default.script" ]] || {
    echo "build-initramfs: ERROR: udhcpc default.script missing or not executable" >&2
    exit 1
}
