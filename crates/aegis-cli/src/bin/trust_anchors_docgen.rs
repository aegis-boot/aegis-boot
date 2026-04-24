// SPDX-License-Identifier: MIT OR Apache-2.0

//! `trust-anchors-docgen` — renders the ADR 0002 epoch history from
//! `keys/historical-anchors.json` (and the active floor from
//! `keys/canonical-epoch.json`) into marker regions in
//! `docs/architecture/KEY_MANAGEMENT.md`.
//!
//! Companion to `constants-docgen` (numeric drift guard) and
//! `tiers-docgen` (trust-tier table). Solves the specific drift
//! observed after ADR 0002 rev-3 landed: the illustrative code fence
//! in §3.6 described a schema that diverged from the `aegis-trust`
//! crate's actual [`EpochEntry`], and the table of "anchors this
//! binary recognizes" had no self-updating source. The `keys/`
//! directory is already the committed source of truth for both files;
//! this binary just surfaces them in the operator-facing doc.
//!
//! ## Contract
//!
//! Target doc contains marker pairs on their own lines:
//!
//! ```text
//! <!-- trust-anchors:BEGIN:EPOCH_TABLE -->
//! | Epoch | Valid from | Expires at | Pubkey fingerprint | Source file | Note |
//! | ----- | ---------- | ---------- | ------------------ | ----------- | ---- |
//! | 1     | 2026-04-24 | (active)   | RWSl…pF/I          | ...         | ...  |
//! <!-- trust-anchors:END:EPOCH_TABLE -->
//! ```
//!
//! ## Modes
//!
//! - `--write` (default): rewrite target doc files in place.
//! - `--check`: diff-only; non-zero exit if any file would change.
//!   Wired into CI so drift fails PR.
//!
//! [`EpochEntry`]: crates/aegis-trust/src/anchor.rs

#![forbid(unsafe_code)]

use std::env;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use serde::Deserialize;

/// Parsed entry from `keys/historical-anchors.json`. Mirrors the
/// shape of `aegis_trust::EpochEntry` but decoupled — the docgen
/// owns its own serde types so the aegis-trust crate doesn't grow
/// a docgen-only API surface.
#[derive(Debug, Clone, Deserialize)]
struct EpochEntry {
    epoch: u32,
    pubkey_file: String,
    pubkey_fingerprint: String,
    valid_from: String,
    expires_at: Option<String>,
    #[serde(default)]
    note: String,
}

/// Parsed shape of `keys/canonical-epoch.json`. Only the integer
/// `epoch` matters for the table (it pins which row is the active
/// floor); the SHA-256 anchor + `schema_version` + note fields are
/// consumed by `build.rs` elsewhere.
#[derive(Debug, Deserialize)]
struct CanonicalEpoch {
    epoch: u32,
}

/// A registered marker + its rendered body. Kept structurally
/// identical to `constants-docgen::Marker` so the marker-replace
/// engine below can stay prose-free.
struct Marker {
    name: &'static str,
    value: String,
}

/// Markers this tool fills. For now there's exactly one, but the
/// registry shape leaves room to add e.g. a canonical-epoch-sha row
/// without changing the apply-markers plumbing.
fn registry(epochs: &[EpochEntry], canonical: &CanonicalEpoch) -> Vec<Marker> {
    vec![Marker {
        name: "EPOCH_TABLE",
        value: render_epoch_table(epochs, canonical),
    }]
}

/// Render the epoch-history table as GitHub-flavored Markdown.
///
/// Row order: active (canonical) epoch first, then prior epochs in
/// descending order — operators debugging trust drift care about the
/// active row first.
fn render_epoch_table(epochs: &[EpochEntry], canonical: &CanonicalEpoch) -> String {
    let mut out = String::new();
    out.push('\n');
    out.push_str("| Epoch | Valid from | Expires at | Pubkey fingerprint | Source file | Note |\n");
    out.push_str("| ----- | ---------- | ---------- | ------------------ | ----------- | ---- |\n");
    let mut sorted: Vec<&EpochEntry> = epochs.iter().collect();
    sorted.sort_by(|a, b| b.epoch.cmp(&a.epoch));
    for e in sorted {
        let active = if e.epoch == canonical.epoch {
            " (active)"
        } else {
            ""
        };
        let valid_from = short_date(&e.valid_from);
        let expires_at = match &e.expires_at {
            Some(v) => short_date(v),
            None => "(none)".to_string(),
        };
        let _ = writeln!(
            out,
            "| {epoch}{active} | {valid_from} | {expires_at} | `{fp}` | `{file}` | {note} |",
            epoch = e.epoch,
            active = active,
            valid_from = valid_from,
            expires_at = expires_at,
            fp = short_fingerprint(&e.pubkey_fingerprint),
            file = e.pubkey_file,
            note = one_line(&e.note),
        );
    }
    out
}

/// Truncate RFC3339 timestamps to the date portion for table
/// readability. Leaves non-conforming strings alone.
fn short_date(s: &str) -> String {
    s.split_once('T')
        .map_or_else(|| s.to_string(), |(date, _)| date.to_string())
}

/// Short-render a base64 minisign pubkey fingerprint as
/// `<first 4>…<last 4>`. Full fingerprint is still in the source
/// JSON; the table wants a glance-identifier, not a 56-char blob.
fn short_fingerprint(fp: &str) -> String {
    if fp.len() <= 12 {
        return fp.to_string();
    }
    let head: String = fp.chars().take(4).collect();
    let tail_start = fp.chars().count().saturating_sub(4);
    let tail: String = fp.chars().skip(tail_start).collect();
    format!("{head}…{tail}")
}

/// Collapse newlines + pipe chars in a note to keep the row on one
/// line. Notes are author-controlled JSON and may legitimately use
/// pipes in prose; escape them rather than truncate.
fn one_line(s: &str) -> String {
    s.replace('\n', " ").replace('|', "\\|")
}

/// Fixed list of doc files this tool may rewrite. Hard-coded rather
/// than a `walkdir` scan so the scope is explicit — mirrors
/// `constants-docgen::target_files`.
fn target_files(repo_root: &Path) -> Vec<PathBuf> {
    ["docs/architecture/KEY_MANAGEMENT.md"]
        .iter()
        .map(|p| repo_root.join(p))
        .collect()
}

/// Apply all `trust-anchors:BEGIN:NAME` / `:END:NAME` marker pairs
/// in `input`, rewriting the body between them with the matching
/// registry value. Preserves the markers themselves verbatim.
fn apply_markers(input: &str, markers: &[Marker]) -> (String, usize) {
    let mut out = String::with_capacity(input.len());
    let mut cursor = 0usize;
    let mut replacements = 0usize;
    let bytes = input.as_bytes();

    while let Some(begin_abs) = find_from(bytes, cursor, b"<!-- trust-anchors:BEGIN:") {
        let Some(close_idx) = find_from(bytes, begin_abs, b"-->") else {
            break;
        };
        let after_begin_tag = close_idx + 3;
        let Some(name) = marker_name(&input[begin_abs..after_begin_tag]) else {
            out.push_str(&input[cursor..after_begin_tag]);
            cursor = after_begin_tag;
            continue;
        };
        let end_tag = format!("<!-- trust-anchors:END:{name} -->");
        let Some(end_abs) = find_from(bytes, after_begin_tag, end_tag.as_bytes()) else {
            break;
        };

        out.push_str(&input[cursor..after_begin_tag]);
        if let Some(m) = markers.iter().find(|m| m.name == name) {
            out.push_str(&m.value);
            replacements += 1;
        } else {
            out.push_str(&input[after_begin_tag..end_abs]);
        }
        out.push_str(&end_tag);
        cursor = end_abs + end_tag.len();
    }

    out.push_str(&input[cursor..]);
    (out, replacements)
}

fn marker_name(begin_tag: &str) -> Option<&str> {
    let prefix = "<!-- trust-anchors:BEGIN:";
    let suffix = " -->";
    let inner = begin_tag.strip_prefix(prefix)?.strip_suffix(suffix)?;
    if inner.is_empty() || !inner.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return None;
    }
    Some(inner)
}

fn find_from(haystack: &[u8], start: usize, needle: &[u8]) -> Option<usize> {
    if start > haystack.len() {
        return None;
    }
    haystack[start..]
        .windows(needle.len())
        .position(|w| w == needle)
        .map(|p| start + p)
}

enum Mode {
    Write,
    Check,
}

fn parse_mode(args: &[String]) -> Result<Mode, String> {
    match args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        [] | ["--write"] => Ok(Mode::Write),
        ["--check"] => Ok(Mode::Check),
        _ => Err(format!(
            "usage: trust-anchors-docgen [--write|--check]  (got: {args:?})"
        )),
    }
}

fn find_repo_root(start: &Path) -> Result<PathBuf, String> {
    let mut cur = start;
    loop {
        let candidate = cur.join("Cargo.toml");
        if candidate.is_file() {
            if let Ok(body) = fs::read_to_string(&candidate) {
                if body.contains("[workspace]") {
                    return Ok(cur.to_path_buf());
                }
            }
        }
        match cur.parent() {
            Some(parent) => cur = parent,
            None => {
                return Err(format!(
                    "trust-anchors-docgen: could not find workspace root walking up from {}",
                    start.display()
                ));
            }
        }
    }
}

fn load_inputs(repo_root: &Path) -> Result<(Vec<EpochEntry>, CanonicalEpoch), String> {
    let anchors_path = repo_root.join("keys/historical-anchors.json");
    let canonical_path = repo_root.join("keys/canonical-epoch.json");

    let anchors_body = fs::read_to_string(&anchors_path)
        .map_err(|e| format!("cannot read {}: {e}", anchors_path.display()))?;
    let canonical_body = fs::read_to_string(&canonical_path)
        .map_err(|e| format!("cannot read {}: {e}", canonical_path.display()))?;

    let epochs: Vec<EpochEntry> = serde_json::from_str(&anchors_body)
        .map_err(|e| format!("{} parse: {e}", anchors_path.display()))?;
    let canonical: CanonicalEpoch = serde_json::from_str(&canonical_body)
        .map_err(|e| format!("{} parse: {e}", canonical_path.display()))?;

    Ok((epochs, canonical))
}

fn main() -> ExitCode {
    // Devtool — args are flag names only. Same rationale as sibling
    // docgens (see constants-docgen::main).
    // nosemgrep: rust.lang.security.args.args
    let args: Vec<String> = env::args().skip(1).collect();
    let mode = match parse_mode(&args) {
        Ok(m) => m,
        Err(msg) => {
            eprintln!("{msg}");
            return ExitCode::from(2);
        }
    };

    let cwd = match env::current_dir() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("trust-anchors-docgen: cannot read CWD: {e}");
            return ExitCode::from(2);
        }
    };
    let repo_root = match find_repo_root(&cwd) {
        Ok(p) => p,
        Err(msg) => {
            eprintln!("{msg}");
            return ExitCode::from(2);
        }
    };
    let (epochs, canonical) = match load_inputs(&repo_root) {
        Ok(v) => v,
        Err(msg) => {
            eprintln!("trust-anchors-docgen: {msg}");
            return ExitCode::from(2);
        }
    };

    let markers = registry(&epochs, &canonical);
    let mut drift_files: Vec<PathBuf> = Vec::new();
    let mut total_replacements = 0usize;

    for path in target_files(&repo_root) {
        let body = match fs::read_to_string(&path) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("trust-anchors-docgen: cannot read {}: {e}", path.display());
                return ExitCode::from(2);
            }
        };
        let (rendered, n) = apply_markers(&body, &markers);
        total_replacements += n;

        match mode {
            Mode::Write => {
                if rendered == body {
                    println!("unchanged {} ({n} markers)", path.display());
                } else {
                    if let Err(e) = fs::write(&path, &rendered) {
                        eprintln!("trust-anchors-docgen: cannot write {}: {e}", path.display());
                        return ExitCode::from(2);
                    }
                    println!("updated {} ({n} markers)", path.display());
                }
            }
            Mode::Check => {
                if rendered != body {
                    drift_files.push(path.clone());
                    eprintln!(
                        "drift: {} would change ({n} markers render differently than committed)",
                        path.display()
                    );
                }
            }
        }
    }

    match mode {
        Mode::Write => {
            println!(
                "trust-anchors-docgen: wrote {} target files, {total_replacements} total markers rendered",
                target_files(&repo_root).len()
            );
            ExitCode::SUCCESS
        }
        Mode::Check => {
            if drift_files.is_empty() {
                println!(
                    "trust-anchors-docgen: OK — {total_replacements} markers across {} files all match registry",
                    target_files(&repo_root).len()
                );
                ExitCode::SUCCESS
            } else {
                eprintln!();
                eprintln!(
                    "trust-anchors-docgen: FAIL — {} file(s) diverge from keys/*.json.",
                    drift_files.len()
                );
                eprintln!(
                    "Fix: run `cargo run -p aegis-bootctl --bin trust-anchors-docgen --features docgen` locally and commit the result."
                );
                ExitCode::from(1)
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn sample_epochs() -> Vec<EpochEntry> {
        vec![
            EpochEntry {
                epoch: 1,
                pubkey_file: "keys/maintainer-epoch-1.pub".to_string(),
                pubkey_fingerprint: "RWSldhpvdTOXAKMBeOheDkE8N2rCPsD9Duct4n2ToDOf/cRqRijjpF/I"
                    .to_string(),
                valid_from: "2026-04-24T14:09:30-04:00".to_string(),
                expires_at: None,
                note: "Initial key.".to_string(),
            },
            EpochEntry {
                epoch: 2,
                pubkey_file: "keys/maintainer-epoch-2.pub".to_string(),
                pubkey_fingerprint: "RWAAAABBBBCCCCDDDDEEEEFFFFGGGGHHHHIIIIJJJJKKKKLLLLMMMMNNNN"
                    .to_string(),
                valid_from: "2027-03-15T00:00:00Z".to_string(),
                expires_at: Some("2028-03-15T00:00:00Z".to_string()),
                note: "Scheduled rotation.".to_string(),
            },
        ]
    }

    fn canonical_epoch_two() -> CanonicalEpoch {
        CanonicalEpoch { epoch: 2 }
    }

    #[test]
    fn table_puts_active_row_first() {
        let table = render_epoch_table(&sample_epochs(), &canonical_epoch_two());
        let lines: Vec<&str> = table.lines().collect();
        // lines[0] = blank padding, [1] = header, [2] = separator, [3] = first data row.
        assert!(
            lines[3].contains("| 2 (active) |"),
            "first data row must be the active (epoch 2) entry: {table}"
        );
        assert!(
            lines[4].contains("| 1 |"),
            "second data row must be epoch 1: {table}"
        );
    }

    #[test]
    fn table_marks_only_canonical_epoch_active() {
        let table = render_epoch_table(&sample_epochs(), &CanonicalEpoch { epoch: 1 });
        assert!(table.contains("| 1 (active) |"));
        // Epoch 2 row must NOT be marked active when canonical=1.
        assert!(!table.contains("| 2 (active) |"));
    }

    #[test]
    fn short_fingerprint_elides_long_keys() {
        let fp = "RWSldhpvdTOXAKMBeOheDkE8N2rCPsD9Duct4n2ToDOf/cRqRijjpF/I";
        let short = short_fingerprint(fp);
        assert!(short.starts_with("RWSl"));
        assert!(short.ends_with("pF/I"));
        assert!(short.contains('…'));
    }

    #[test]
    fn short_date_strips_time_component() {
        assert_eq!(short_date("2026-04-24T14:09:30-04:00"), "2026-04-24");
        assert_eq!(short_date("2027-03-15T00:00:00Z"), "2027-03-15");
        // Non-RFC3339 strings are passed through.
        assert_eq!(short_date("just a date"), "just a date");
    }

    #[test]
    fn expires_at_renders_as_none_when_absent() {
        let table = render_epoch_table(&sample_epochs()[..1], &CanonicalEpoch { epoch: 1 });
        assert!(table.contains("(none)"));
    }

    #[test]
    fn apply_markers_replaces_between_tags() {
        let body = "prefix\n<!-- trust-anchors:BEGIN:EPOCH_TABLE -->\nold content\n<!-- trust-anchors:END:EPOCH_TABLE -->\nsuffix\n";
        let markers = vec![Marker {
            name: "EPOCH_TABLE",
            value: "NEW".to_string(),
        }];
        let (rendered, n) = apply_markers(body, &markers);
        assert_eq!(n, 1);
        assert!(rendered.contains("EPOCH_TABLE -->NEW<!-- trust-anchors:END"));
        assert!(!rendered.contains("old content"));
    }

    #[test]
    fn apply_markers_preserves_unknown_names() {
        let body = "<!-- trust-anchors:BEGIN:UNKNOWN -->keep-me<!-- trust-anchors:END:UNKNOWN -->";
        let markers = vec![Marker {
            name: "EPOCH_TABLE",
            value: "NEW".to_string(),
        }];
        let (rendered, n) = apply_markers(body, &markers);
        assert_eq!(n, 0);
        assert!(rendered.contains("keep-me"));
    }

    #[test]
    fn one_line_escapes_pipes_and_newlines() {
        assert_eq!(one_line("a | b\nc"), "a \\| b c");
    }

    #[test]
    fn marker_name_rejects_invalid_chars() {
        assert_eq!(
            marker_name("<!-- trust-anchors:BEGIN:OK_1 -->"),
            Some("OK_1")
        );
        assert_eq!(marker_name("<!-- trust-anchors:BEGIN:bad-name -->"), None);
        assert_eq!(marker_name("<!-- trust-anchors:BEGIN: -->"), None);
    }
}
