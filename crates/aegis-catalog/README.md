# `aegis-catalog`

Public API façade for the aegis-boot ISO catalog. As of [#701](https://github.com/aegis-boot/aegis-boot/issues/701) Phase 2, this crate is a thin re-export shim over [`aegis-catalog-data`](https://github.com/aegis-boot/aegis-catalog-data) — the actual `Entry` list, vendor PGP keyring, and per-distro URL resolvers live in that sibling repo and are consumed via a Cargo `git+tag` dependency.

## Why two crates

The data side (50+ vendor PGP keys, 14 → eventual 200 ISO entries, per-distro resolver functions) churns on a different cadence than the API surface. Phase 2 of #701 moved `aegis-catalog-data` to a sibling git repo (`aegis-boot/aegis-catalog-data`) so a catalog refresh doesn't require an aegis-boot release — bumping the `tag` value in `crates/aegis-catalog/Cargo.toml` is now the only change needed.

This shim crate stays in the aegis-boot main repo with the rest of the workspace. Consumers (`aegis-cli`, `aegis-fetch`, `rescue-tui`) import from `aegis_catalog::*` regardless of where the data physically lives.

## Imports

```rust
use aegis_catalog::{Entry, Vendor, SbStatus, SigPattern, find_entry, CATALOG};
```

Same as before. No call-site changes were required when this crate moved to a re-export shim.

## When to depend on `aegis-catalog-data` directly

Practically never. The shim re-exports everything `aegis-catalog-data` makes public. Going around the shim would tie a consumer to a specific catalog-data revision and skip the version pin held in `crates/aegis-catalog/Cargo.toml` — defeating the single-point-of-update promise of the façade.
