# Attestation Manifest — Wire-Format Contract

This doc captures the schema aegis-boot writes to the ESP at flash time and that downstream tools — notably the [`aegis-hwsim`](https://github.com/aegis-boot/aegis-hwsim) E6 attestation roundtrip scenario — read back. The schema is **already pinned** in `crates/aegis-wire-formats/src/lib.rs`; this doc exists so external consumers don't have to read Rust to know what they're committing to.

## Two contracts, one repo

aegis-boot maintains two independent attestation contracts. They have separate `schema_version` fields and bump independently.

| Contract | What | Where it lives |
|---|---|---|
| **`Manifest`** (this doc) | Signed wire format on the ESP. Boot-time verifier reads this. | `<ESP>/EFI/aegis-boot/manifest.json` + `manifest.json.minisig` |
| `Attestation` | Host-side audit-trail JSON. Operator-side `aegis-boot attest list` reads this. | `$XDG_DATA_HOME/aegis-boot/attestations/<guid>-<ts>.json` |

`aegis-hwsim` E6 consumes the on-ESP `Manifest`. The host-side `Attestation` is documented separately in `crates/aegis-wire-formats/src/lib.rs::Attestation` and is not part of the boot-time roundtrip.

## Top-level shape

```json
{
  "schema_version": 1,
  "tool_version": "aegis-boot 0.17.0",
  "manifest_sequence": 7,
  "device": { ... },
  "esp_files": [ ... ],
  "allowed_files_closed_set": true,
  "expected_pcrs": []
}
```

### Field guarantees aegis-hwsim can stably depend on

- **`schema_version` (u32)**: top-level integer. Currently `1`. Bumped only on breaking shape changes (field removal, type change). Adding a new optional field is **backwards-compatible** and does NOT bump this — verifiers ignore unknown fields. Pin assertions on `schema_version >= N`, not `==`.
- **`tool_version` (string)**: `"aegis-boot <semver>"` as the binary's `--version` would print. Informational; not a trust anchor.
- **`manifest_sequence` (u64)**: monotonic-per-flash. Rollback defense — verifiers track the highest sequence seen for a given device fingerprint and reject manifests where `sequence < seen`. Wire field name is `manifest_sequence` (not `sequence`); the Rust struct field is `sequence` via `#[serde(rename)]`.
- **`device` (object)**: GPT identity captured at flash time — disk GUID, partition count, ESP+data partition PARTUUIDs / type GUIDs / FS UUIDs / LBAs / labels. Verifier re-reads from runtime `blkid`+`sgdisk -p` and asserts equality. Full schema in `aegis-wire-formats::Device`.
- **`esp_files` (array of object)**: closed set of files on the ESP. Each entry is `{ "path": "::/EFI/...", "sha256": "<lowercase hex>", "size_bytes": <u64> }`. Verifier rejects the stick if any ESP file is unlisted or has a different sha256.
- **`allowed_files_closed_set` (bool)**: when `true`, `esp_files` is exhaustive (any extra ESP file violates). Always `true` in PR3; left as a field so a future "extended" manifest can ship without breaking consumers.
- **`expected_pcrs` (array of object)**: see below.

## `expected_pcrs` — the E6 contract

```json
"expected_pcrs": [
  { "pcr_index": 12, "bank": "sha256", "digest_hex": "abc123..." },
  { "pcr_index":  7, "bank": "sha256", "digest_hex": "def456..." }
]
```

**Shape**: `Vec<PcrEntry>` where each entry is

```rust
struct PcrEntry {
    pub pcr_index: u32,    // 0..23 for most banks
    pub bank: String,       // "sha256", "sha384", etc.
    pub digest_hex: String, // lowercase hex
}
```

**Why a `Vec<PcrEntry>` and not `{"12": "sha256:abc..."}`**: an array of typed records lets us:

1. Carry multiple banks for the same PCR index (a future SHA-384 vs. SHA-256 split).
2. Add new fields on each entry (e.g. an `algorithm_uri`) without remapping the parent shape.
3. Stay in `serde_json::to_vec` canonical form, which the manifest signature is computed over.

A consumer asserting "PCR 12 sha256 is X" should iterate:

```rust
manifest.expected_pcrs
    .iter()
    .find(|e| e.pcr_index == 12 && e.bank == "sha256")
    .map(|e| &e.digest_hex)
```

**Current population status**: `expected_pcrs` is **always empty** in shipped releases through `0.17.x` (PR3-era). aegis-boot does not select PCRs at flash time yet; that selection lands when E6 maps which PCRs are stable across the boot chain we control. Until then, the field is `[]` and a verifier checking `expected_pcrs[N]` will get `None` for every `N` — that's correct behaviour, not a regression.

**E6 forward compatibility**: when `expected_pcrs` starts being populated, **the schema_version stays at 1** (additive change — empty → non-empty doesn't break parsers). Consumers that fail-open on missing entries (the recommended posture) keep working unchanged. Consumers that want to assert "this manifest WAS produced by an E6-aware tool" should pin on `tool_version >= "aegis-boot 0.18.0"` or whatever release lands the populated field.

## Bump policy

- **Adding a field to the top-level body or to any nested struct**: backwards-compatible. Verifiers ignore unknown fields. No `schema_version` bump.
- **Removing a field**: breaking. Bump `SCHEMA_VERSION` in `aegis-wire-formats/src/lib.rs` AND coordinate a paired PR on every consuming repo (aegis-hwsim included) AND call out the change in CHANGELOG.md.
- **Changing a field's type or wire name**: breaking. Same bump + coordinate posture.
- **Changing the meaning of an existing field while keeping the type**: also breaking. Same bump posture.

The CI gate that catches accidental breakage: `crates/aegis-wire-formats/tests/...` round-trip + golden-file tests pin the wire shape. Any PR that touches `Manifest`, `PcrEntry`, etc. without updating the goldens fails CI.

## How to consume from Rust

Reuse the published types:

```toml
[dependencies]
aegis-wire-formats = { git = "https://github.com/aegis-boot/aegis-boot", tag = "v0.17.0" }
```

```rust
use aegis_wire_formats::{Manifest, SCHEMA_VERSION};

let manifest: Manifest = serde_json::from_slice(&bytes)?;
assert!(manifest.schema_version >= SCHEMA_VERSION);
```

The crate is `MIT OR Apache-2.0` and the only public deps are `serde` + `serde_json` + (feature-gated) `schemars`. Pulling it into `aegis-hwsim` is the recommended path; reimplementing the structs is brittle.

## How to consume without depending on aegis-boot

If pulling the crate isn't desirable, the JSON Schema is auto-generated at build time:

```
docs/reference/schemas/aegis-boot-manifest.schema.json
```

Generated from `Manifest::json_schema()` via the `schema` feature flag on `aegis-wire-formats`. CI fails any PR that touches the wire types without regenerating. Treat the file as canonical.

## References

- **In-code source**: [`crates/aegis-wire-formats/src/lib.rs`](../crates/aegis-wire-formats/src/lib.rs) — `Manifest`, `PcrEntry`, `SCHEMA_VERSION`.
- **JSON Schema**: [`docs/reference/schemas/aegis-boot-manifest.schema.json`](reference/schemas/aegis-boot-manifest.schema.json).
- **aegis-hwsim E6 epic**: <https://github.com/aegis-boot/aegis-hwsim/issues/6>.
- **Architect review constraint**: aegis-boot#226.
- **Discussion thread**: aegis-boot#677.
