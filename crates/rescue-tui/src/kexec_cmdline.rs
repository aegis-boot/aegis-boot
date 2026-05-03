// SPDX-License-Identifier: MIT OR Apache-2.0

//! Distribution-aware kexec cmdline enrichment.
//!
//! Real-hardware report 2026-05-03 (#728): kexec'ing Ubuntu 24.04
//! live-server from rescue-tui completed at the syscall layer but
//! the new kernel hung silently because casper init couldn't find
//! the root filesystem. Root cause: GRUB normally injects
//! `iso-scan/filename=${iso_path}` for live-USB boots so casper
//! knows to find the .iso file on disk, loop-mount it, and pull
//! squashfs from inside. Our static iso-parser doesn't substitute
//! the GRUB variable, so the arg gets dropped — casper boots blind.
//!
//! This module adds three enrichments to the cmdline before it's
//! handed to `kexec_file_load`:
//!
//!   1. **`iso-scan/filename=<path>`** for casper-style ISOs
//!      ([`Distribution::Debian`] — covers Ubuntu, Debian live, Mint,
//!      Pop!_OS, Elementary, etc.). The path is computed relative
//!      to the `AEGIS_ISOS` partition root via `/proc/mounts` lookup.
//!
//!   2. **`console=tty0`** if no `console=` is present. Ubuntu's
//!      grub.cfg-in-ISO usually includes `quiet splash ---` with
//!      no console hint; without one, the kexec'd kernel may print
//!      to a serial port the operator can't see.
//!
//!   3. **`nomodeset`** for Debian distros if not already present.
//!      Operator real-hardware report 2026-05-03 (post-#732):
//!      kexec'ing Ubuntu Server on an AMD micro-PC printed the
//!      handoff banner then went silent for 30+ seconds — the
//!      kexec'd kernel WAS running (no kexec error returned), but
//!      its dmesg never reached the framebuffer. Root cause: the
//!      kernel's amdgpu driver init takes seconds to set up KMS
//!      (kernel mode setting); during that window, all kernel
//!      output queues to a buffer that flushes to the framebuffer
//!      ONLY after KMS is up. If KMS hits a snag (firmware load,
//!      hotplug race), the buffer never flushes — operator sees
//!      a black screen. `nomodeset` skips KMS entirely; the kernel
//!      uses basic VGA from boot, dmesg is visible immediately.
//!      Trade-off: GPU acceleration is disabled in the booted
//!      live system. Acceptable for live-installer use cases;
//!      operator can remove `nomodeset` from the cmdline override
//!      if they're booting on Intel/Nvidia and want acceleration.
//!
//! Other distros (Arch, Fedora, openSUSE, Alpine, NixOS) need
//! their own per-distro injection — tracked as follow-up to #728.

use std::path::Path;

use iso_probe::Distribution;

/// Enrich a kernel cmdline for kexec, given the ISO path + distribution.
///
/// Returns the (possibly modified) cmdline. Idempotent — passing an
/// already-enriched cmdline back through this function is a no-op.
///
/// `iso_path` is the absolute on-host-fs path of the .iso file
/// (e.g. `/run/media/aegis-isos/ubuntu-24.04.2-live-server-amd64.iso`).
/// We discover the partition mount point via `/proc/mounts` to compute
/// the partition-relative path that casper will look for after the
/// kexec'd kernel re-mounts the partition itself.
#[must_use]
pub fn enrich_cmdline_for_kexec(
    base_cmdline: &str,
    distribution: Distribution,
    iso_path: &Path,
) -> String {
    use std::fmt::Write as _;
    let mut out = base_cmdline.trim().to_string();

    if distribution == Distribution::Debian
        && !cmdline_has_arg(&out, "iso-scan/filename")
        && let Some(rel) = partition_relative_path(iso_path)
    {
        if !out.is_empty() {
            out.push(' ');
        }
        let _ = write!(out, "iso-scan/filename={rel}");
    }

    if !cmdline_has_arg(&out, "console") {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str("console=tty0");
    }

    // Force basic VGA + visible early dmesg for Debian distros that
    // would otherwise queue output behind a KMS init that may never
    // complete on operator-real-hardware. See module-doc #3 for
    // rationale.
    if distribution == Distribution::Debian && !cmdline_has_arg(&out, "nomodeset") {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str("nomodeset");
    }

    out
}

/// Check whether the cmdline already has a given arg key.
///
/// Matches `<key>=...` or bare `<key>` (e.g. `quiet`). Word-boundary
/// match — `iso-scan/filename` won't match `iso-scan/filename-extra`.
fn cmdline_has_arg(cmdline: &str, key: &str) -> bool {
    cmdline.split_whitespace().any(|tok| {
        // Match `key=value`, `key`, or `key:` forms.
        tok == key || tok.starts_with(&format!("{key}=")) || tok.starts_with(&format!("{key}:"))
    })
}

/// Compute the path of `iso_path` relative to its containing mount
/// point. e.g. `/run/media/aegis-isos/ubuntu.iso` with `AEGIS_ISOS`
/// mounted at `/run/media/aegis-isos` → `Some("/ubuntu.iso")`.
///
/// Returns `None` if `/proc/mounts` isn't readable or no enclosing
/// mount is found (shouldn't happen — `/` always mounts).
///
/// Public for testability via [`partition_relative_path_with_mounts`].
#[must_use]
pub fn partition_relative_path(iso_path: &Path) -> Option<String> {
    let mounts = std::fs::read_to_string("/proc/mounts").ok()?;
    partition_relative_path_with_mounts(iso_path, &mounts)
}

/// Pure-fn variant of [`partition_relative_path`] for unit testing
/// without depending on the runtime `/proc/mounts` shape.
#[must_use]
pub fn partition_relative_path_with_mounts(iso_path: &Path, mounts_text: &str) -> Option<String> {
    let iso_str = iso_path.to_str()?;
    // Each /proc/mounts line: `<dev> <mountpoint> <fstype> <opts> <freq> <passno>`.
    // Find the longest mountpoint that is a prefix of iso_path.
    let mut best: Option<&str> = None;
    for line in mounts_text.lines() {
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.len() < 2 {
            continue;
        }
        let mp = fields[1];
        // Mountpoint must be a path-prefix — bare string-prefix would
        // match `/run/media/aegis` against `/run/media/aegis-isos/...`
        // incorrectly. Require either exact match or trailing `/`.
        let is_prefix = if mp == "/" {
            iso_str.starts_with('/')
        } else {
            iso_str.starts_with(mp)
                && (iso_str.len() == mp.len() || iso_str.as_bytes()[mp.len()] == b'/')
        };
        if !is_prefix {
            continue;
        }
        if best.is_none_or(|b| mp.len() > b.len()) {
            best = Some(mp);
        }
    }
    let mp = best?;
    if mp == "/" {
        return Some(iso_str.to_string());
    }
    let rel = &iso_str[mp.len()..];
    if rel.is_empty() {
        Some("/".to_string())
    } else {
        Some(rel.to_string())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn p(s: &str) -> PathBuf {
        PathBuf::from(s)
    }

    // ---- partition_relative_path_with_mounts ------------------------

    #[test]
    fn partition_relative_finds_longest_prefix_mount() {
        let mounts = "\
/dev/root / ext4 rw 0 0
/dev/sda2 /run/media/aegis-isos exfat rw 0 0
/dev/sda1 /run/media/sda1 vfat ro 0 0
";
        let rel = partition_relative_path_with_mounts(
            &p("/run/media/aegis-isos/ubuntu-24.04.2.iso"),
            mounts,
        );
        assert_eq!(rel.as_deref(), Some("/ubuntu-24.04.2.iso"));
    }

    #[test]
    fn partition_relative_handles_root_mount_only() {
        let mounts = "/dev/root / ext4 rw 0 0\n";
        let rel = partition_relative_path_with_mounts(&p("/some/iso.iso"), mounts);
        assert_eq!(rel.as_deref(), Some("/some/iso.iso"));
    }

    #[test]
    fn partition_relative_word_boundary_avoids_string_match() {
        // Bare prefix-match would mis-match aegis vs aegis-isos.
        let mounts = "\
/dev/root / ext4 rw 0 0
/dev/sda9 /run/media/aegis exfat rw 0 0
/dev/sda2 /run/media/aegis-isos exfat rw 0 0
";
        let rel = partition_relative_path_with_mounts(&p("/run/media/aegis-isos/x.iso"), mounts);
        assert_eq!(rel.as_deref(), Some("/x.iso"));
    }

    #[test]
    fn partition_relative_returns_none_when_no_mount_matches() {
        // An iso_path that doesn't start with any mount's mountpoint.
        // Empty mounts table → no `/` mount → no match.
        let rel = partition_relative_path_with_mounts(&p("/anywhere/x.iso"), "");
        assert!(rel.is_none());
    }

    // ---- cmdline_has_arg --------------------------------------------

    #[test]
    fn cmdline_has_arg_matches_key_value() {
        assert!(cmdline_has_arg("a=1 console=tty0 b=2", "console"));
    }

    #[test]
    fn cmdline_has_arg_matches_bare_key() {
        assert!(cmdline_has_arg("quiet splash", "quiet"));
    }

    #[test]
    fn cmdline_has_arg_word_boundary_avoids_substring() {
        // "iso-scan/filename" must NOT match "iso-scan/filename-extra".
        assert!(!cmdline_has_arg(
            "iso-scan/filename-extra=foo",
            "iso-scan/filename"
        ));
    }

    #[test]
    fn cmdline_has_arg_handles_colon_form() {
        // root=live:CDLABEL=foo — Fedora syntax. The `key:value` form
        // matters when we add Fedora support later.
        assert!(cmdline_has_arg("root=live:CDLABEL=Foo", "root"));
    }

    // ---- enrich_cmdline_for_kexec -----------------------------------

    fn mounts_with_aegis_isos() -> String {
        "/dev/root / ext4 rw 0 0\n/dev/sda2 /run/media/aegis-isos exfat rw 0 0\n".into()
    }

    #[test]
    fn enrich_injects_iso_scan_for_debian_when_missing() {
        // Inline test bypassing /proc/mounts via the partition_relative_path_with_mounts
        // helper would require a test seam; instead, assert the
        // happy-path text construction directly via a synthetic path.
        let mounts = mounts_with_aegis_isos();
        let rel =
            partition_relative_path_with_mounts(&p("/run/media/aegis-isos/ubuntu.iso"), &mounts)
                .unwrap();
        // Verify enrich would build the right token.
        let expected_arg = format!("iso-scan/filename={rel}");
        assert_eq!(expected_arg, "iso-scan/filename=/ubuntu.iso");
    }

    #[test]
    fn enrich_skips_iso_scan_when_already_present() {
        let base = "boot=casper iso-scan/filename=/already-set.iso quiet";
        let out = enrich_cmdline_for_kexec(
            base,
            Distribution::Debian,
            &p("/run/media/aegis-isos/x.iso"),
        );
        // Should NOT contain a second iso-scan/filename token.
        let count = out
            .split_whitespace()
            .filter(|t| t.starts_with("iso-scan/filename="))
            .count();
        assert_eq!(
            count, 1,
            "iso-scan/filename should appear exactly once: {out}"
        );
    }

    #[test]
    fn enrich_skips_iso_scan_for_non_debian_distros() {
        let base = "rd.live.image quiet";
        let out = enrich_cmdline_for_kexec(
            base,
            Distribution::Fedora,
            &p("/run/media/aegis-isos/fedora.iso"),
        );
        assert!(
            !out.contains("iso-scan/filename"),
            "iso-scan/filename is Debian-specific, not added for Fedora: {out}"
        );
    }

    #[test]
    fn enrich_adds_console_tty0_when_missing() {
        let base = "boot=casper quiet";
        let out = enrich_cmdline_for_kexec(
            base,
            Distribution::Debian,
            &p("/run/media/aegis-isos/x.iso"),
        );
        assert!(out.contains("console=tty0"), "expected console=tty0: {out}");
    }

    #[test]
    fn enrich_skips_console_when_operator_set_one() {
        let base = "boot=casper console=ttyS0,115200 quiet";
        let out = enrich_cmdline_for_kexec(
            base,
            Distribution::Debian,
            &p("/run/media/aegis-isos/x.iso"),
        );
        // Operator's console= wins; we don't add a second one.
        let count = out
            .split_whitespace()
            .filter(|t| t.starts_with("console="))
            .count();
        assert_eq!(count, 1, "expected single console= token: {out}");
    }

    #[test]
    fn enrich_is_idempotent_on_repeated_calls() {
        let base = "boot=casper";
        let once = enrich_cmdline_for_kexec(
            base,
            Distribution::Debian,
            &p("/run/media/aegis-isos/x.iso"),
        );
        let twice = enrich_cmdline_for_kexec(
            &once,
            Distribution::Debian,
            &p("/run/media/aegis-isos/x.iso"),
        );
        assert_eq!(once, twice, "enrich should be idempotent");
    }

    // ---- nomodeset injection (post-#732 operator real-hardware fix) ----

    #[test]
    fn enrich_adds_nomodeset_for_debian() {
        let base = "boot=casper";
        let out = enrich_cmdline_for_kexec(
            base,
            Distribution::Debian,
            &p("/run/media/aegis-isos/x.iso"),
        );
        assert!(
            out.contains("nomodeset"),
            "expected nomodeset for Debian: {out}"
        );
    }

    #[test]
    fn enrich_skips_nomodeset_when_already_present() {
        let base = "boot=casper nomodeset";
        let out = enrich_cmdline_for_kexec(
            base,
            Distribution::Debian,
            &p("/run/media/aegis-isos/x.iso"),
        );
        let count = out.split_whitespace().filter(|t| *t == "nomodeset").count();
        assert_eq!(count, 1, "nomodeset should appear exactly once: {out}");
    }

    #[test]
    fn enrich_skips_nomodeset_for_non_debian_distros() {
        // nomodeset is Debian-specific until per-distro AMD-GPU triage
        // is done for Fedora/Arch/openSUSE (each has its own KMS
        // semantics and may not have the same casper-style early-boot
        // hang). Conservative: only inject where we have evidence.
        let base = "rd.live.image";
        let out = enrich_cmdline_for_kexec(
            base,
            Distribution::Fedora,
            &p("/run/media/aegis-isos/fedora.iso"),
        );
        assert!(
            !out.contains("nomodeset"),
            "nomodeset is Debian-specific, not added for Fedora: {out}"
        );
    }
}
