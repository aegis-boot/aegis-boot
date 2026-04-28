# Third-Party Licenses

aegis-boot ships under the dual `MIT OR Apache-2.0` outbound
license. This file inventories notable third-party dependencies
that contribute distinctive licenses or that warrant explicit
attribution.

The full transitive license graph is enforced by `cargo-deny`
against the allow-list in `deny.toml`. CI fails the build on any
crate whose license is not in that allow-list. This document
covers the human-readable rationale for each non-permissive
license and for the cryptographic-trust-path crates.

## Cryptographic trust path (#655 aegis-fetch)

The `aegis-fetch` crate downloads + verifies catalog ISOs against
vendor PGP signatures. Its dependencies include:

| Crate           | License           | Notes                                                                                     |
| --------------- | ----------------- | ----------------------------------------------------------------------------------------- |
| `pgp` (rpgp)    | MIT OR Apache-2.0 | Pure-Rust OpenPGP implementation. Verify-only consumer; we never sign or encrypt with it. |
| `ureq`          | MIT OR Apache-2.0 | HTTPS transport with `https_only(true)` enforcement.                                      |
| `rustls`        | Apache-2.0 OR ISC OR MIT | TLS 1.3 implementation. Pulled in via ureq's `rustls` feature.                      |
| `ring`          | Apache-2.0 + ISC + OpenSSL  | Crypto primitives backing rustls. Mixed-license; the Apache-2.0 portion is the dominant component for our usage. |
| `webpki-roots`  | (code) ISC; (data) CDLA-Permissive-2.0 | Mozilla CA root certificate bundle. The data-license split is intentional upstream — the cert bundle data is published under the Linux Foundation's permissive data license; the crate code itself remains ISC. |
| `sha2`          | MIT OR Apache-2.0 | SHA-256 of the downloaded ISO.                                                            |
| `webpki`        | ISC               | X.509 path validation used by rustls.                                                     |

We picked `pgp` (rpgp) over `sequoia-openpgp` for licensing
compatibility: sequoia is `LGPL-2.0-or-later`, which `deny.toml`
explicitly excludes from the workspace's allow-list (LGPL §6
imposes "object file" disclosure obligations on every static-musl
rescue-tui binary we ship). rpgp's `MIT OR Apache-2.0` is the
clean fit. See `crates/aegis-fetch/README.md` for the API
contract.

## Allow-listed licenses (deny.toml)

The full allow-list is in `deny.toml`. Inclusion criteria:
permissive, statically-link-friendly, and either OSI-approved or
documented above with a rationale.

## Updates

When a new third-party dependency is added that ships under a
license not already in `deny.toml`, the PR adding the dependency
must also update both `deny.toml` (the enforcement) and this
file (the human-readable rationale). The CI license gate fails
the build until the policy update lands.
