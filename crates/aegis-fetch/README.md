# aegis-fetch

HTTPS download + signed-chain verification for aegis-boot catalog ISOs.
Shared by the host CLI (`aegis-boot fetch <slug>`) and the rescue-tui's
in-rescue Catalog screen (#655 Phase 2B).

## Trust model

Each fetch performs three steps:

1. HTTPS GET the ISO + its signature artifacts from `aegis_catalog::Entry`.
2. Verify the OpenPGP signature against the pinned vendor cert in
   `crates/aegis-catalog/keyring/<vendor>.asc` (fingerprint pinned in
   `fingerprints.toml`).
3. SHA-256 the ISO bytes and match against the (now authenticated)
   sums file.

Three signature shapes are dispatched on `aegis_catalog::SigPattern`:

- `ClearsignedSums` — sums file is a PGP cleartext envelope; signature
  authenticates the inline checksum lines (AlmaLinux, Fedora, Rocky).
- `DetachedSigOnSums` — `.gpg` / `.sign` / `.asc` separate file
  authenticates the sums file, sums then authenticates the ISO
  (Debian, Ubuntu, Kali, Linux Mint, GParted, openSUSE, Pop!_OS).
- `DetachedSigOnIso` — sig directly authenticates the ISO bytes
  (Alpine, Manjaro, MX Linux, SystemRescue).

## Public API

```rust
use aegis_fetch::{fetch_catalog_entry, FetchEvent, VendorKeyring};

let keyring = VendorKeyring::embedded()?;
let outcome = fetch_catalog_entry(
    entry,
    &dest_dir,
    &keyring,
    &mut |event| match event {
        FetchEvent::Downloading(p) => render_progress(p),
        FetchEvent::VerifyingSig => println!("verifying signature..."),
        _ => {}
    },
)?;
println!("verified: {}", outcome.iso_path.display());
```

## License

MIT OR Apache-2.0. See `THIRD_PARTY_LICENSES.md` at the workspace
root for the licenses of bundled crypto dependencies.
