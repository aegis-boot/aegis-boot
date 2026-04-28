// SPDX-License-Identifier: MIT OR Apache-2.0

//! HTTPS GET with progress callbacks.
//!
//! [`download_to_file`] streams a response into a file, calling
//! `on_event(Downloading)` per chunk. [`download_to_vec`] reads
//! the entire body into memory — used for SHA256SUMS / signature
//! sidecars whose payloads are KB-scale.
//!
//! ## Trust posture
//!
//! - `https_only(true)` — rejects `http://` URLs at the request
//!   level AND across the redirect chain. A vendor redirecting
//!   from HTTPS to HTTP fails the fetch loudly.
//! - `max_redirects(10)` — most distro mirrors redirect through
//!   1-3 hops (e.g., `download.fedoraproject.org` → mirror); 10
//!   is room for outliers without enabling a redirect-spam attack.
//! - Generous request timeout: ISO downloads on slow links can
//!   take many minutes. We set the connect timeout tight (30 s)
//!   but leave the body-read timeout generous (no fixed cap; the
//!   stall-detection comes from the missing-progress signal at
//!   the rescue-tui UI layer).
//! - Maximum body size is set to [`u64::MAX`] for the ISO path —
//!   ureq's default is a 10 MB cap that would truncate every
//!   real catalog ISO. The sidecar path keeps the default.

use std::fs::File;
use std::io::{BufWriter, Read, Write};
use std::path::Path;
use std::time::Duration;

use ureq::Agent;

use crate::{FetchError, FetchEvent, FetchProgress};

/// Identical 1 MiB chunk size as the SHA-256 hasher; keeps the
/// progress-callback cadence consistent across phases.
const CHUNK_BYTES: usize = 1 << 20;

/// Build the HTTPS-only ureq agent used by both download paths.
fn build_agent() -> Agent {
    Agent::config_builder()
        .https_only(true)
        .max_redirects(10)
        .max_redirects_will_error(true)
        .timeout_connect(Some(Duration::from_secs(30)))
        .build()
        .into()
}

/// Stream `url` into `dest`, calling `on_event(Downloading(...))`
/// per chunk. Returns the total bytes written.
///
/// # Errors
///
/// - [`FetchError::Network`] for transport / non-2xx / redirect-
///   to-http failures.
/// - [`FetchError::Filesystem`] for create/write/sync errors on
///   `dest`. On error the partial file is left in place — caller
///   chooses whether to retain (Phase 3 resume) or remove.
pub(crate) fn download_to_file(
    url: &str,
    dest: &Path,
    on_event: &mut dyn FnMut(FetchEvent),
) -> Result<u64, FetchError> {
    on_event(FetchEvent::Connecting);
    let agent = build_agent();
    let mut resp = agent.get(url).call().map_err(|e| FetchError::Network {
        url: url.to_string(),
        detail: format!("{e}"),
    })?;
    let total = resp.body().content_length();
    let reader = resp.body_mut().with_config().limit(u64::MAX).reader();
    let file = File::create(dest).map_err(|e| FetchError::Filesystem {
        detail: format!("create {}: {e}", dest.display()),
    })?;
    let written = pump_to_writer(reader, BufWriter::new(file), total, on_event, dest)?;
    Ok(written)
}

/// Read `url`'s body into a `Vec<u8>`. Used for sums/signature
/// sidecars whose payloads are small. Caps the body at 4 MiB —
/// signed sums files are typically < 100 KB.
///
/// # Errors
///
/// [`FetchError::Network`] for transport / size-limit / non-2xx.
pub(crate) fn download_to_vec(url: &str) -> Result<Vec<u8>, FetchError> {
    let agent = build_agent();
    let mut resp = agent.get(url).call().map_err(|e| FetchError::Network {
        url: url.to_string(),
        detail: format!("{e}"),
    })?;
    resp.body_mut()
        .with_config()
        .limit(4 * 1024 * 1024)
        .read_to_vec()
        .map_err(|e| FetchError::Network {
            url: url.to_string(),
            detail: format!("read body: {e}"),
        })
}

/// Pump bytes from `reader` to `writer` in 1 MiB chunks, emitting
/// progress events. Hermetically testable — no network involved.
fn pump_to_writer<R: Read, W: Write>(
    mut reader: R,
    mut writer: W,
    total: Option<u64>,
    on_event: &mut dyn FnMut(FetchEvent),
    dest: &Path,
) -> Result<u64, FetchError> {
    let mut buf = vec![0u8; CHUNK_BYTES];
    let mut written: u64 = 0;
    loop {
        let n = reader.read(&mut buf).map_err(|e| FetchError::Network {
            url: dest.display().to_string(),
            detail: format!("read body: {e}"),
        })?;
        if n == 0 {
            break;
        }
        writer
            .write_all(&buf[..n])
            .map_err(|e| FetchError::Filesystem {
                detail: format!("write {}: {e}", dest.display()),
            })?;
        written += n as u64;
        on_event(FetchEvent::Downloading(FetchProgress {
            bytes: written,
            total,
        }));
    }
    writer.flush().map_err(|e| FetchError::Filesystem {
        detail: format!("flush {}: {e}", dest.display()),
    })?;
    Ok(written)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::unwrap_used)]
    use super::*;
    use std::io::Cursor;

    #[test]
    fn pump_writes_all_bytes_and_emits_progress() {
        let dir = tempfile::tempdir().expect("tempdir");
        let dest = dir.path().join("dl");
        // Three full chunks plus a partial.
        let payload: Vec<u8> = (0..(CHUNK_BYTES * 3 + 17))
            .map(|i| u8::try_from(i & 0xff).unwrap_or(0))
            .collect();
        let cursor = Cursor::new(payload.clone());
        let file = File::create(&dest).expect("create");
        let mut events: Vec<FetchEvent> = Vec::new();
        let mut on_event = |e: FetchEvent| events.push(e);
        let n = pump_to_writer(
            cursor,
            BufWriter::new(file),
            Some(payload.len() as u64),
            &mut on_event,
            &dest,
        )
        .expect("pump");
        assert_eq!(n, payload.len() as u64);
        // File on disk matches.
        let on_disk = std::fs::read(&dest).expect("read");
        assert_eq!(on_disk, payload);
        // Progress events: at least 4 (3 full + 1 partial).
        let downloading: Vec<&FetchProgress> = events
            .iter()
            .filter_map(|e| match e {
                FetchEvent::Downloading(p) => Some(p),
                _ => None,
            })
            .collect();
        assert!(
            downloading.len() >= 4,
            "expected >=4 progress events, got {}",
            downloading.len()
        );
        assert_eq!(downloading.last().expect("at least one").bytes, n);
        for p in &downloading {
            assert_eq!(p.total, Some(payload.len() as u64));
        }
        // Monotonic.
        for w in downloading.windows(2) {
            assert!(w[0].bytes <= w[1].bytes);
        }
    }

    #[test]
    fn pump_zero_bytes_emits_no_downloading_events() {
        let dir = tempfile::tempdir().expect("tempdir");
        let dest = dir.path().join("dl");
        let cursor: Cursor<Vec<u8>> = Cursor::new(Vec::new());
        let file = File::create(&dest).expect("create");
        let mut events: Vec<FetchEvent> = Vec::new();
        let mut on_event = |e: FetchEvent| events.push(e);
        let n = pump_to_writer(cursor, BufWriter::new(file), Some(0), &mut on_event, &dest)
            .expect("pump");
        assert_eq!(n, 0);
        let downloading = events
            .iter()
            .filter(|e| matches!(e, FetchEvent::Downloading(_)))
            .count();
        assert_eq!(downloading, 0);
    }

    #[test]
    fn pump_with_unknown_total_still_emits_progress() {
        // Chunked / streaming responses have no Content-Length;
        // progress events still fire with `total: None`.
        let dir = tempfile::tempdir().expect("tempdir");
        let dest = dir.path().join("dl");
        let payload: Vec<u8> = (0..CHUNK_BYTES + 5).map(|_| 0u8).collect();
        let cursor = Cursor::new(payload.clone());
        let file = File::create(&dest).expect("create");
        let mut downloading_seen = 0u32;
        let mut totals: Vec<Option<u64>> = Vec::new();
        let mut on_event = |e: FetchEvent| {
            if let FetchEvent::Downloading(p) = e {
                downloading_seen += 1;
                totals.push(p.total);
            }
        };
        let _ =
            pump_to_writer(cursor, BufWriter::new(file), None, &mut on_event, &dest).expect("pump");
        assert!(downloading_seen >= 2);
        assert!(totals.iter().all(Option::is_none));
    }
}
