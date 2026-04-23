// SPDX-License-Identifier: MIT OR Apache-2.0

//! `aegis-boot verify --stick <device>` — manifest-drift integrity check.
//!
//! Answers the question: "has my stick silently diverged from the bytes
//! we signed into its attestation manifest?" Complements
//! `aegis-boot update` which answers "does my stick need new bits?"
//!
//! # Semantics
//!
//! For each [`EspFileEntry`] in the host-side attestation manifest
//! (written at flash time by `flash::write_manifest_stage` or the
//! direct-install path post-#429):
//!
//!   1. Read the corresponding path from the stick's ESP via `mtype`
//!   2. sha256 the bytes
//!   3. Compare to the manifest-recorded hash
//!
//! Report per-file verdict: `OK` (hash matches) / `DRIFT` (hash
//! mismatch — tampering, corruption, manual edit) / `UNREADABLE`
//! (can't read the file from the stick).
//!
//! # Exit codes
//!
//! - `0` — every ESP file matches its recorded hash
//! - `1` — at least one `DRIFT` or `UNREADABLE` verdict
//! - `2` — couldn't resolve the device or locate the attestation
//!
//! Tracked in [#432](https://github.com/aegis-boot/aegis-boot/issues/432).
//! Design context in the #430 discussion (freshness vs integrity split).

use std::path::{Path, PathBuf};

/// Per-file integrity verdict.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FileVerdict {
    /// Stick-observed hash matches the manifest-recorded hash.
    Ok,
    /// Stick-observed hash differs from the manifest — tamper / corruption.
    Drift { stick: String, manifest: String },
    /// Couldn't read the file from the stick (missing, mtools error).
    Unreadable { reason: String },
}

/// One row of the stick verification report.
#[derive(Debug, Clone)]
pub(crate) struct FileResult {
    pub esp_path: String,
    pub expected_sha256: String,
    pub verdict: FileVerdict,
}

/// Aggregated counts across the report — used by both the human
/// renderer + the exit-code decision.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct VerifyTally {
    pub ok: u32,
    pub drift: u32,
    pub unreadable: u32,
}

impl VerifyTally {
    pub fn record(&mut self, v: &FileVerdict) {
        match v {
            FileVerdict::Ok => self.ok += 1,
            FileVerdict::Drift { .. } => self.drift += 1,
            FileVerdict::Unreadable { .. } => self.unreadable += 1,
        }
    }

    pub fn has_failures(self) -> bool {
        self.drift > 0 || self.unreadable > 0
    }
}

/// Top-level entry point called from `verify::try_run` when `--stick`
/// is set. Device path required — there's no sensible default for
/// "the stick" when multiple removable drives could be plugged in.
pub(crate) fn run(dev: &Path, json_mode: bool) -> Result<(), u8> {
    if !json_mode {
        println!("aegis-boot verify --stick — manifest integrity check");
        println!();
        println!("Target device: {}", dev.display());
        println!();
    }

    // Resolve disk GUID → attestation lookup. Same two-step we use in
    // `update::check_eligibility`; not refactored out yet because
    // update's resolver is wrapped in its own eligibility gate.
    let Some(disk_guid) = read_disk_guid(dev) else {
        return emit_error(
            json_mode,
            &format!(
                "could not read disk GUID from {} — ensure it's a GPT-partitioned aegis-boot stick",
                dev.display()
            ),
            2,
        );
    };

    let Some(manifest_path) = find_attestation_by_guid(&disk_guid) else {
        return emit_error(
            json_mode,
            &format!(
                "no attestation manifest found for disk GUID {disk_guid}. \
                 Run `aegis-boot flash --direct-install` on this stick to create one, \
                 or copy the manifest from the host that flashed it."
            ),
            2,
        );
    };

    verify_against_manifest(dev, &disk_guid, &manifest_path, json_mode)
}

/// Read the manifest, diff against the stick, tally, print, return
/// exit-code-shaped Result. Split from `run` to stay under clippy's
/// 100-line-per-function soft cap.
fn verify_against_manifest(
    dev: &Path,
    disk_guid: &str,
    manifest_path: &Path,
    json_mode: bool,
) -> Result<(), u8> {
    let manifest = match read_manifest(manifest_path) {
        Ok(m) => m,
        Err(e) => {
            return emit_error(
                json_mode,
                &format!(
                    "could not parse attestation manifest at {}: {e}",
                    manifest_path.display()
                ),
                2,
            );
        }
    };

    let esp_part = crate::update::partition_path(dev, 1);
    let results = verify_esp_files(&esp_part, &manifest.esp_files);
    let mut tally = VerifyTally::default();
    for r in &results {
        tally.record(&r.verdict);
    }

    if json_mode {
        print_json_report(dev, disk_guid, manifest_path, &results, tally);
    } else {
        print_human_report(manifest_path, disk_guid, &results, tally);
    }

    if tally.has_failures() { Err(1) } else { Ok(()) }
}

fn emit_error(json_mode: bool, msg: &str, code: u8) -> Result<(), u8> {
    if json_mode {
        let envelope = aegis_wire_formats::CliError {
            schema_version: aegis_wire_formats::CLI_ERROR_SCHEMA_VERSION,
            error: msg.to_string(),
        };
        match serde_json::to_string_pretty(&envelope) {
            Ok(body) => println!("{body}"),
            Err(err) => eprintln!("aegis-boot verify --stick: serialize error envelope: {err}"),
        }
    } else {
        eprintln!("aegis-boot verify --stick: {msg}");
    }
    Err(code)
}

/// For each `EspFileEntry`, hash the stick's copy and compare to the
/// manifest-recorded hash. Pure-fn over the mtype boundary so tests
/// can substitute a fixture hasher if needed.
fn verify_esp_files(
    esp_part: &Path,
    esp_files: &[aegis_wire_formats::EspFileEntry],
) -> Vec<FileResult> {
    esp_files
        .iter()
        .map(|entry| {
            // Manifest stores paths in `::/`-rooted mtools form. Strip
            // the `::` prefix for mtype_sha256 (which adds it back
            // internally — our existing convention).
            let stripped = entry.path.strip_prefix("::").unwrap_or(&entry.path);
            let verdict = match crate::update::mtype_sha256(esp_part, stripped) {
                Ok(observed) if hash_eq(&observed, &entry.sha256) => FileVerdict::Ok,
                Ok(observed) => FileVerdict::Drift {
                    stick: observed,
                    manifest: entry.sha256.clone(),
                },
                Err(reason) => FileVerdict::Unreadable { reason },
            };
            FileResult {
                esp_path: entry.path.clone(),
                expected_sha256: entry.sha256.clone(),
                verdict,
            }
        })
        .collect()
}

fn hash_eq(a: &str, b: &str) -> bool {
    a.trim().eq_ignore_ascii_case(b.trim())
}

fn print_human_report(
    manifest_path: &Path,
    disk_guid: &str,
    results: &[FileResult],
    tally: VerifyTally,
) {
    println!("  disk GUID:    {disk_guid}");
    println!("  attestation:  {}", manifest_path.display());
    println!();
    println!("Per-file integrity:");
    for r in results {
        match &r.verdict {
            FileVerdict::Ok => {
                let short = short_hash(&r.expected_sha256);
                println!("  [OK]     {}  sha256:{short}…", r.esp_path);
            }
            FileVerdict::Drift { stick, manifest } => {
                let s = short_hash(stick);
                let m = short_hash(manifest);
                println!("  [DRIFT]  {}  manifest:{m}…  stick:{s}…", r.esp_path);
            }
            FileVerdict::Unreadable { reason } => {
                println!("  [UNREAD] {}  {reason}", r.esp_path);
            }
        }
    }
    println!();
    println!(
        "Summary: {} OK, {} drift, {} unreadable (of {} total).",
        tally.ok,
        tally.drift,
        tally.unreadable,
        tally.ok + tally.drift + tally.unreadable
    );
    if tally.has_failures() {
        println!();
        println!("Recovery options (in order of least to most disruptive):");
        println!(
            "  1. `mren ::/<path>.bak ::/<path>` on the affected file(s) — \
             if a prior `update --apply` left a `.bak` you can roll back to."
        );
        println!(
            "  2. `aegis-boot update --apply --experimental-apply` (#181) — \
             rolls fresh host-side bytes onto the stick, preserves AEGIS_ISOS."
        );
        println!(
            "  3. `aegis-boot flash --direct-install` — destructive re-flash \
             (wipes AEGIS_ISOS); last resort if (1) and (2) don't resolve."
        );
    }
}

fn print_json_report(
    dev: &Path,
    disk_guid: &str,
    manifest_path: &Path,
    results: &[FileResult],
    tally: VerifyTally,
) {
    // Minimal inline JSON — a wire-format crate type can be added later
    // if scripted consumers want one. Today's shape is stable for the
    // feature's first release.
    let entries: Vec<serde_json::Value> = results
        .iter()
        .map(|r| match &r.verdict {
            FileVerdict::Ok => serde_json::json!({
                "path": r.esp_path,
                "verdict": "ok",
                "expected_sha256": r.expected_sha256,
            }),
            FileVerdict::Drift { stick, manifest } => serde_json::json!({
                "path": r.esp_path,
                "verdict": "drift",
                "expected_sha256": manifest,
                "observed_sha256": stick,
            }),
            FileVerdict::Unreadable { reason } => serde_json::json!({
                "path": r.esp_path,
                "verdict": "unreadable",
                "reason": reason,
            }),
        })
        .collect();
    let doc = serde_json::json!({
        "schema_version": 1,
        "device": dev.display().to_string(),
        "disk_guid": disk_guid,
        "attestation_path": manifest_path.display().to_string(),
        "files": entries,
        "summary": {
            "ok": tally.ok,
            "drift": tally.drift,
            "unreadable": tally.unreadable,
        }
    });
    match serde_json::to_string_pretty(&doc) {
        Ok(body) => println!("{body}"),
        Err(e) => eprintln!("aegis-boot verify --stick: serialize JSON: {e}"),
    }
}

fn short_hash(h: &str) -> &str {
    &h[..h.len().min(16)]
}

/// Read `disk_guid` via `sgdisk -p`. Thin wrapper around the same
/// parse used by `update::parse_disk_guid` to stay decoupled from that
/// module's internal state.
fn read_disk_guid(dev: &Path) -> Option<String> {
    use std::process::Command;
    let out = Command::new("sudo")
        .args(["sgdisk", "-p", dev.to_str()?])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    crate::update::parse_disk_guid(&text)
}

fn find_attestation_by_guid(target_guid: &str) -> Option<PathBuf> {
    let dir = crate::paths::attestations_dir();
    let entries = std::fs::read_dir(&dir).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let Ok(body) = std::fs::read_to_string(&path) else {
            continue;
        };
        if crate::update::body_contains_guid(&body, target_guid) {
            return Some(path);
        }
    }
    None
}

fn read_manifest(path: &Path) -> Result<aegis_wire_formats::Manifest, String> {
    let body = std::fs::read_to_string(path).map_err(|e| format!("read: {e}"))?;
    serde_json::from_str::<aegis_wire_formats::Manifest>(&body).map_err(|e| format!("parse: {e}"))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::missing_panics_doc)]
mod tests {
    use super::*;

    #[test]
    fn hash_eq_is_case_insensitive_and_whitespace_tolerant() {
        assert!(hash_eq("abc123", "ABC123"));
        assert!(hash_eq("  abc123  ", "abc123"));
        assert!(hash_eq("abc123", "abc123"));
        assert!(!hash_eq("abc123", "def456"));
        assert!(!hash_eq("abc12", "abc123"));
    }

    #[test]
    fn tally_record_counts_all_verdicts() {
        let mut t = VerifyTally::default();
        t.record(&FileVerdict::Ok);
        t.record(&FileVerdict::Ok);
        t.record(&FileVerdict::Drift {
            stick: "s".into(),
            manifest: "m".into(),
        });
        t.record(&FileVerdict::Unreadable { reason: "r".into() });
        assert_eq!(t.ok, 2);
        assert_eq!(t.drift, 1);
        assert_eq!(t.unreadable, 1);
        assert!(t.has_failures());
    }

    #[test]
    fn tally_all_ok_has_no_failures() {
        let mut t = VerifyTally::default();
        t.record(&FileVerdict::Ok);
        t.record(&FileVerdict::Ok);
        assert!(!t.has_failures());
    }

    #[test]
    fn short_hash_truncates_long_values() {
        assert_eq!(short_hash("0123456789abcdefFF"), "0123456789abcdef");
        assert_eq!(short_hash("deadbeef"), "deadbeef"); // shorter than 16 → full
        assert_eq!(short_hash(""), "");
    }
}
