// SPDX-License-Identifier: MIT OR Apache-2.0

//! Linux removable-drive detection via sysfs.
//!
//! Enumerates `/sys/block/sd*` looking for removable USB mass storage
//! devices. Filters out system drives, `NVMe`, loop devices, and anything
//! not flagged as removable by the kernel.
//!
//! ## USB-attached SSD fallback (#661)
//!
//! Some USB-attached SSDs (Kingston, Samsung T7, etc.) report
//! `/sys/block/<dev>/removable == 0` even though they're on the USB
//! bus and the operator wants to flash them. The strict
//! `removable == 1` gate excludes them, leaving the user with a
//! confusing "not a removable drive" error against an obviously-USB
//! device.
//!
//! Fallback: when `removable == 0`, accept the drive anyway if its
//! transport is USB **and** its size is at or below
//! [`USB_FALLBACK_SIZE_CAP_BYTES`] (2 TiB). The size ceiling
//! prevents false-positives on USB-attached external HDDs that
//! happen to be plugged in for backup purposes — those are
//! typically > 2 TiB, and flashing an operator's backup drive
//! would be catastrophic.

use super::{BlockDevice, BlockDeviceTransport, Drive};
use std::fs;
use std::path::{Path, PathBuf};

/// Hard size ceiling for the USB-fallback path (#661). Drives
/// reporting `removable == 0` are accepted only when their
/// transport is USB AND their size is at or below this value.
/// 2 TiB covers every common USB stick + USB SSD product on the
/// market (largest commodity drives in 2026: ~4 TB, marketed as
/// external storage; sticks max at ~2 TB). Operators with
/// 4+ TiB USB drives can still flash via explicit
/// `/dev/sdX` (which bypasses auto-detect entirely).
const USB_FALLBACK_SIZE_CAP_BYTES: u64 = 2 * 1024 * 1024 * 1024 * 1024;

/// Scan sysfs for removable USB block devices suitable for flashing.
/// Returns them sorted by device name.
#[must_use]
pub fn list_removable_drives() -> Vec<Drive> {
    let mut drives = Vec::new();
    let Ok(entries) = fs::read_dir("/sys/block") else {
        return drives;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        // Only sd* devices (SCSI/USB mass storage).
        if !name_str.starts_with("sd") {
            continue;
        }
        let sysdir = entry.path();
        // Read model + size + removable + transport up-front so the
        // accept-decision can consider all four together (#661 fallback).
        let model = read_sysfs_str(&sysdir.join("device/model"))
            .unwrap_or_else(|| "(unknown model)".to_string());
        let size_bytes = read_sysfs_int_u64(&sysdir.join("size")).unwrap_or(0) * 512;
        let removable_flag = read_sysfs_int(&sysdir.join("removable"));
        let transport = classify_transport(&sysdir, &name_str);
        if !drive_passes_filter(removable_flag, transport, size_bytes) {
            continue;
        }
        // Count partitions (sdX1, sdX2, ...).
        let partitions = fs::read_dir(&sysdir).map_or(0, |iter| {
            iter.flatten()
                .filter(|e| {
                    e.file_name()
                        .to_string_lossy()
                        .starts_with(name_str.as_ref())
                })
                .count()
        });

        drives.push(Drive {
            dev: PathBuf::from(format!("/dev/{name_str}")),
            model: model.trim().to_string(),
            size_bytes,
            partitions,
        });
    }
    drives.sort_by(|a, b| a.dev.cmp(&b.dev));
    drives
}

/// Scan `/sys/block` for ALL block devices an operator might care about,
/// regardless of removable bit. Used by `doctor` to surface a full disk
/// inventory (#560).
///
/// Includes: `sd*` (SCSI/SATA/USB), `nvme*n*` (`NVMe` namespaces), `vd*`
/// (virtio), `mmcblk*` (SD/eMMC). Excludes: `loop*`, `ram*`, `dm-*`,
/// `sr*` (optical), `zram*` — none of which are persistent installable
/// targets and would just clutter the inventory row.
///
/// Best-effort: missing sysfs files surface as `Unknown` transport,
/// `(unknown model)` text, or `0` size. The doctor row is informational
/// only, never a Fail trigger.
#[must_use]
pub fn list_block_devices() -> Vec<BlockDevice> {
    let Ok(entries) = fs::read_dir("/sys/block") else {
        return Vec::new();
    };
    let mut devices: Vec<BlockDevice> = entries
        .flatten()
        .filter_map(|entry| build_block_device(&entry.path()))
        .collect();
    devices.sort_by(|a, b| a.dev.cmp(&b.dev));
    devices
}

fn build_block_device(sysdir: &Path) -> Option<BlockDevice> {
    let name_os = sysdir.file_name()?;
    let name = name_os.to_string_lossy().into_owned();
    if !is_inventoried_block_device(&name) {
        return None;
    }
    let model = read_sysfs_str(&sysdir.join("device/model"))
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "(unknown model)".to_string());
    let size_bytes = read_sysfs_int_u64(&sysdir.join("size")).unwrap_or(0) * 512;
    let removable = read_sysfs_int(&sysdir.join("removable")) == Some(1);
    let transport = classify_transport(sysdir, &name);
    Some(BlockDevice {
        dev: PathBuf::from(format!("/dev/{name}")),
        model,
        size_bytes,
        removable,
        transport,
    })
}

fn is_inventoried_block_device(name: &str) -> bool {
    // Whitelist persistent storage prefixes; drop pseudo / volatile / optical.
    name.starts_with("sd")
        || name.starts_with("nvme")
        || name.starts_with("vd")
        || name.starts_with("mmcblk")
        || name.starts_with("xvd")
}

fn classify_transport(sysdir: &Path, name: &str) -> BlockDeviceTransport {
    if name.starts_with("nvme") {
        return BlockDeviceTransport::Nvme;
    }
    if name.starts_with("vd") || name.starts_with("xvd") {
        return BlockDeviceTransport::Virtio;
    }
    if name.starts_with("mmcblk") {
        return BlockDeviceTransport::Mmc;
    }
    // For sd* the bus type is reported under device/../subsystem (a symlink
    // pointing into /sys/bus/{usb,scsi,ata,...}). Read the resolved target's
    // last component as the transport label.
    let subsystem = sysdir.join("device/subsystem");
    if let Ok(resolved) = fs::read_link(&subsystem)
        && let Some(last) = resolved.file_name()
    {
        return match last.to_string_lossy().as_ref() {
            "usb" => BlockDeviceTransport::Usb,
            "scsi" => BlockDeviceTransport::Scsi,
            "ata" | "sata" => BlockDeviceTransport::Sata,
            _ => BlockDeviceTransport::Unknown,
        };
    }
    BlockDeviceTransport::Unknown
}

/// Decide whether a `sd*` block device is acceptable as a flash
/// target given its kernel `removable` flag, bus transport, and
/// reported size in bytes.
///
/// Strict accept: `removable == 1` (the historic gate; matches
/// the SD-card / canonical USB-stick case).
///
/// USB-attached-SSD fallback (#661): when `removable == 0`,
/// accept iff transport is USB AND size is at or below
/// [`USB_FALLBACK_SIZE_CAP_BYTES`]. Drives with `removable == 0`
/// and a non-USB transport (SATA, SCSI, virtio, unknown) are
/// rejected — those are system disks. Drives with USB transport
/// but `size_bytes == 0` (sysfs read failed) are also rejected;
/// we'd rather miss the drive than risk a false-positive when
/// the size ceiling can't be applied.
fn drive_passes_filter(
    removable_flag: Option<i64>,
    transport: BlockDeviceTransport,
    size_bytes: u64,
) -> bool {
    if removable_flag == Some(1) {
        return true;
    }
    matches!(transport, BlockDeviceTransport::Usb)
        && size_bytes > 0
        && size_bytes <= USB_FALLBACK_SIZE_CAP_BYTES
}

fn read_sysfs_str(path: &Path) -> Option<String> {
    fs::read_to_string(path).ok()
}

fn read_sysfs_int(path: &Path) -> Option<i64> {
    read_sysfs_str(path)?.trim().parse().ok()
}

fn read_sysfs_int_u64(path: &Path) -> Option<u64> {
    read_sysfs_str(path)?.trim().parse().ok()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::missing_panics_doc)]
mod tests {
    use super::*;

    #[test]
    fn is_inventoried_keeps_persistent_prefixes() {
        for keep in ["sda", "sdb1", "nvme0n1", "vda", "xvdc", "mmcblk0"] {
            assert!(
                is_inventoried_block_device(keep),
                "expected {keep} to be inventoried"
            );
        }
    }

    // ---- #661: USB-fallback accept logic --------------------------

    const KIB: u64 = 1024;
    const MIB: u64 = 1024 * KIB;
    const GIB: u64 = 1024 * MIB;
    const TIB: u64 = 1024 * GIB;

    #[test]
    fn removable_flag_one_is_always_accepted() {
        // Historic case: SD card / canonical USB stick reports
        // removable=1. Transport doesn't matter; size doesn't matter.
        assert!(drive_passes_filter(
            Some(1),
            BlockDeviceTransport::Usb,
            8 * GIB
        ));
        assert!(drive_passes_filter(
            Some(1),
            BlockDeviceTransport::Sata,
            128 * GIB
        ));
        assert!(drive_passes_filter(
            Some(1),
            BlockDeviceTransport::Unknown,
            0
        ));
    }

    #[test]
    fn usb_with_removable_zero_under_cap_is_accepted() {
        // #661: 256 GB Kingston USB SSD reports removable=0.
        // Should be accepted via USB-fallback path.
        assert!(drive_passes_filter(
            Some(0),
            BlockDeviceTransport::Usb,
            256 * GIB
        ));
        // Edge case: exactly at the cap (2 TiB) is accepted.
        assert!(drive_passes_filter(
            Some(0),
            BlockDeviceTransport::Usb,
            USB_FALLBACK_SIZE_CAP_BYTES
        ));
    }

    #[test]
    fn usb_with_removable_zero_over_cap_is_rejected() {
        // 4 TiB external HDD on USB — most likely operator's backup
        // drive, not a flash target. Reject.
        assert!(!drive_passes_filter(
            Some(0),
            BlockDeviceTransport::Usb,
            4 * TIB
        ));
        // Just over the cap.
        assert!(!drive_passes_filter(
            Some(0),
            BlockDeviceTransport::Usb,
            USB_FALLBACK_SIZE_CAP_BYTES + 1
        ));
    }

    #[test]
    fn usb_with_zero_size_is_rejected() {
        // sysfs size read failed (size_bytes == 0). Reject under the
        // USB fallback path — without size we can't apply the ceiling.
        assert!(!drive_passes_filter(Some(0), BlockDeviceTransport::Usb, 0));
    }

    #[test]
    fn non_usb_with_removable_zero_is_rejected() {
        // SATA / SCSI / virtio / unknown transports with removable=0
        // are system disks. Reject regardless of size.
        for tran in [
            BlockDeviceTransport::Sata,
            BlockDeviceTransport::Scsi,
            BlockDeviceTransport::Virtio,
            BlockDeviceTransport::Unknown,
        ] {
            assert!(
                !drive_passes_filter(Some(0), tran, 8 * GIB),
                "{tran:?} with removable=0 must be rejected"
            );
        }
    }

    #[test]
    fn missing_removable_flag_is_rejected_for_non_usb() {
        // Some virtual / virtio devices have no removable file.
        // Treat as not removable.
        assert!(!drive_passes_filter(
            None,
            BlockDeviceTransport::Sata,
            8 * GIB
        ));
    }

    #[test]
    fn missing_removable_flag_falls_through_to_usb_fallback() {
        // If removable is unreadable but transport is USB and size
        // is sane, still accept under the fallback path.
        assert!(drive_passes_filter(
            None,
            BlockDeviceTransport::Usb,
            32 * GIB
        ));
    }

    #[test]
    fn is_inventoried_drops_pseudo_and_optical() {
        for drop in ["loop0", "ram0", "dm-0", "sr0", "zram0"] {
            assert!(
                !is_inventoried_block_device(drop),
                "expected {drop} to be excluded"
            );
        }
    }

    #[test]
    fn classify_transport_dispatches_on_name_prefix() {
        let tmp = tempfile::tempdir().unwrap();
        // Name-only dispatch covers nvme/virtio/mmc without needing sysfs
        // symlinks.
        assert_eq!(
            classify_transport(tmp.path(), "nvme0n1"),
            BlockDeviceTransport::Nvme
        );
        assert_eq!(
            classify_transport(tmp.path(), "vda"),
            BlockDeviceTransport::Virtio
        );
        assert_eq!(
            classify_transport(tmp.path(), "xvdc"),
            BlockDeviceTransport::Virtio
        );
        assert_eq!(
            classify_transport(tmp.path(), "mmcblk0"),
            BlockDeviceTransport::Mmc
        );
    }

    #[test]
    fn classify_transport_returns_unknown_for_sd_without_subsystem() {
        let tmp = tempfile::tempdir().unwrap();
        // No device/subsystem symlink → Unknown (graceful fallback).
        assert_eq!(
            classify_transport(tmp.path(), "sda"),
            BlockDeviceTransport::Unknown
        );
    }
}
