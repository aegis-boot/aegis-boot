// SPDX-License-Identifier: MIT OR Apache-2.0

//! Tier B failure log (#347 Phase 3b).
//!
//! Writes a structured JSONL record of every parse-failed ISO
//! (`iso-probe`'s `DiscoveryReport::failed`) to the `AEGIS_ISOS`
//! partition at rescue-tui startup. Operators pulling the stick
//! post-rescue can read this file from any host machine to troubleshoot
//! why specific ISOs didn't surface in the boot menu, without needing
//! to capture journald output during the rescue session.
//!
//! Parallel pattern to the verify-now audit log shipped in #548:
//! - One JSON object per line, one entry per failed ISO.
//! - Filename `aegis-boot-failures-<unix-ts>.jsonl` so multiple boots
//!   of the same stick produce comparable, non-clobbering files.
//! - Best-effort: write failures are logged via `tracing::warn!` but
//!   never block rescue-tui startup. The same UX principle as #602's
//!   audit-warning banner — degraded logging signal is surfaced, not
//!   pretended-away.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use iso_probe::{FailedIso, FailureKind};
use serde::Serialize;

/// One entry in the Tier B failure log. JSON-serialized one-per-line.
#[derive(Debug, Serialize)]
struct TierBEntry<'a> {
    /// Schema version. Bump only when adding/removing required fields.
    schema_version: u8,
    /// UNIX timestamp (seconds) the entry was written. Same value
    /// across all entries from the same rescue-tui run.
    timestamp: u64,
    /// Absolute path of the failed ISO on the `AEGIS_ISOS` partition.
    iso_path: String,
    /// Sanitized human-readable reason from iso-parser.
    reason: &'a str,
    /// Structured failure classification (`io_error`, `mount_failed`,
    /// `no_boot_entries`). Stable wire identifier; kebab-case to match
    /// the rest of the JSONL schemas.
    kind: &'static str,
}

const SCHEMA_VERSION: u8 = 1;

fn kind_label(k: FailureKind) -> &'static str {
    match k {
        FailureKind::IoError => "io_error",
        FailureKind::MountFailed => "mount_failed",
        FailureKind::NoBootEntries => "no_boot_entries",
    }
}

/// Write a Tier B failure log to `log_dir` listing every parse-failed
/// ISO. Returns the path of the written file. No-op (returns `Ok(None)`)
/// if `failed` is empty — operators with a clean stick don't get an
/// empty file cluttering `AEGIS_ISOS`.
///
/// # Errors
///
/// Returns `std::io::Error` if the file cannot be created or written.
/// `serde_json` serialization errors are converted to `io::Error` —
/// they're not actually possible for the simple `TierBEntry` shape but
/// the conversion keeps the signature uniform for callers.
pub fn write_failure_log(failed: &[FailedIso], log_dir: &Path) -> std::io::Result<Option<PathBuf>> {
    if failed.is_empty() {
        return Ok(None);
    }
    // SystemTime before UNIX_EPOCH means a clock so broken we
    // can't produce a usable filename. Fall back to 0 rather
    // than refuse to log; the file timestamp on disk preserves
    // the ordering anyway.
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let path = log_dir.join(format!("aegis-boot-failures-{timestamp}.jsonl"));
    std::fs::create_dir_all(log_dir)?;
    let mut file = std::fs::File::create(&path)?;
    for f in failed {
        let entry = TierBEntry {
            schema_version: SCHEMA_VERSION,
            timestamp,
            iso_path: f.iso_path.display().to_string(),
            reason: &f.reason,
            kind: kind_label(f.kind),
        };
        let line = serde_json::to_string(&entry).map_err(std::io::Error::other)?;
        writeln!(file, "{line}")?;
    }
    file.flush()?;
    Ok(Some(path))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use tempfile::tempdir;

    fn fake_failed(iso: &str, reason: &str, kind: FailureKind) -> FailedIso {
        FailedIso {
            iso_path: PathBuf::from(iso),
            reason: reason.to_string(),
            kind,
        }
    }

    #[test]
    fn empty_failed_list_writes_no_file() {
        let dir = tempdir().unwrap();
        let res = write_failure_log(&[], dir.path()).unwrap();
        assert!(res.is_none(), "no log file expected for empty failed list");
        // Directory should also remain empty.
        let entries: Vec<_> = std::fs::read_dir(dir.path()).unwrap().collect();
        assert!(
            entries.is_empty(),
            "log_dir should not have been created/populated"
        );
    }

    #[test]
    fn writes_one_jsonl_line_per_failure() {
        let dir = tempdir().unwrap();
        let failed = vec![
            fake_failed(
                "/run/aegis-isos/a.iso",
                "mount: wrong fs type",
                FailureKind::MountFailed,
            ),
            fake_failed(
                "/run/aegis-isos/b.iso",
                "no kernel found",
                FailureKind::NoBootEntries,
            ),
            fake_failed(
                "/run/aegis-isos/c.iso",
                "EIO reading sector 1",
                FailureKind::IoError,
            ),
        ];
        let path = write_failure_log(&failed, dir.path()).unwrap().unwrap();
        let body = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = body.lines().collect();
        assert_eq!(lines.len(), 3, "one JSONL line per failure");

        // Each line parses as JSON and has the expected fields.
        for (i, line) in lines.iter().enumerate() {
            let v: serde_json::Value = serde_json::from_str(line)
                .unwrap_or_else(|e| panic!("line {i} not valid JSON: {e} -- {line}"));
            assert_eq!(v["schema_version"], 1);
            assert!(v["timestamp"].is_u64());
            assert!(v["iso_path"].is_string());
            assert!(v["reason"].is_string());
            assert!(v["kind"].is_string());
        }
    }

    #[test]
    fn kind_labels_are_stable_kebab_case() {
        // These strings are wire-identifiers — changing them breaks
        // any downstream tooling that greps the log. Pin them here.
        assert_eq!(kind_label(FailureKind::IoError), "io_error");
        assert_eq!(kind_label(FailureKind::MountFailed), "mount_failed");
        assert_eq!(kind_label(FailureKind::NoBootEntries), "no_boot_entries");
    }

    #[test]
    fn filename_includes_unix_timestamp() {
        let dir = tempdir().unwrap();
        let failed = vec![fake_failed("/x.iso", "r", FailureKind::IoError)];
        let path = write_failure_log(&failed, dir.path()).unwrap().unwrap();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        assert!(
            name.starts_with("aegis-boot-failures-"),
            "filename prefix unexpected: {name}"
        );
        // Use Path::extension() to satisfy clippy's
        // case_sensitive_file_extension_comparisons lint without an
        // allow attr. We control the filename so we know the case
        // anyway, but the lint is right that the idiomatic check goes
        // through Path.
        assert_eq!(
            path.extension().and_then(|e| e.to_str()),
            Some("jsonl"),
            "filename extension unexpected: {name}"
        );
        // Middle is the unix timestamp; should be all digits.
        let middle = name
            .strip_prefix("aegis-boot-failures-")
            .unwrap()
            .strip_suffix(".jsonl")
            .unwrap();
        assert!(
            middle.chars().all(|c| c.is_ascii_digit()),
            "expected unix-timestamp middle, got {middle:?}"
        );
    }

    #[test]
    fn each_entry_carries_the_reason_verbatim() {
        let dir = tempdir().unwrap();
        let weird = "mount: wrong fs type, bad option, bad superblock";
        let failed = vec![fake_failed("/x.iso", weird, FailureKind::MountFailed)];
        let path = write_failure_log(&failed, dir.path()).unwrap().unwrap();
        let body = std::fs::read_to_string(&path).unwrap();
        let v: serde_json::Value = serde_json::from_str(body.trim()).unwrap();
        assert_eq!(v["reason"], weird);
    }
}
