//! `aegis-boot fetch-image` — download + verify a pre-built aegis-boot
//! disk image.
//!
//! Pairs with `aegis-boot flash --image PATH` (PR #229). Today, macOS
//! and Windows operators who can't run `mkusb.sh` (Linux-only)
//! had no clean way to get a buildable image. With `fetch-image`:
//!
//! ```sh
//! img=$(aegis-boot fetch-image --url https://example.com/aegis-boot.img \
//!     --sha256 abcd...1234)
//! aegis-boot flash --image "$img" /dev/disk5
//! ```
//!
//! Verification has two layers:
//!
//!   1. **sha256** (`--sha256 HASH`) — required-if-provided; mismatch
//!      deletes the file + exits 1. When omitted, a WARNING surfaces
//!      the computed hash so the operator can pin it for next time.
//!   2. **cosign keyless** (auto-enabled, hardcoded identity bound to
//!      aegis-boot's own `release.yml`) — downloads `<URL>.sig` +
//!      `<URL>.pem` from the same origin as the image, then shells out
//!      to `cosign verify-blob`. **Graceful-degrades** when the `.sig`
//!      / `.pem` aren't published (curl 404): surfaces a warning and
//!      proceeds on the sha256 contract alone. `--no-cosign` skips
//!      the attempt entirely for air-gapped / offline contexts where
//!      the Sigstore transparency-log lookup would fail anyway.
//!
//! Why auto-enabled with graceful-degrade (2026-04-18 decision on #235):
//! best practice is "verify when possible, don't fail-closed on old /
//! fork / test-release URLs that never published signatures." Operators
//! who need strict verification can watch for the "cosign sig ✓" line
//! in the progress output, or grep the `--json` envelope once that
//! surface ships.
//!
//! Subprocess use: shells out to `curl` (already a host dep used by
//! install.sh + other subcommands) and to `cosign` (added as an
//! operator-host dep; `aegis-boot doctor` now reports cosign presence).
//! No new crate dependencies.

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

/// Entry point for `aegis-boot fetch-image [args]`.
pub fn run(args: &[String]) -> ExitCode {
    match try_run(args) {
        Ok(path) => {
            // Print the path to stdout so it composes via $(...).
            println!("{}", path.display());
            ExitCode::SUCCESS
        }
        Err(code) => ExitCode::from(code),
    }
}

/// Inner runner returning the verified path on success.
fn try_run(args: &[String]) -> Result<PathBuf, u8> {
    let parsed = parse_args(args)?;
    if parsed.help_requested {
        print_help();
        return Err(0);
    }
    let url = parsed.url.ok_or_else(|| {
        eprintln!("aegis-boot fetch-image: --url is required");
        eprintln!("Run 'aegis-boot fetch-image --help' for usage.");
        2
    })?;

    if !is_safe_https_url(&url) {
        eprintln!(
            "aegis-boot fetch-image: refusing URL '{url}' — only https:// URLs are accepted \
             (signed-chain integrity assumes TLS)."
        );
        return Err(2);
    }

    let out_path = match parsed.out {
        Some(p) => p,
        None => default_cache_path(&url, parsed.expected_sha256.as_deref())?,
    };

    if let Some(parent) = out_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            eprintln!(
                "aegis-boot fetch-image: cannot create cache dir {}: {e}",
                parent.display()
            );
            1
        })?;
    }

    eprintln!(
        "aegis-boot fetch-image: downloading {url} → {}",
        out_path.display()
    );
    download_via_curl(&url, &out_path)?;

    if let Some(expected) = parsed.expected_sha256.as_deref() {
        let got = compute_sha256(&out_path)?;
        if !got.eq_ignore_ascii_case(expected) {
            eprintln!("aegis-boot fetch-image: sha256 mismatch");
            eprintln!("  expected: {expected}");
            eprintln!("  got:      {got}");
            // Remove the file so a second run doesn't accidentally
            // skip the download + believe the cached bytes.
            let _ = std::fs::remove_file(&out_path);
            return Err(1);
        }
        eprintln!("aegis-boot fetch-image: sha256 verified");
    } else {
        // No --sha256 provided. Surface the computed hash so the
        // operator can pin it on subsequent runs.
        let got = compute_sha256(&out_path).unwrap_or_else(|_| "<sha256 unavailable>".into());
        eprintln!(
            "aegis-boot fetch-image: WARNING — no --sha256 supplied; cannot verify integrity"
        );
        eprintln!("  computed: {got}");
        eprintln!("  Re-run with --sha256 {got} to pin this image for future fetches.");
    }

    // Cosign keyless verification (#235) — auto-enabled with
    // graceful-degrade on missing signatures. The operator can bypass
    // with --no-cosign for offline / air-gapped scenarios where the
    // Sigstore transparency-log lookup would fail anyway.
    if parsed.cosign_disabled {
        eprintln!("aegis-boot fetch-image: cosign verification skipped (--no-cosign)");
    } else {
        try_cosign_verify(&url, &out_path);
    }

    Ok(out_path)
}

/// Attempt to download `<url>.sig` + `<url>.pem` and run
/// `cosign verify-blob` against them. On verification **success**,
/// emit a confirmation line and return. On the following conditions,
/// surface a WARNING and proceed (graceful degrade — the sha256 layer
/// is still active, if supplied):
///
///   * `cosign` not on PATH — operator host lacks the binary
///   * curl fails to download `.sig` or `.pem` — typically HTTP 404
///     when the release pre-dates signature publishing
///
/// On verification **failure** (signatures present but don't match the
/// hardcoded identity or the image bytes), the image is deleted and
/// we return early — same fail-closed contract as the sha256-mismatch
/// path. The `out_path` argument is kept by reference because we want
/// to be able to unlink it on mismatch.
fn try_cosign_verify(url: &str, image_path: &Path) {
    if !cosign_on_path() {
        eprintln!(
            "aegis-boot fetch-image: WARNING — cosign not on PATH; skipping signature \
             verification. Install cosign (https://docs.sigstore.dev/cosign/installation/) \
             or pass --no-cosign to silence this warning."
        );
        return;
    }

    // Stage .sig + .pem into the same directory as the image so a
    // future rerun that re-downloads the image also overwrites stale
    // signature files. Keep the files (don't delete on success) so
    // operators can re-verify offline later if desired.
    let sig_path = sibling_with_suffix(image_path, ".sig");
    let pem_path = sibling_with_suffix(image_path, ".pem");

    let sig_url = format!("{url}.sig");
    let pem_url = format!("{url}.pem");

    if !try_download_signature(&sig_url, &sig_path, ".sig") {
        return;
    }
    if !try_download_signature(&pem_url, &pem_path, ".pem") {
        return;
    }

    match run_cosign_verify_blob(image_path, &sig_path, &pem_path) {
        Ok(()) => {
            eprintln!("aegis-boot fetch-image: cosign keyless signature verified ✓");
            eprintln!("  identity: {COSIGN_IDENTITY_REGEX}\n  issuer:   {COSIGN_OIDC_ISSUER}");
        }
        Err(reason) => {
            eprintln!(
                "aegis-boot fetch-image: cosign verification FAILED — {reason}\n  \
                 The downloaded bytes could not be cryptographically attributed to \
                 aegis-boot's release workflow. Deleting the file."
            );
            let _ = std::fs::remove_file(image_path);
            let _ = std::fs::remove_file(&sig_path);
            let _ = std::fs::remove_file(&pem_path);
            std::process::exit(1);
        }
    }
}

/// Try downloading a signature-sidecar file. Returns `true` when the
/// operator should proceed with cosign verification; `false` when a
/// graceful-degrade warning has been emitted and the caller should
/// skip verification (e.g. the sidecar isn't published for this
/// release).
fn try_download_signature(url: &str, out: &Path, suffix_label: &str) -> bool {
    // Quiet curl — we surface our own messages on failure.
    let status = Command::new("curl")
        .args([
            "--fail",
            "--silent",
            "--show-error",
            "--location",
            "--proto",
            "=https",
            "--tlsv1.2",
            "--output",
            &out.display().to_string(),
            url,
        ])
        .status();
    match status {
        Ok(s) if s.success() => true,
        Ok(_) => {
            eprintln!(
                "aegis-boot fetch-image: WARNING — {suffix_label} not available at {url} \
                 (release likely pre-dates cosign signing). Skipping signature \
                 verification; sha256 contract still applies."
            );
            false
        }
        Err(e) => {
            eprintln!(
                "aegis-boot fetch-image: WARNING — cannot invoke curl to fetch {suffix_label}: {e}. \
                 Skipping signature verification."
            );
            false
        }
    }
}

/// The cosign identity regex hardcoded for aegis-boot's own releases.
/// Matches `release.yml` at any tag ref (`refs/tags/...`) on the
/// upstream repository. Forks that publish their own releases with
/// their own OIDC identity need a different CLI tool — this one is
/// bound to `williamzujkowski/aegis-boot` by design.
///
/// If we're wrong about this regex shape, the worst that happens is
/// a FAILED verification on otherwise-correct artifacts — operators
/// see the failure, can manually re-verify with an adjusted regex,
/// and we fix the regex in a point release.
const COSIGN_IDENTITY_REGEX: &str = r"^https://github\.com/williamzujkowski/aegis-boot/\.github/workflows/release\.yml@refs/tags/.+$";

/// The Sigstore OIDC issuer for GitHub Actions' ambient OIDC tokens.
/// This is a stable public endpoint; hardcoding is intentional.
const COSIGN_OIDC_ISSUER: &str = "https://token.actions.githubusercontent.com";

/// Invoke `cosign verify-blob` against a detached signature + cert.
/// Returns `Ok(())` when cosign exits 0 and the blob is attributed to
/// our hardcoded identity; `Err(msg)` otherwise. The stderr output is
/// captured and forwarded into the error message so operators can see
/// exactly what cosign refused.
fn run_cosign_verify_blob(image: &Path, sig: &Path, pem: &Path) -> Result<(), String> {
    let output = Command::new("cosign")
        .args([
            "verify-blob",
            "--signature",
            &sig.display().to_string(),
            "--certificate",
            &pem.display().to_string(),
            "--certificate-identity-regexp",
            COSIGN_IDENTITY_REGEX,
            "--certificate-oidc-issuer",
            COSIGN_OIDC_ISSUER,
            &image.display().to_string(),
        ])
        .output()
        .map_err(|e| format!("cannot run cosign: {e}"))?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stderr_trimmed = stderr.trim();
    if stderr_trimmed.is_empty() {
        Err(format!("cosign exited {}", output.status))
    } else {
        Err(format!("cosign exited {}: {stderr_trimmed}", output.status))
    }
}

/// Check `cosign --version` returns 0 on the operator's PATH. Same
/// pattern as the `sudo` / `sha256sum` presence checks in
/// `aegis-boot doctor`. Called from both here (lazy) and from doctor
/// (eager) so operators get the same answer either way.
pub(crate) fn cosign_on_path() -> bool {
    Command::new("cosign")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Append a suffix to the file path. `/tmp/a.img` + `.sig` →
/// `/tmp/a.img.sig`. Kept as a small helper because the `Path` API
/// doesn't have a direct "append to basename" verb.
fn sibling_with_suffix(image: &Path, suffix: &str) -> PathBuf {
    let mut s = image.as_os_str().to_os_string();
    s.push(suffix);
    PathBuf::from(s)
}

/// Parsed argv. All options optional; --url is required at runtime.
#[derive(Debug)]
struct ParsedArgs {
    help_requested: bool,
    url: Option<String>,
    out: Option<PathBuf>,
    expected_sha256: Option<String>,
    /// When true, skip the cosign auto-verification step entirely.
    /// Useful for air-gapped / offline contexts where the Sigstore
    /// transparency-log lookup would fail regardless.
    cosign_disabled: bool,
}

fn parse_args(args: &[String]) -> Result<ParsedArgs, u8> {
    let mut p = ParsedArgs {
        help_requested: false,
        url: None,
        out: None,
        expected_sha256: None,
        cosign_disabled: false,
    };
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        match a.as_str() {
            "--help" | "-h" => p.help_requested = true,
            "--url" => {
                i += 1;
                let Some(v) = args.get(i) else {
                    eprintln!("aegis-boot fetch-image: --url requires a value");
                    return Err(2);
                };
                p.url = Some(v.clone());
            }
            arg if arg.starts_with("--url=") => {
                p.url = Some(arg["--url=".len()..].to_string());
            }
            "--out" => {
                i += 1;
                let Some(v) = args.get(i) else {
                    eprintln!("aegis-boot fetch-image: --out requires a path");
                    return Err(2);
                };
                p.out = Some(PathBuf::from(v));
            }
            arg if arg.starts_with("--out=") => {
                p.out = Some(PathBuf::from(&arg["--out=".len()..]));
            }
            "--sha256" => {
                i += 1;
                let Some(v) = args.get(i) else {
                    eprintln!("aegis-boot fetch-image: --sha256 requires a 64-char hex value");
                    return Err(2);
                };
                p.expected_sha256 = Some(v.clone());
            }
            arg if arg.starts_with("--sha256=") => {
                p.expected_sha256 = Some(arg["--sha256=".len()..].to_string());
            }
            "--no-cosign" => p.cosign_disabled = true,
            arg if arg.starts_with("--") => {
                eprintln!("aegis-boot fetch-image: unknown option '{arg}'");
                return Err(2);
            }
            other => {
                eprintln!("aegis-boot fetch-image: unexpected positional arg '{other}'");
                return Err(2);
            }
        }
        i += 1;
    }
    if let Some(s) = p.expected_sha256.as_deref() {
        if !is_valid_sha256_hex(s) {
            eprintln!(
                "aegis-boot fetch-image: --sha256 must be 64 hex chars (got {} chars)",
                s.len()
            );
            return Err(2);
        }
    }
    Ok(p)
}

fn print_help() {
    println!("aegis-boot fetch-image — download + verify a pre-built aegis-boot image");
    println!();
    println!("USAGE:");
    println!("  aegis-boot fetch-image --url URL [--out PATH] [--sha256 HEX] [--no-cosign]");
    println!();
    println!("  --url URL       HTTPS URL of the aegis-boot.img to download (required)");
    println!("  --out PATH      Where to write the image (default: $XDG_CACHE_HOME/aegis-boot/)");
    println!("  --sha256 HEX    Required sha256; mismatch deletes the download + exits 1");
    println!("  --no-cosign     Skip the cosign keyless signature check (air-gap, offline)");
    println!();
    println!("VERIFICATION:");
    println!("  sha256:  required-if-passed; prints computed hash when omitted");
    println!("  cosign:  auto-attempted; downloads <URL>.sig + <URL>.pem, runs");
    println!("           `cosign verify-blob` against aegis-boot's release-workflow OIDC");
    println!("           identity. Graceful-degrades when signatures aren't published.");
    println!();
    println!("Composes with `flash`:");
    println!("  img=$(aegis-boot fetch-image --url ... --sha256 ...) && \\");
    println!("    aegis-boot flash --image \"$img\" /dev/sdX");
}

/// Reject anything that isn't a plain `https://` URL. We don't accept
/// `http://` (no integrity), `file://` (use the file directly), or
/// anything fancy. Keeps the attack surface tiny.
fn is_safe_https_url(s: &str) -> bool {
    s.starts_with("https://") && !s.contains('\0') && !s.contains('\n') && !s.contains('\r')
}

/// 64 lowercase or uppercase hex chars.
fn is_valid_sha256_hex(s: &str) -> bool {
    s.len() == 64 && s.chars().all(|c| c.is_ascii_hexdigit())
}

/// Default cache path under `$XDG_CACHE_HOME` (or `~/.cache/`). The
/// filename includes a sha256 prefix when one was supplied so distinct
/// pinned images don't collide.
fn default_cache_path(url: &str, expected_sha256: Option<&str>) -> Result<PathBuf, u8> {
    let cache_home = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|h| {
                let mut p = PathBuf::from(h);
                p.push(".cache");
                p
            })
        })
        .ok_or_else(|| {
            eprintln!("aegis-boot fetch-image: cannot determine cache dir; set --out");
            1_u8
        })?;
    let mut p = cache_home.join("aegis-boot");
    let basename = if let Some(hash) = expected_sha256 {
        format!("aegis-boot-{}.img", &hash[..16])
    } else {
        // Derive a stable suffix from the URL's last path component so
        // distinct URLs don't collide in the cache.
        let suffix = url.rsplit('/').next().unwrap_or("aegis-boot.img");
        let suffix = suffix.split('?').next().unwrap_or(suffix);
        let sanitized: String = suffix
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
            .collect();
        if sanitized.is_empty() {
            "aegis-boot.img".to_string()
        } else {
            sanitized
        }
    };
    p.push(basename);
    Ok(p)
}

fn download_via_curl(url: &str, out: &Path) -> Result<(), u8> {
    // -fsSL: fail on HTTP errors, silent (we print our own progress hint),
    //        show errors, follow redirects.
    // --proto =https: refuse anything not https (defense in depth).
    // --tlsv1.2: minimum TLS version.
    let status = Command::new("curl")
        .args([
            "--fail",
            "--silent",
            "--show-error",
            "--location",
            "--proto",
            "=https",
            "--tlsv1.2",
            "--output",
            &out.display().to_string(),
            url,
        ])
        .status()
        .map_err(|e| {
            eprintln!("aegis-boot fetch-image: cannot run curl: {e}. Is curl installed?");
            1_u8
        })?;
    if !status.success() {
        eprintln!("aegis-boot fetch-image: curl exited with {status}");
        return Err(1);
    }
    Ok(())
}

/// Compute sha256 by shelling out to `sha256sum` (Linux/macOS GNU
/// coreutils) — already a host dep used by other aegis-boot
/// subcommands. Returns the lowercase hex hash.
fn compute_sha256(path: &Path) -> Result<String, u8> {
    let output = Command::new("sha256sum").arg(path).output().map_err(|e| {
        eprintln!("aegis-boot fetch-image: cannot run sha256sum: {e}");
        1_u8
    })?;
    if !output.status.success() {
        eprintln!(
            "aegis-boot fetch-image: sha256sum failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
        return Err(1);
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    // Output format: "<64hex>  <path>\n"
    let hash = stdout.split_whitespace().next().unwrap_or("").to_string();
    if !is_valid_sha256_hex(&hash) {
        eprintln!("aegis-boot fetch-image: unexpected sha256sum output: {stdout}");
        return Err(1);
    }
    Ok(hash)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn is_safe_https_url_accepts_plain_https() {
        assert!(is_safe_https_url("https://example.com/aegis-boot.img"));
        assert!(is_safe_https_url(
            "https://github.com/williamzujkowski/aegis-boot/releases/download/v0.13.0/aegis-boot.img"
        ));
    }

    #[test]
    fn is_safe_https_url_rejects_http_and_file_and_others() {
        assert!(!is_safe_https_url("http://example.com/aegis-boot.img"));
        assert!(!is_safe_https_url("file:///etc/passwd"));
        assert!(!is_safe_https_url("ftp://example.com/img"));
        assert!(!is_safe_https_url("javascript:alert(1)"));
    }

    #[test]
    fn is_safe_https_url_rejects_control_chars() {
        assert!(!is_safe_https_url("https://example.com/img\0"));
        assert!(!is_safe_https_url("https://example.com/img\n"));
        assert!(!is_safe_https_url("https://example.com/img\r"));
    }

    #[test]
    fn is_valid_sha256_hex_accepts_64_hex() {
        let h = "abcdef0123456789".repeat(4);
        assert_eq!(h.len(), 64);
        assert!(is_valid_sha256_hex(&h));
        // Uppercase is fine (we eq_ignore_ascii_case on compare).
        assert!(is_valid_sha256_hex(&h.to_uppercase()));
    }

    #[test]
    fn is_valid_sha256_hex_rejects_wrong_lengths_and_chars() {
        assert!(!is_valid_sha256_hex(""));
        assert!(!is_valid_sha256_hex(&"a".repeat(63)));
        assert!(!is_valid_sha256_hex(&"a".repeat(65)));
        assert!(!is_valid_sha256_hex(&"g".repeat(64))); // g is not hex
        assert!(!is_valid_sha256_hex(&format!("{}.", "a".repeat(63))));
    }

    #[test]
    fn parse_args_requires_url_at_runtime_not_at_parse() {
        // parse_args succeeds without --url; try_run is the gate.
        let args: Vec<String> = vec![];
        let p = parse_args(&args).unwrap();
        assert!(p.url.is_none());
    }

    #[test]
    fn parse_args_handles_equals_form() {
        let args = vec![
            "--url=https://example.com/img".to_string(),
            "--sha256=".to_string() + &"a".repeat(64),
        ];
        let p = parse_args(&args).unwrap();
        assert_eq!(p.url.as_deref(), Some("https://example.com/img"));
        assert!(p.expected_sha256.is_some());
    }

    #[test]
    fn parse_args_rejects_short_sha256() {
        let args = vec![
            "--url".to_string(),
            "https://example.com/img".to_string(),
            "--sha256".to_string(),
            "tooshort".to_string(),
        ];
        let err = parse_args(&args).unwrap_err();
        assert_eq!(err, 2);
    }

    #[test]
    fn parse_args_rejects_unknown_option() {
        let args = vec!["--evil-flag".to_string()];
        let err = parse_args(&args).unwrap_err();
        assert_eq!(err, 2);
    }

    #[test]
    fn parse_args_help_is_recognized() {
        for h in ["-h", "--help"] {
            let p = parse_args(&[h.to_string()]).unwrap();
            assert!(p.help_requested);
        }
    }

    #[test]
    fn default_cache_path_uses_sha256_prefix_when_pinned() {
        let hash = "abcdef0123456789".repeat(4);
        let path = default_cache_path("https://example.com/img", Some(&hash)).unwrap();
        let basename = path.file_name().unwrap().to_string_lossy().to_string();
        assert!(
            basename.starts_with("aegis-boot-abcdef01"),
            "got {basename}"
        );
        assert!(std::path::Path::new(&basename)
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("img")));
    }

    #[test]
    fn default_cache_path_falls_back_to_sanitized_url_basename() {
        let path = default_cache_path(
            "https://example.com/path/aegis-boot-v1.2.img?token=abc",
            None,
        )
        .unwrap();
        let basename = path.file_name().unwrap().to_string_lossy().to_string();
        assert_eq!(basename, "aegis-boot-v1.2.img");
    }

    #[test]
    fn default_cache_path_handles_pathless_url() {
        let path = default_cache_path("https://example.com", None).unwrap();
        let basename = path.file_name().unwrap().to_string_lossy().to_string();
        // Our basename derivation: rsplit('/').next() of "https://example.com"
        // yields "example.com" — sanitized form is the same; that's a
        // reasonable default since there's no .img to extract.
        assert!(!basename.is_empty());
    }

    // ---- #235 PR1: cosign auto-verify plumbing --------------------------

    #[test]
    fn parse_args_accepts_no_cosign_flag() {
        let p = parse_args(&[
            "--url".to_string(),
            "https://example.com/x.img".to_string(),
            "--no-cosign".to_string(),
        ])
        .unwrap();
        assert!(p.cosign_disabled);
    }

    #[test]
    fn parse_args_default_leaves_cosign_enabled() {
        // The operator who passes --url without touching cosign flags
        // gets auto-verification. This is the "most automatic"
        // contract #235 committed to.
        let p =
            parse_args(&["--url".to_string(), "https://example.com/x.img".to_string()]).unwrap();
        assert!(!p.cosign_disabled);
    }

    #[test]
    fn sibling_with_suffix_appends_to_basename() {
        let p = sibling_with_suffix(std::path::Path::new("/tmp/aegis-boot.img"), ".sig");
        assert_eq!(p, std::path::PathBuf::from("/tmp/aegis-boot.img.sig"));
        let p = sibling_with_suffix(std::path::Path::new("/tmp/aegis-boot.img"), ".pem");
        assert_eq!(p, std::path::PathBuf::from("/tmp/aegis-boot.img.pem"));
    }

    #[test]
    fn sibling_with_suffix_handles_pathless_basename() {
        // Defensive: the function should append even when the path
        // has no parent directory.
        let p = sibling_with_suffix(std::path::Path::new("x.img"), ".sig");
        assert_eq!(p, std::path::PathBuf::from("x.img.sig"));
    }

    #[test]
    fn cosign_identity_regex_shape_is_locked() {
        // Regression guard: the identity regex is a security-critical
        // constant. A drift that widens it (e.g. drops the workflow
        // anchor) would let a non-release workflow sign artifacts that
        // this CLI would accept. Spot-check the anchor + path shape.
        assert!(
            COSIGN_IDENTITY_REGEX.starts_with("^https://github\\.com/williamzujkowski/aegis-boot/")
        );
        assert!(COSIGN_IDENTITY_REGEX.contains(".github/workflows/release\\.yml"));
        assert!(COSIGN_IDENTITY_REGEX.contains("refs/tags/"));
        assert!(COSIGN_IDENTITY_REGEX.ends_with(".+$"));
    }

    #[test]
    fn cosign_oidc_issuer_points_at_github_actions() {
        assert_eq!(
            COSIGN_OIDC_ISSUER,
            "https://token.actions.githubusercontent.com"
        );
    }
}
