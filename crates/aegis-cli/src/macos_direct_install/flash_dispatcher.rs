// SPDX-License-Identifier: MIT OR Apache-2.0

//! [#418] Phase 4c — macOS `aegis-boot flash --direct-install`
//! dispatcher. Composes device-ID parsing + source resolution +
//! the pipeline composer into the operator-facing CLI entry path.
//!
//! ## Operator flow
//!
//! ```text
//! aegis-boot flash --direct-install /dev/disk5 --out-dir ./out
//! ```
//!
//! - Device argument accepts `disk5`, `/dev/disk5`, or `/dev/rdisk5`
//!   — whichever form `diskutil list` printed for the operator.
//! - `--out-dir` defaults to `./out` (matches Linux + Windows
//!   paths); contains the 6 signed-chain files the source-resolution
//!   module looks for.
//!
//! Unlike Windows' dispatcher, this one doesn't print an interactive
//! candidate list when the device argument is missing — the macOS
//! operator UX for flash targets is `diskutil list` plus
//! copy/paste, and a second candidate list from aegis-boot would
//! just reproduce that out of date. Missing argument is a usage
//! error.
//!
//! ## Why a separate module from `flash.rs`
//!
//! Same rationale as [`crate::windows_direct_install::flash_dispatcher`]
//! — host-specific composition stays co-located with its phase
//! modules, leaving `flash.rs` as the thin dispatcher that picks a
//! platform implementation.
//!
//! [#418]: https://github.com/aegis-boot/aegis-boot/issues/418

#![allow(dead_code)]

use std::fmt::Write as _;
use std::path::Path;
use std::time::Duration;

#[cfg(target_os = "macos")]
use crate::macos_direct_install::pipeline::MacosPhaseRunner;
use crate::macos_direct_install::pipeline::{
    DirectInstallError, DirectInstallPlan, DirectInstallReceipt, DirectInstallStage, PhaseRunner,
    run,
};
use crate::windows_direct_install::source_resolution::{
    SourceResolutionError, build_staging_sources,
};

/// Parse an operator-supplied device argument into the bare
/// `disk<N>` identifier aegis-boot's partition + preflight modules
/// accept. Accepts:
///
/// - `"disk5"` — bare identifier
/// - `"/dev/disk5"` — full block-device path
/// - `"/dev/rdisk5"` — raw character-device path (what `dd` wants);
///   macOS's block-vs-raw distinction is pure convenience here
///
/// # Errors
///
/// Returns a descriptive `String` if `raw` doesn't match any of the
/// accepted shapes. The message is operator-facing — it names every
/// form aegis-boot accepts.
pub(crate) fn parse_device_id(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("aegis-boot flash: device argument is empty".into());
    }
    let stem = trimmed
        .strip_prefix("/dev/rdisk")
        .or_else(|| trimmed.strip_prefix("/dev/disk"))
        .or_else(|| trimmed.strip_prefix("disk"))
        .ok_or_else(|| {
            format!(
                "aegis-boot flash: can't parse device identifier {raw:?} as a macOS disk. \
                 Expected `disk5`, `/dev/disk5`, or `/dev/rdisk5`."
            )
        })?;
    if stem.is_empty() || !stem.bytes().all(|b| b.is_ascii_digit()) {
        return Err(format!(
            "aegis-boot flash: device identifier {raw:?} has a non-numeric suffix. \
             Pass the parent whole-disk id (e.g. `disk5`), not a partition slice (`disk5s1`)."
        ));
    }
    Ok(format!("disk{stem}"))
}

/// Top-level dispatch error: either the plan couldn't be built
/// (missing argument, bad device arg, source files missing) or the
/// pipeline ran and aborted at one of its stages.
#[derive(Debug)]
pub(crate) enum DispatchError {
    /// `explicit_dev` was `None`. Unlike the Windows dispatcher we
    /// don't print an interactive candidate list — macOS operators
    /// use `diskutil list` to find their target.
    MissingDeviceArg,
    /// Device argument supplied but unparsable.
    BadDeviceArg(String),
    /// Source resolution failed (missing files, etc).
    Sources(SourceResolutionError),
    /// Pipeline aborted at one of its stages; receipt reports which
    /// stages actually ran.
    Pipeline(Box<(DirectInstallError, DirectInstallReceipt)>),
}

impl std::fmt::Display for DispatchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingDeviceArg => write!(
                f,
                "aegis-boot flash --direct-install: no device specified. \
                 Find your USB target via `diskutil list` and pass its `diskN` \
                 identifier (e.g. `aegis-boot flash --direct-install disk5`)."
            ),
            Self::BadDeviceArg(detail) => write!(f, "{detail}"),
            Self::Sources(e) => write!(f, "{e}"),
            Self::Pipeline(err_and_receipt) => {
                let (err, receipt) = err_and_receipt.as_ref();
                writeln!(f, "aegis-boot flash --direct-install: {err}")?;
                write!(f, "{}", format_receipt(receipt))
            }
        }
    }
}

/// Format the partial-progress receipt into a multi-line
/// `Stage N done in XXXms` report matching the Linux + Windows
/// flash paths' ending lines.
pub(crate) fn format_receipt(r: &DirectInstallReceipt) -> String {
    let mut out = String::new();
    let mut push = |stage: DirectInstallStage, d: Option<Duration>| {
        if let Some(dur) = d {
            let _ = writeln!(out, "  {:<24} {}", stage.name(), format_elapsed(dur));
        }
    };
    push(DirectInstallStage::Preflight, r.preflight);
    push(DirectInstallStage::Partition, r.partition);
    push(DirectInstallStage::StageEsp, r.stage_esp);
    push(DirectInstallStage::Unmount, r.unmount);
    let _ = writeln!(out, "  {:<24} {}", "total", format_elapsed(r.total()));
    out
}

/// Human-readable elapsed time. Matches the Windows dispatcher's
/// private formatter so per-platform output reads the same.
fn format_elapsed(d: Duration) -> String {
    let secs = d.as_secs_f64();
    if secs < 60.0 {
        format!("{secs:.1}s")
    } else {
        let mins = d.as_secs() / 60;
        let remaining = d.as_secs() % 60;
        format!("{mins}m {remaining:02}s")
    }
}

/// Shared pure core — runs the dispatcher against an injected
/// [`PhaseRunner`] so tests don't need `diskutil`. The real
/// [`run_direct_install`] on macOS supplies [`MacosPhaseRunner`].
pub(crate) fn run_direct_install_using(
    explicit_dev: Option<&str>,
    out_dir: &Path,
    runner: &dyn PhaseRunner,
) -> Result<DirectInstallReceipt, DispatchError> {
    let raw = explicit_dev.ok_or(DispatchError::MissingDeviceArg)?;
    let device_id = parse_device_id(raw).map_err(DispatchError::BadDeviceArg)?;
    let sources = build_staging_sources(out_dir).map_err(DispatchError::Sources)?;
    let plan = DirectInstallPlan { device_id, sources };
    run(runner, &plan).map_err(DispatchError::Pipeline)
}

/// Top-level entry for the macOS-only CLI dispatch. Compiles only
/// on macOS; the Linux + Windows builds skip this entirely via the
/// cfg gate on the caller in `flash.rs`.
#[cfg(target_os = "macos")]
pub(crate) fn run_direct_install(
    explicit_dev: Option<&str>,
    out_dir: &Path,
) -> Result<DirectInstallReceipt, DispatchError> {
    run_direct_install_using(explicit_dev, out_dir, &MacosPhaseRunner)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::macos_direct_install::esp_stage::CopyPlan;
    use crate::macos_direct_install::partition::DiskutilPartitionPlan;
    use crate::macos_direct_install::preflight::DiskInfo;
    use std::cell::RefCell;

    struct MockRunner {
        calls: RefCell<Vec<String>>,
    }

    impl MockRunner {
        fn new() -> Self {
            Self {
                calls: RefCell::new(Vec::new()),
            }
        }

        fn calls(&self) -> Vec<String> {
            self.calls.borrow().clone()
        }
    }

    impl PhaseRunner for MockRunner {
        fn preflight(&self, device_id: &str) -> Result<DiskInfo, String> {
            self.calls
                .borrow_mut()
                .push(format!("preflight({device_id})"));
            Ok(DiskInfo {
                device_id: device_id.to_string(),
                whole_disk: true,
                removable: true,
                external: true,
                size_bytes: 16 * 1024 * 1024 * 1024,
                media_name: "Test Media".to_string(),
            })
        }
        fn partition(&self, plan: &DiskutilPartitionPlan) -> Result<(), String> {
            self.calls
                .borrow_mut()
                .push(format!("partition({})", plan.device_id));
            Ok(())
        }
        fn stage_esp(&self, plan: &CopyPlan) -> Result<(), String> {
            self.calls
                .borrow_mut()
                .push(format!("stage_esp(files={})", plan.copies.len()));
            Ok(())
        }
        fn unmount_esp(&self, mount_point: &std::path::Path) -> Result<(), String> {
            self.calls
                .borrow_mut()
                .push(format!("unmount_esp({})", mount_point.display()));
            Ok(())
        }
    }

    fn write_six_sources(dir: &Path) {
        use std::fs;
        for name in [
            "shimx64.efi.signed",
            "grubx64.efi.signed",
            "mmx64.efi.signed",
            "grub.cfg",
            "vmlinuz",
            "initramfs.cpio.gz",
        ] {
            fs::write(dir.join(name), b"x").unwrap();
        }
    }

    #[test]
    fn parse_device_id_accepts_bare_disk() {
        assert_eq!(parse_device_id("disk5").unwrap(), "disk5");
    }

    #[test]
    fn parse_device_id_strips_dev_prefix() {
        assert_eq!(parse_device_id("/dev/disk5").unwrap(), "disk5");
    }

    #[test]
    fn parse_device_id_strips_rdisk_prefix() {
        assert_eq!(parse_device_id("/dev/rdisk5").unwrap(), "disk5");
    }

    #[test]
    fn parse_device_id_trims_whitespace() {
        assert_eq!(parse_device_id("  disk2  ").unwrap(), "disk2");
    }

    #[test]
    fn parse_device_id_rejects_empty() {
        let err = parse_device_id("").unwrap_err();
        assert!(err.contains("empty"));
    }

    #[test]
    fn parse_device_id_rejects_partition_suffix() {
        let err = parse_device_id("disk5s1").unwrap_err();
        assert!(err.contains("partition slice"));
    }

    #[test]
    fn parse_device_id_rejects_unknown_form() {
        let err = parse_device_id("sda1").unwrap_err();
        assert!(err.contains("macOS disk"));
    }

    #[test]
    fn missing_device_arg_surfaces_dispatch_error() {
        let mock = MockRunner::new();
        let tmp = tempfile::tempdir().unwrap();
        let err = run_direct_install_using(None, tmp.path(), &mock).unwrap_err();
        assert!(matches!(err, DispatchError::MissingDeviceArg));
        // No pipeline stages ran — missing device is caught before
        // source resolution or anything downstream.
        assert_eq!(mock.calls().len(), 0);
    }

    #[test]
    fn missing_device_arg_display_points_at_diskutil_list() {
        let s = DispatchError::MissingDeviceArg.to_string();
        assert!(s.contains("diskutil list"));
        assert!(s.contains("diskN"));
    }

    #[test]
    fn bad_device_arg_surfaces_dispatch_error_with_detail() {
        let mock = MockRunner::new();
        let tmp = tempfile::tempdir().unwrap();
        let err = run_direct_install_using(Some("wat"), tmp.path(), &mock).unwrap_err();
        match err {
            DispatchError::BadDeviceArg(detail) => {
                assert!(detail.contains("wat"));
            }
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[test]
    fn source_resolution_error_surfaces_before_pipeline() {
        // Empty out_dir — the 6 signed-chain files aren't there,
        // so source resolution fails and pipeline never runs.
        let mock = MockRunner::new();
        let tmp = tempfile::tempdir().unwrap();
        let err = run_direct_install_using(Some("disk5"), tmp.path(), &mock).unwrap_err();
        assert!(matches!(err, DispatchError::Sources(_)));
        assert_eq!(mock.calls().len(), 0, "pipeline must not run");
    }

    #[test]
    fn happy_path_runs_pipeline_with_parsed_device_id() {
        let tmp = tempfile::tempdir().unwrap();
        write_six_sources(tmp.path());
        let mock = MockRunner::new();
        let receipt = run_direct_install_using(Some("/dev/disk5"), tmp.path(), &mock)
            .expect("happy path should succeed");
        assert!(receipt.preflight.is_some());

        let calls = mock.calls();
        assert_eq!(calls.len(), 4);
        // Device id was normalized to bare `disk5` — the dispatcher
        // stripped the /dev/ prefix before handing to the pipeline.
        assert_eq!(calls[0], "preflight(disk5)");
        assert_eq!(calls[1], "partition(disk5)");
        assert!(calls[2].starts_with("stage_esp(files="));
        assert!(calls[3].starts_with("unmount_esp("));
    }

    #[test]
    fn format_receipt_includes_every_completed_stage() {
        let r = DirectInstallReceipt {
            preflight: Some(Duration::from_millis(40)),
            partition: Some(Duration::from_millis(3200)),
            stage_esp: Some(Duration::from_millis(180)),
            unmount: Some(Duration::from_millis(60)),
        };
        let s = format_receipt(&r);
        for needle in [
            "preflight:diskutil_info",
            "partition:diskutil",
            "stage_esp:cp+sync",
            "unmount:diskutil",
            "total",
        ] {
            assert!(s.contains(needle), "receipt missing {needle}: {s}");
        }
    }

    #[test]
    fn format_receipt_skips_unrun_stages() {
        let r = DirectInstallReceipt {
            preflight: Some(Duration::from_millis(40)),
            partition: Some(Duration::from_millis(3200)),
            stage_esp: None,
            unmount: None,
        };
        let s = format_receipt(&r);
        assert!(s.contains("preflight:diskutil_info"));
        assert!(s.contains("partition:diskutil"));
        assert!(!s.contains("stage_esp:cp+sync"));
        assert!(!s.contains("unmount:diskutil"));
    }

    #[test]
    fn format_elapsed_uses_seconds_under_minute() {
        assert_eq!(format_elapsed(Duration::from_millis(3200)), "3.2s");
        assert_eq!(format_elapsed(Duration::from_secs(59)), "59.0s");
    }

    #[test]
    fn format_elapsed_switches_to_minutes_past_sixty_seconds() {
        let s = format_elapsed(Duration::from_secs(125));
        assert_eq!(s, "2m 05s");
    }
}
