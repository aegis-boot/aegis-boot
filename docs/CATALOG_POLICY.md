# Catalog curation policy

The `aegis-boot recommend` catalog ships hardcoded ISO entries operators can browse and `aegis-boot fetch` directly. This file documents what gets added, what doesn't, and why — so the catalog stays principled instead of accumulating ad-hoc entries.

## Inclusion criteria

An entry must satisfy **all** of these to be added:

1. **HTTPS-served** canonical download URL hosted by the upstream project (or a project-blessed mirror — distrowatch, sourceforge, fosshub OK if signed; random forum re-uploads NOT OK).
2. **Published signed checksums** — either GPG-signed `SHA256SUMS` or minisign / ssh-key-signed equivalent. The signature URL goes in the entry; aegis-boot's trust anchor is the project's release-signing key, not our catalog. A project that publishes only an unsigned SHA-256 file fails this test.
3. **Operator value** — solves a real problem for someone who already has aegis-boot. Generic "I want to install X" goes through the distro's normal channels; the catalog is for ISOs that benefit from being on a signed-USB rescue stick.
4. **Reasonably stable URL** — distros that rotate URLs every point release without a `/latest/` symlink are skipped; we don't want the catalog rotting.
5. **Secure Boot posture stated honestly** — `signed (CA)`, `unsigned (MOK needed)`, or `unknown`. No fudging "unknown" to make an entry look better.

## Categories we accept

- **Major distros** (Ubuntu LTS, Fedora, Debian, Alpine, Arch, NixOS, openSUSE, RHEL-rebuilds)
- **Rescue / recovery** (SystemRescue, GParted Live, Clonezilla, Boot-Repair-Disk)
- **Diagnostics** (Memtest86+, Hiren's BootCD PE — when a signed source exists)
- **Privacy / forensic** (Tails, Kali, CAINE, Parrot Security)
- **Niche but signed** (special-purpose tools that ship with their own signing chain)

## Categories we deliberately skip

- **Unsigned re-spins** of mainstream distros (we don't want to be in the "trust this random ISO" business)
- **Vendor preinstall images** (OEM Windows / OEM Ubuntu — out of scope; operator should boot the OEM media)
- **Live Linux desktop variants** lacking signed checksums (e.g. some hobbyist remixes)
- **End-of-life releases** — we list current stable + LTS, not historical
- **Anything BSD** (different boot ecosystem, different trust model — separate epic if there's demand)

## How to propose an addition

Open a PR against `crates/aegis-cli/src/catalog.rs` adding an `Entry` to the `CATALOG` slice (alphabetically by slug). The PR description should:

1. Quote the criteria-1 download URL
2. Quote the criteria-2 signature URL
3. Cite the SB posture: where did you observe the kernel is signed/unsigned? (kernel boot log on real hardware, ENODATA from kexec_file_load, or the project's own statement)
4. Pick a stable kebab-case slug. Major-version segments allowed (`ubuntu-24.04-live-server`); point versions are NOT in the slug (they'd rot).
5. Update tests: bump `catalog_has_at_least_ten_entries`'s floor only if the new entries justify it (don't bump for a single PR — bump in a separate "raise floor" PR after several land).

Reviewer approves only if all 5 inclusion criteria are met. Reject explicitly + politely otherwise; pointer to this doc.

## Maintenance cadence

The catalog is intentionally small (target: 20-30 entries). Adding entries is cheap; removing them is rare.

We re-validate the URL set **once per major aegis-boot release** by:

1. `aegis-boot recommend` — eyeball the table for stale-looking entries
2. For each entry, `curl -sI <iso_url>` should return 200 (or a 30x to a current URL)
3. For each entry, `curl -sI <sha256_url>` should return 200
4. Anything broken: PR a fix or a removal

If a project fundamentally changes its trust posture (e.g. drops signed checksums), open a tracking issue + remove the entry until they add it back.

## Non-goals

- **Per-version pinning** — we don't list `ubuntu-24.04.2-live-server`; we list `ubuntu-24.04-live-server`. Point releases land in the upstream URL automatically.
- **Mirror selection** — operators on slow networks pick their own mirror. The catalog uses one canonical URL per entry.
- **Auto-updating from upstream catalogs** — distrowatch + GitHub releases APIs go stale; manual curation gives us veto power and a clear trust story.

## See also

- [docs/CLI.md `aegis-boot recommend`](./CLI.md#aegis-boot-recommend) — what operators see
- [docs/UNSIGNED_KERNEL.md](./UNSIGNED_KERNEL.md) — what `unsigned (MOK needed)` means
- `crates/aegis-cli/src/catalog.rs` — the canonical `CATALOG` slice
