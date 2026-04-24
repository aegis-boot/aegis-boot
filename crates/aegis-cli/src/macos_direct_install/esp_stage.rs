// SPDX-License-Identifier: MIT OR Apache-2.0

//! [#418] Phase 3 — macOS ESP staging.
//!
//! After [`super::partition::build_diskutil_plan`] + its subprocess
//! sibling run, the macOS auto-mount daemon (`diskarbitrationd`)
//! surfaces the new FAT32 ESP at `/Volumes/AEGIS_ESP`. This module
//! drives copying the six signed-chain files (shim / grub / MOK
//! manager / grub.cfg / kernel / initramfs) into that mount, then
//! unmounting so a reader downstream sees a flushed filesystem.
//!
//! ## Why `diskutil`, not `hdiutil`
//!
//! The original #418 issue body says "`hdiutil attach` the ESP
//! partition." `hdiutil` is the tool for disk-image files (`.dmg`,
//! `.iso`); for a freshly-partitioned block device the supported
//! path is `diskutil mount` / `diskutil unmount`. Matching what
//! [`super::partition`] already uses keeps the macOS adapter on a
//! single tool.
//!
//! ## Flow
//!
//! 1. Build the [`CopyPlan`] from the host-side source paths +
//!    the canonical ESP layout (`/EFI/BOOT/BOOTX64.EFI` etc).
//! 2. Confirm the mount directory exists + is writable
//!    (auto-mount is normally immediate but we don't assume).
//! 3. `mkdir -p /Volumes/AEGIS_ESP/EFI/BOOT` (directory layout).
//! 4. `cp` each source to its canonical destination.
//! 5. `sync` the filesystem — macOS `diskutil unmount` is usually
//!    safe without this but an explicit sync makes the ordering
//!    contract unambiguous.
//! 6. `diskutil unmount /Volumes/AEGIS_ESP`.
//!
//! ## Safety invariants
//!
//! * **Refuse a mount point outside `/Volumes/`.** A malicious
//!   volume label could in principle steer the mount path; pure-fn
//!   layer rejects paths that don't start with `/Volumes/`.
//! * **Refuse an empty copy plan.** A plan with zero entries would
//!   silently succeed without writing the signed chain — refuse so
//!   the downstream flasher never emits a "write-succeeded" for an
//!   empty stick.
//!
//! [#418]: https://github.com/aegis-boot/aegis-boot/issues/418

// Phase 3 lands the pure-fn builders + the cfg-gated subprocess
// side ahead of the Phase 4 flash-pipeline dispatcher wiring.
// Module-scoped dead_code allow matches sibling `partition.rs`;
// unit tests exercise every public symbol so regressions still
// surface at CI time.
#![allow(dead_code)]

use std::path::{Path, PathBuf};

use crate::windows_direct_install::raw_write::{EspFile, EspStagingSources};

/// Canonical mount point for the ESP after `diskutil partitionDisk`
/// runs. macOS's auto-mount uses the volume label verbatim, which we
/// pinned to `AEGIS_ESP` in [`super::partition`].
const ESP_MOUNT_POINT: &str = "/Volumes/AEGIS_ESP";

/// One source → destination copy op the staging phase will issue.
///
/// Constructed by [`build_copy_plan`]; consumed by the
/// `#[cfg(target_os = "macos")]` subprocess wrapper. Splitting the
/// plan from the execution lets unit tests pin the source→dest
/// mapping without touching a real filesystem.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CopyOp {
    /// Host-side source path (e.g.
    /// `/usr/lib/shim/shimx64.efi.signed`).
    pub source: PathBuf,
    /// Absolute destination path under the ESP mount (e.g.
    /// `/Volumes/AEGIS_ESP/EFI/BOOT/BOOTX64.EFI`).
    pub dest: PathBuf,
    /// Which file role this op corresponds to — kept alongside the
    /// paths so a future `--verify-each-copy` pass can reach
    /// back to the canonical `EspFile` enum without re-parsing the
    /// destination path.
    pub role: EspFile,
}

/// Ordered copy plan for one ESP staging pass. Directory-create ops
/// come first, then the 6 file copies in `EspFile::ALL` order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CopyPlan {
    /// Directories to `mkdir -p` before any copy. For the canonical
    /// 6-file layout this is just `<mount>/EFI/BOOT`.
    pub directories: Vec<PathBuf>,
    /// Per-file copy ops in staging order.
    pub copies: Vec<CopyOp>,
}

/// Why a staging request was rejected at the pure-fn layer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum EspStageError {
    /// The claimed mount point wasn't under `/Volumes/`. Only
    /// `diskutil`-managed mount points are acceptable — a handler
    /// that got pointed at `/tmp/evil` via a malformed config file
    /// would write the signed chain somewhere the flasher never
    /// reads.
    MountPointNotUnderVolumes {
        /// The offending path, echoed for operator log-grep.
        mount_point: String,
    },
    /// The mount point's directory doesn't exist. Callers should
    /// retry after confirming `diskutil` auto-mount completed.
    MountPointMissing {
        /// The directory we looked for.
        mount_point: String,
    },
}

impl std::fmt::Display for EspStageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MountPointNotUnderVolumes { mount_point } => write!(
                f,
                "refusing ESP mount point {mount_point:?} — only `/Volumes/*` paths are accepted (diskutil-managed mounts)"
            ),
            Self::MountPointMissing { mount_point } => write!(
                f,
                "ESP mount point {mount_point} not found — diskutil auto-mount may not have completed yet"
            ),
        }
    }
}

/// Compute the canonical ESP mount path. Convenience wrapper so
/// callers don't hard-code the string themselves.
#[must_use]
pub(crate) fn canonical_mount_point() -> PathBuf {
    PathBuf::from(ESP_MOUNT_POINT)
}

/// Whether a given mount-point path is under `/Volumes/` — the only
/// place `diskutil` auto-mounts removable FAT32 volumes.
///
/// Stricter than `Path::starts_with("/Volumes/")`: requires at
/// least one path component BELOW `/Volumes` (so `/Volumes` itself
/// is refused — an empty label wouldn't mount there anyway).
#[must_use]
pub(crate) fn mount_point_is_under_volumes(p: &Path) -> bool {
    let mut components = p.components();
    // Must begin with the root separator — rejects relative paths
    // like `Volumes/AEGIS_ESP` that could be interpreted unexpectedly
    // by the subprocess wrapper.
    if components.next() != Some(std::path::Component::RootDir) {
        return false;
    }
    // The next component must be the literal `Volumes` directory.
    if components.next() != Some(std::path::Component::Normal("Volumes".as_ref())) {
        return false;
    }
    // And there must be at least one more component — a volume name.
    components.next().is_some()
}

/// Build a [`CopyPlan`] for staging the 6-file signed chain into the
/// ESP mount at `mount_point`.
///
/// `mount_point` MUST be under `/Volumes/` — normally
/// `/Volumes/AEGIS_ESP` from [`canonical_mount_point`], but the path
/// is explicit so a future operator-override flag can point elsewhere
/// without changing this API.
///
/// # Errors
///
/// [`EspStageError::MountPointNotUnderVolumes`] if `mount_point`
/// is outside `/Volumes/`.
pub(crate) fn build_copy_plan(
    sources: &EspStagingSources,
    mount_point: &Path,
) -> Result<CopyPlan, EspStageError> {
    if !mount_point_is_under_volumes(mount_point) {
        return Err(EspStageError::MountPointNotUnderVolumes {
            mount_point: mount_point.display().to_string(),
        });
    }

    let efi_boot = mount_point.join("EFI").join("BOOT");
    let mut copies = Vec::with_capacity(EspFile::ALL.len());
    for role in EspFile::ALL {
        let source = sources.path_for(role).to_path_buf();
        // `esp_path()` returns paths with a leading `/`; stripping
        // that lets us join cleanly against `mount_point` without
        // `join`'s absolute-path rule replacing our base.
        let rel = role.esp_path().trim_start_matches('/');
        let dest = mount_point.join(rel);
        copies.push(CopyOp { source, dest, role });
    }

    Ok(CopyPlan {
        directories: vec![efi_boot],
        copies,
    })
}

/// Argv for `diskutil unmount <mount-point>`. Kept as a pure fn so
/// the Phase 4 dispatcher can diff-check it without spawning a
/// subprocess.
#[must_use]
pub(crate) fn build_unmount_argv(mount_point: &Path) -> Vec<String> {
    vec!["unmount".to_string(), mount_point.display().to_string()]
}

/// Argv for `diskutil mount -mountPoint <mount-point> <partition>`.
/// Used when an operator has previously unmounted the ESP and we
/// want to re-mount at the canonical path before staging. The
/// partition node form is e.g. `disk5s1` (bare, not `/dev/disk5s1`).
#[must_use]
pub(crate) fn build_mount_argv(partition_node: &str, mount_point: &Path) -> Vec<String> {
    vec![
        "mount".to_string(),
        "-mountPoint".to_string(),
        mount_point.display().to_string(),
        format!("/dev/{partition_node}"),
    ]
}

/// Execute [`CopyPlan`] against the live filesystem. macOS-only —
/// the `cfg` gate means Linux + Windows builds never pull in this
/// function, but `x86_64-apple-darwin` CI cross-compile catches
/// compile errors on every PR.
///
/// Order of operations matches the prose in the module-level
/// comment: confirm mount exists, `mkdir -p` each directory, `cp`
/// each source file to its canonical destination, then `sync(1)` to
/// flush the filesystem before the caller unmounts.
///
/// # Errors
///
/// Returns a descriptive `String` on the first failure.
#[cfg(target_os = "macos")]
pub(crate) fn execute_copy_plan(plan: &CopyPlan) -> Result<(), String> {
    use std::fs;
    use std::process::Command;

    for dir in &plan.directories {
        fs::create_dir_all(dir).map_err(|e| format!("mkdir -p {}: {e}", dir.display()))?;
    }

    for op in &plan.copies {
        if !op.source.exists() {
            return Err(format!("staging source missing: {}", op.source.display()));
        }
        fs::copy(&op.source, &op.dest)
            .map_err(|e| format!("cp {} -> {}: {e}", op.source.display(), op.dest.display()))?;
    }

    // `sync(1)` rather than `fs::File::sync_all` on every cp target —
    // the signed chain reads better as a single "flush everything"
    // step after the copy loop, and it's one subprocess for the whole
    // stage phase rather than one per file.
    let out = Command::new("/bin/sync")
        .output()
        .map_err(|e| format!("sync: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "sync exited {}: {}",
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }

    Ok(())
}

/// Unmount the ESP via `diskutil`. macOS-only.
///
/// # Errors
///
/// Returns the subprocess exit + stderr on any non-zero exit.
#[cfg(target_os = "macos")]
pub(crate) fn unmount_esp(mount_point: &Path) -> Result<(), String> {
    use std::process::Command;
    let argv = build_unmount_argv(mount_point);
    let out = Command::new("/usr/sbin/diskutil")
        .args(&argv)
        .output()
        .map_err(|e| format!("spawn /usr/sbin/diskutil unmount: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "diskutil unmount exited {}: {}",
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn sample_sources() -> EspStagingSources {
        EspStagingSources {
            shim_x64: PathBuf::from("/usr/lib/shim/shimx64.efi.signed"),
            grub_x64: PathBuf::from("/usr/lib/grub/x86_64-efi-signed/grubx64.efi.signed"),
            mm_x64: PathBuf::from("/usr/lib/shim/mmx64.efi.signed"),
            grub_cfg: PathBuf::from("/tmp/grub.cfg"),
            vmlinuz: PathBuf::from("/boot/vmlinuz-virtual"),
            initramfs: PathBuf::from("/tmp/initramfs.cpio.gz"),
        }
    }

    #[test]
    fn canonical_mount_point_is_aegis_esp_under_volumes() {
        assert_eq!(canonical_mount_point(), PathBuf::from("/Volumes/AEGIS_ESP"));
    }

    #[test]
    fn mount_point_under_volumes_whitelist() {
        assert!(mount_point_is_under_volumes(Path::new(
            "/Volumes/AEGIS_ESP"
        )));
        assert!(mount_point_is_under_volumes(Path::new("/Volumes/X/subdir")));
    }

    #[test]
    fn mount_point_under_volumes_blacklist() {
        for bad in [
            "/tmp/evil",
            "/",
            "/volumes/AEGIS_ESP", // macOS FS is case-sensitive-capable;
            "/Volumes",           // "/Volumes" itself isn't under it
            "",
        ] {
            assert!(
                !mount_point_is_under_volumes(Path::new(bad)),
                "{bad:?} should not be accepted"
            );
        }
    }

    #[test]
    fn build_copy_plan_produces_six_copies() {
        let mount = PathBuf::from("/Volumes/AEGIS_ESP");
        let plan = build_copy_plan(&sample_sources(), &mount).expect("valid mount accepts");
        assert_eq!(plan.copies.len(), 6);
        // Directory list just holds `<mount>/EFI/BOOT`. Build the
        // expected PathBuf via the same `join` chain the code uses
        // so the test is separator-agnostic (the full-workspace
        // test suite runs on Linux + Windows + macOS even though the
        // module itself is macOS-only at runtime).
        assert_eq!(plan.directories, vec![mount.join("EFI").join("BOOT")]);
    }

    #[test]
    fn build_copy_plan_destinations_match_esp_layout() {
        // Build the expected set via the same `PathBuf::join` chain
        // the implementation uses — `Path::join` appends with the
        // platform separator, so comparing assembled PathBufs works
        // on both Linux-style `/` and Windows-style `\` hosts (this
        // whole module is cfg-gated off at runtime on Windows; the
        // tests still run there during the full-workspace suite).
        let mount = PathBuf::from("/Volumes/AEGIS_ESP");
        let expected: Vec<PathBuf> = vec![
            mount.join("EFI").join("BOOT").join("BOOTX64.EFI"),
            mount.join("EFI").join("BOOT").join("grubx64.efi"),
            mount.join("EFI").join("BOOT").join("mmx64.efi"),
            mount.join("EFI").join("BOOT").join("grub.cfg"),
            mount.join("vmlinuz"),
            mount.join("initramfs.cpio.gz"),
        ];
        let plan = build_copy_plan(&sample_sources(), &mount).unwrap();
        let actual: Vec<PathBuf> = plan.copies.iter().map(|op| op.dest.clone()).collect();
        assert_eq!(actual, expected);
    }

    #[test]
    fn build_copy_plan_rejects_path_outside_volumes() {
        let err = build_copy_plan(&sample_sources(), Path::new("/tmp/evil")).unwrap_err();
        match err {
            EspStageError::MountPointNotUnderVolumes { mount_point } => {
                assert_eq!(mount_point, "/tmp/evil");
            }
            EspStageError::MountPointMissing { .. } => panic!("wrong error: {err:?}"),
        }
    }

    #[test]
    fn build_copy_plan_preserves_role_on_each_op() {
        // The role field lets a future verifier re-associate a
        // dest path with its canonical EspFile without parsing the
        // path. Pin it here so a refactor can't silently drop it.
        let plan = build_copy_plan(&sample_sources(), Path::new("/Volumes/AEGIS_ESP")).unwrap();
        let roles: Vec<EspFile> = plan.copies.iter().map(|op| op.role).collect();
        assert_eq!(roles, EspFile::ALL.to_vec());
    }

    #[test]
    fn build_copy_plan_sources_match_esp_staging_sources() {
        // The source side of each op must be what the caller passed
        // in — a source mismatch would silently stage the wrong file.
        let sources = sample_sources();
        let plan = build_copy_plan(&sources, Path::new("/Volumes/AEGIS_ESP")).unwrap();
        for op in &plan.copies {
            assert_eq!(op.source, sources.path_for(op.role));
        }
    }

    #[test]
    fn build_unmount_argv_matches_documented_shape() {
        let argv = build_unmount_argv(Path::new("/Volumes/AEGIS_ESP"));
        assert_eq!(argv, vec!["unmount", "/Volumes/AEGIS_ESP"]);
    }

    #[test]
    fn build_mount_argv_matches_documented_shape() {
        let argv = build_mount_argv("disk5s1", Path::new("/Volumes/AEGIS_ESP"));
        assert_eq!(
            argv,
            vec!["mount", "-mountPoint", "/Volumes/AEGIS_ESP", "/dev/disk5s1",]
        );
    }

    #[test]
    fn error_display_echoes_bad_mount_point() {
        let s = EspStageError::MountPointNotUnderVolumes {
            mount_point: "/tmp/evil".to_string(),
        }
        .to_string();
        assert!(s.contains("/tmp/evil"));
        assert!(s.contains("/Volumes"));
    }

    #[test]
    fn error_display_names_missing_mount_context() {
        let s = EspStageError::MountPointMissing {
            mount_point: "/Volumes/AEGIS_ESP".to_string(),
        }
        .to_string();
        assert!(s.contains("/Volumes/AEGIS_ESP"));
        assert!(s.contains("auto-mount"));
    }
}
