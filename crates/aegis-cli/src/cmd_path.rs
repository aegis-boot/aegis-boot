// SPDX-License-Identifier: MIT OR Apache-2.0

//! Shared command-on-PATH probe. Every aegis-boot surface that asks
//! "is this command available?" goes through [`which`] so the answer
//! is the same whether `doctor`, `fetch-image`, or a future caller
//! is asking. Inconsistent answers were the surface area of #332 —
//! `doctor` said cosign was present while `fetch-image` said it was
//! missing, because each used a different probe.
//!
//! The probe is deliberately just "does the file exist as a regular
//! file at one of these paths?". It does NOT try to run the binary
//! (`--version` probes can return non-zero for reasons unrelated to
//! whether the binary is installed — missing network, locked
//! transparency log, corrupted keyring). Execution errors surface
//! at the actual use-site with the real stderr, which is more
//! actionable than "cosign not on PATH".
//!
//! ## Windows PATHEXT support (#504)
//!
//! On Windows, the shell auto-appends `PATHEXT` extensions
//! (`.EXE`, `.BAT`, `.CMD`, etc.) so operators type `cosign` and
//! the shell runs `cosign.exe`. This probe mirrors that — callers
//! pass a stem (no extension) and [`which`] tries the stem as given
//! plus every `PATHEXT` extension. On POSIX the stem is used
//! verbatim (no extension auto-append).

use std::path::{Path, PathBuf};

/// Canonical sbin directories probed when `$PATH` lookup misses.
/// Many distros (notably openSUSE, #328) do not include `/usr/sbin`
/// in the `$PATH` inherited by `sudo` or by child processes of the
/// install.sh post-install preflight. Root-utility commands that
/// live only in sbin (e.g. `sgdisk`) would otherwise produce a
/// FAIL row in `doctor` despite being installed.
pub(crate) const SBIN_FALLBACKS: &[&str] = &["/usr/sbin", "/sbin", "/usr/local/sbin"];

/// Fallback `PATHEXT` value when the env var is unset. Matches the
/// Windows default — `cmd.exe` uses this list in the same order.
/// Lowercase leading `.` is the canonical form but PATHEXT is
/// case-insensitive on Windows, so `.EXE` would match too.
#[cfg(target_os = "windows")]
const DEFAULT_PATHEXT: &str = ".COM;.EXE;.BAT;.CMD;.VBS;.JS;.WS;.PS1";

/// Look up `cmd` on the current process's PATH, falling back to the
/// canonical sbin directories if it's not found on PATH. Returns the
/// absolute path of the first match, or `None` if neither lookup hits.
pub(crate) fn which(cmd: &str) -> Option<PathBuf> {
    let pathext = pathext_for_lookup();
    which_in_with_pathext(
        cmd,
        std::env::var_os("PATH").as_deref(),
        SBIN_FALLBACKS,
        pathext.as_deref(),
    )
}

/// Resolve the effective `PATHEXT` value for the current platform.
/// On Windows, reads the env var and falls back to the canonical
/// default if unset. On POSIX, always `None` — shells don't
/// auto-append extensions.
///
/// Returning `Option<String>` even on POSIX (where it's always
/// `None`) keeps the caller signature single-shaped. The caller
/// passes `.as_deref()` straight into [`which_in_with_pathext`]
/// whose `pathext: Option<&str>` is the contract — collapsing
/// both platforms onto a bare `String` would force per-platform
/// branching at every call site.
#[allow(clippy::unnecessary_wraps)]
fn pathext_for_lookup() -> Option<String> {
    #[cfg(target_os = "windows")]
    {
        Some(std::env::var("PATHEXT").unwrap_or_else(|_| DEFAULT_PATHEXT.to_string()))
    }
    #[cfg(not(target_os = "windows"))]
    {
        None
    }
}

/// Explicit-inputs variant of [`which`] for testing. Takes the PATH
/// env value and the sbin-fallback list directly so a test can pin
/// the exact search space without mutating process-wide state.
///
/// No `PATHEXT` fallback — the POSIX lookup shape. For Windows
/// semantics in a test, use [`which_in_with_pathext`] and pass a
/// pathext value.
#[cfg(test)]
fn which_in(
    cmd: &str,
    path_env: Option<&std::ffi::OsStr>,
    sbin_fallbacks: &[&str],
) -> Option<PathBuf> {
    which_in_with_pathext(cmd, path_env, sbin_fallbacks, None)
}

/// Most-explicit variant — takes the PATHEXT value in addition to
/// PATH and sbin fallbacks. `Some(pathext)` triggers extension
/// auto-append behavior; `None` uses the stem as given (POSIX
/// semantics). Production callers go through [`which`] which
/// handles the platform switch.
///
/// Matching rules:
///
/// 1. For each directory in `path_env` + `sbin_fallbacks`, try the
///    stem as given (handles operators who pass `curl.exe` already),
///    then — if `pathext` is `Some` — try `dir.join(cmd + ext)` for
///    each `;`-separated extension.
/// 2. First match wins; return its absolute path.
///
/// Extension matching is case-insensitive on Windows by virtue of
/// NTFS being case-insensitive — we don't need to lower-case the
/// probe ourselves.
pub(crate) fn which_in_with_pathext(
    cmd: &str,
    path_env: Option<&std::ffi::OsStr>,
    sbin_fallbacks: &[&str],
    pathext: Option<&str>,
) -> Option<PathBuf> {
    if let Some(path) = path_env {
        for dir in std::env::split_paths(path) {
            if let Some(p) = find_in_dir(&dir, cmd, pathext) {
                return Some(p);
            }
        }
    }
    for sbin in sbin_fallbacks {
        if let Some(p) = find_in_dir(Path::new(sbin), cmd, pathext) {
            return Some(p);
        }
    }
    None
}

/// Probe `dir` for `cmd` — first as given, then (if `pathext` is
/// `Some`) appending each `;`-separated extension. Returns the
/// matching path or `None` if nothing resolves.
fn find_in_dir(dir: &Path, cmd: &str, pathext: Option<&str>) -> Option<PathBuf> {
    let bare = dir.join(cmd);
    if bare.is_file() {
        return Some(bare);
    }
    if let Some(pathext) = pathext {
        for ext in pathext.split(';') {
            // Skip empty entries (PATHEXT sometimes has trailing `;`).
            if ext.is_empty() {
                continue;
            }
            let candidate = dir.join(format!("{cmd}{ext}"));
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn which_in_finds_binary_on_path() {
        let Ok(dir) = tempfile::tempdir() else { return };
        let bin = dir.path().join("fake-cmd");
        if std::fs::write(&bin, b"#!/bin/sh\n").is_err() {
            return;
        }
        let path_env = std::ffi::OsString::from(dir.path());
        let found = which_in("fake-cmd", Some(path_env.as_os_str()), &[]);
        assert_eq!(found.as_deref(), Some(bin.as_path()));
    }

    #[test]
    fn which_in_falls_back_to_sbin_when_path_misses() {
        let Ok(dir) = tempfile::tempdir() else { return };
        let bin = dir.path().join("sbin-only-cmd");
        if std::fs::write(&bin, b"#!/bin/sh\n").is_err() {
            return;
        }
        let fallback = dir.path().to_string_lossy().into_owned();
        let found = which_in(
            "sbin-only-cmd",
            Some(std::ffi::OsStr::new("/nonexistent-path")),
            &[&fallback],
        );
        assert_eq!(found.as_deref(), Some(bin.as_path()));
    }

    #[test]
    fn which_in_returns_none_when_both_miss() {
        let found = which_in(
            "definitely-not-a-real-binary-for-aegis-test",
            Some(std::ffi::OsStr::new("/nonexistent-path")),
            &["/nonexistent-sbin"],
        );
        assert!(found.is_none());
    }

    #[test]
    fn pathext_finds_binary_by_extension_stem() {
        // Simulate Windows: `cosign` on PATH, actual file is
        // `cosign.exe`. The stem-as-given misses; PATHEXT rescues.
        //
        // Test uses lowercase extensions because the tempdir
        // filesystem is case-sensitive on Linux CI runners; on
        // Windows NTFS the match is case-insensitive, so either
        // case would work in production.
        let Ok(dir) = tempfile::tempdir() else { return };
        let bin = dir.path().join("cosign.exe");
        if std::fs::write(&bin, b"stub").is_err() {
            return;
        }
        let path_env = std::ffi::OsString::from(dir.path());
        let found = which_in_with_pathext(
            "cosign",
            Some(path_env.as_os_str()),
            &[],
            Some(".com;.exe;.bat"),
        );
        assert_eq!(found.as_deref(), Some(bin.as_path()));
    }

    #[test]
    fn pathext_tries_extensions_in_order() {
        // Two extensions match — PATHEXT order decides which wins.
        // PowerShell convention: .COM before .EXE. If both exist,
        // .COM should be preferred.
        let Ok(dir) = tempfile::tempdir() else { return };
        let com = dir.path().join("dual.com");
        let exe = dir.path().join("dual.exe");
        for f in [&com, &exe] {
            if std::fs::write(f, b"stub").is_err() {
                return;
            }
        }
        let path_env = std::ffi::OsString::from(dir.path());
        let found =
            which_in_with_pathext("dual", Some(path_env.as_os_str()), &[], Some(".com;.exe"));
        assert_eq!(found.as_deref(), Some(com.as_path()));
    }

    #[test]
    fn pathext_prefers_bare_match_over_extension_match() {
        // Operator passed the full name (e.g. `cosign.exe`) — use
        // that directly, don't re-append .EXE to get `cosign.exe.exe`.
        let Ok(dir) = tempfile::tempdir() else { return };
        let bin = dir.path().join("curl.exe");
        if std::fs::write(&bin, b"stub").is_err() {
            return;
        }
        let path_env = std::ffi::OsString::from(dir.path());
        let found = which_in_with_pathext(
            "curl.exe",
            Some(path_env.as_os_str()),
            &[],
            Some(".COM;.EXE"),
        );
        assert_eq!(found.as_deref(), Some(bin.as_path()));
    }

    #[test]
    fn pathext_returns_none_when_no_extension_matches() {
        // The binary exists as `foo.txt` — not an executable extension.
        let Ok(dir) = tempfile::tempdir() else { return };
        let bin = dir.path().join("foo.txt");
        if std::fs::write(&bin, b"hello").is_err() {
            return;
        }
        let path_env = std::ffi::OsString::from(dir.path());
        let found =
            which_in_with_pathext("foo", Some(path_env.as_os_str()), &[], Some(".COM;.EXE"));
        assert!(found.is_none());
    }

    #[test]
    fn pathext_skips_empty_entries_in_semicolon_list() {
        // Handle `PATHEXT=.com;;.exe` (trailing or duplicate `;`)
        // without probing the bare name twice.
        let Ok(dir) = tempfile::tempdir() else { return };
        let bin = dir.path().join("x.exe");
        if std::fs::write(&bin, b"stub").is_err() {
            return;
        }
        let path_env = std::ffi::OsString::from(dir.path());
        let found =
            which_in_with_pathext("x", Some(path_env.as_os_str()), &[], Some(".com;;.exe;"));
        assert_eq!(found.as_deref(), Some(bin.as_path()));
    }

    #[test]
    fn pathext_none_disables_extension_append() {
        // POSIX semantics — passing `None` means don't try `.EXE`
        // even if a `foo.EXE` file happens to exist on the path.
        let Ok(dir) = tempfile::tempdir() else { return };
        let bin = dir.path().join("posix-cmd.exe");
        if std::fs::write(&bin, b"stub").is_err() {
            return;
        }
        let path_env = std::ffi::OsString::from(dir.path());
        let found = which_in_with_pathext("posix-cmd", Some(path_env.as_os_str()), &[], None);
        assert!(found.is_none());
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn which_resolves_windows_cmd_without_explicit_extension() {
        // Integration: the production `which` finds `cmd` → cmd.exe
        // on the real Windows runner environment. Confirms the
        // PATHEXT plumbing is wired end-to-end.
        let found = which("cmd");
        assert!(
            found.is_some(),
            "expected `cmd` to resolve via PATHEXT on Windows, got None"
        );
    }
}
