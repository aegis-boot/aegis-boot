# Contributing to aegis-boot

Thanks for your interest. This is a small project with a sharp focus — a signed UEFI Secure Boot rescue environment that `kexec`s into operator-selected ISOs. Patches that move us toward that goal are welcome.

> **AI coding agents:** read [AGENTS.md](./AGENTS.md) first. Same workflow as below, distilled for agents plus security-critical guardrails (sensitive surfaces, common traps, when to escalate).

## Quickstart

```bash
git clone git@github.com:aegis-boot/aegis-boot.git
cd aegis-boot
cargo test --workspace               # run every unit + integration test
./scripts/dev-test.sh                # full 8-stage local CI
```

The exact test count drifts every release — `cargo test --workspace 2>&1 | grep 'test result:'` prints the current totals. CI ([.github/workflows/ci.yml](./.github/workflows/ci.yml)) is the authoritative merge gate; see [§CI gates](#ci-gates-your-pr-must-pass) below for the full list.

Prereqs are listed at the top of [`scripts/dev-test.sh`](./scripts/dev-test.sh) and in [`docs/LOCAL_TESTING.md`](./docs/LOCAL_TESTING.md).

For an opt-in pre-push hook that runs the fast subset of `tools/local-ci.sh` (~9s, cargo fmt + check + clippy + lib tests):

```bash
tools/install-hooks.sh           # install (idempotent)
tools/install-hooks.sh --status  # check whether installed
tools/install-hooks.sh --uninstall
```

Bypass the hook for a WIP push: `git push --no-verify`. The hook is opt-in by design — CI remains the merge gate, and some workflows (intentionally test-broken feature branches, doc-only WIP commits) want to push without the gate. The script refuses to clobber a custom `.git/hooks/pre-push` you've installed yourself.

The operator-facing CLI lives in [`crates/aegis-cli`](./crates/aegis-cli) (binary `aegis-boot`). When working on the operator surface, exercise it directly: `cargo run -p aegis-bootctl -- flash --help`. Don't add operator-facing flags without updating [`docs/CLI.md`](./docs/CLI.md).

## Workflow

1. **Open an issue first** for anything bigger than a typo — alignment beats rework.
2. **Branch off `main`**: `feat/<issue>-short-description`, `fix/<issue>-...`, `docs/<topic>`, `chore/<topic>`.
3. **Conventional commits** (validated in PR review; no commitlint hook yet):
   ```
   feat(rescue-tui): add high-contrast theme
   fix(security): block kexec on hash mismatch (#55)
   docs: tighten v0.7.0 CHANGELOG
   ```
   Types: `feat`, `fix`, `refactor`, `docs`, `test`, `chore`, `perf`, `build`, `ci`.
4. **One concern per PR.** Don't bundle a security fix with a refactor.
5. **Tests required for behavior changes.** TDD encouraged — write the failing test first, then make it pass.
6. **Run `dev-test.sh` locally before pushing.** GHA CI is the merge gate but local validation catches problems faster.
7. **PR body should explain the *why*.** The diff explains the what.

## What we look for in a PR

- Tests cover happy path + at least one edge case
- No `unwrap()` / `expect()` outside tests (lints enforce this — won't compile)
- No `unsafe` outside `kexec-loader` (workspace lint forbids)
- Doc strings on new public items (`missing_docs = warn`)
- CHANGELOG updated under the relevant unreleased section if the change is user-visible

## What ships in releases

We follow semver pre-1.0 loosely:

- **patch (`0.x.y`)** — bug fixes, doc fixes, dependency bumps without API change
- **minor (`0.x.0`)** — new features, additive API changes, anything that warrants a release-notes section
- **major (`x.0.0`)** — breaking API changes; v1.0.0 is gated on real-hardware validation ([#51](https://github.com/aegis-boot/aegis-boot/issues/51))

Each release gets a CHANGELOG section, a tag, and a GitHub release. Build artifacts (signed binaries, the `aegis-boot.img`, SBOM, SLSA provenance) are produced + cosign-signed by `.github/workflows/release.yml` on tag push.

### Conventional-commit prefix on PR titles

Release automation parses PR titles for [conventional commit](https://www.conventionalcommits.org/) prefixes and uses them to decide the next semver bump + group entries in the auto-generated CHANGELOG. PR titles **must** match `<type>(<optional-scope>): <subject>`:

| Prefix      | Bump (pre-1.0)        | CHANGELOG section    |
| ----------- | --------------------- | -------------------- |
| `feat:`     | minor (0.18 → 0.19)   | Features             |
| `fix:`      | patch (0.18.0 → .1)   | Bug Fixes            |
| `perf:`     | patch                 | Performance          |
| `refactor:` | patch                 | Code Refactoring     |
| `docs:`     | none                  | Documentation        |
| `ci:`       | none                  | CI/CD                |
| `build:`    | none                  | Build System         |
| `revert:`   | inherits from reverted | Reverts             |
| `chore:` `style:` `test:` | none    | hidden from CHANGELOG (still tagged in commits) |

Breaking changes are signalled by `feat!:` / `fix!:` / a `BREAKING CHANGE:` footer. While we're pre-1.0, breaking changes still bump the **minor** version (per release-please's `bump-minor-pre-major: true` policy in `release-please-config.json`); v1.0.0 cuts only when the real-hardware sweep ([#51](https://github.com/aegis-boot/aegis-boot/issues/51)) clears.

### Cutting a release

The release flow is fully automated end-to-end via Google's [release-please](https://github.com/googleapis/release-please) action (`.github/workflows/release-please.yml`):

1. **Commits accumulate on `main`.** Every PR with a conventional-commit title contributes to the next release.
2. **release-please opens / updates a "chore: release X.Y.Z" PR.** This PR contains:
   - `Cargo.toml` workspace.package.version bumped
   - `CHANGELOG.md` prepended with the new section (grouped by conventional-commit type, with PR links)
   - Annotated lines in `flake.nix`, `docs/INSTALL.md`, `docs/CLI.md`, and the 11 intra-workspace path-deps in `crates/{iso-probe,rescue-tui,aegis-cli}/Cargo.toml` (each carries an `x-release-please-version` annotation comment so release-please knows what to bump)
   - `.release-please-manifest.json` updated
3. **The maintainer reviews + merges the release PR.** This is the editorial control point — hand-edit the auto-generated CHANGELOG entry into aegis-boot's prose style here if you want richer release notes (commit subjects say "what"; the CHANGELOG benefits from the "why" + user-visible impact).
4. **release-please tags the merge commit + creates the GitHub release** with the same body that landed in CHANGELOG.md.
5. **The tag push fires `.github/workflows/release.yml`** — that workflow detects the pre-existing release and uploads cosign-signed assets (Linux + macOS binaries, `aegis-boot.img`, rescue-tui, initramfs, CycloneDX SBOM, SHA256SUMS). SLSA L2 provenance is generated separately.

A note on PR-CI for the release PR: release-please opens its PR with the default `GITHUB_TOKEN`, which (per GitHub's recursive-workflow guard) does **not** trigger `pull_request` workflows. The maintainer can manually re-run CI from the release PR's checks tab. Once we add a fine-scoped PAT or GitHub App as `secrets.RELEASE_PLEASE_TOKEN`, this becomes automatic.

If a release goes sideways (e.g. the v0.18.0 musl-tools build failure that needed a workflow hotfix), retrigger `release.yml` manually via `gh workflow run Release --ref main -f tag=vX.Y.Z` — the workflow's `gh release upload --clobber` is idempotent against the already-created release.

## CI gates your PR must pass

Every PR runs the following — each is also runnable locally. Running them before push saves a CI round-trip.

| Gate | Local command | Source of truth |
| --- | --- | --- |
| Workspace tests (stable + pinned MSRV) | `cargo test --workspace --locked` | `.github/workflows/ci.yml` |
| Clippy `-D warnings` | `cargo clippy --workspace --all-targets -- -D warnings` | same |
| `cargo fmt --check` | `cargo fmt --check` | same |
| macOS + Windows cross-compile check | `cargo check -p aegis-bootctl --target x86_64-apple-darwin --all-targets` | same |
| cargo-deny: advisories + licenses + bans + sources | `cargo deny check` | [`deny.toml`](./deny.toml) |
| `cargo publish --dry-run` on publishable crates | `cargo publish --dry-run -p iso-parser -p kexec-loader --locked` | `.github/workflows/crates-publish-dryrun.yml` |
| Constants drift | `cargo run -p aegis-bootctl --bin constants-docgen --features docgen -- --check` | [`crates/aegis-cli/src/constants.rs`](./crates/aegis-cli/src/constants.rs) |
| CLI drift (subcommand + synopsis) | `cargo run -p aegis-bootctl --bin cli-docgen --features docgen -- --check` | `crates/aegis-cli/src/bin/cli_docgen.rs` |
| JSON schema drift | `cargo run -p aegis-wire-formats --bin aegis-wire-formats-schema-docgen --features schema -- --check` | `docs/reference/schemas/*.schema.json` |
| Workspace version drift | CI job, no local gate | `.github/workflows/ci.yml` |
| Semgrep Rust SAST | GitHub-only | `.github/workflows/ci.yml` (job `sast`) |
| gitleaks secret scan | GitHub-only | `.gitleaks.toml` |
| Miri UB detection (workspace) | `cargo +nightly miri test -p <crate> [--lib]` | `.github/workflows/miri.yml` |
| Real-hardware / OVMF boot smoke | GitHub-only | `.github/workflows/direct-install-e2e.yml`, `ovmf-secboot.yml` |
| Trust-tier + keybinding doc drift | `cargo run -p rescue-tui --bin tiers-docgen -- --check` | `crates/rescue-tui/src/verdict.rs`, `crates/rescue-tui/src/keybindings.rs` |
| Typo gate (#485) | `typos` (install via `cargo install typos-cli --locked`) | [`.typos.toml`](./.typos.toml) |
| OpenSSF Scorecard (#485) | GitHub-only, weekly | `.github/workflows/scorecard.yml` |
| Dependabot auto-bumps (#485) | GitHub-only, weekly + on advisory | `.github/dependabot.yml` |
| Unused-dep gate (#486) | `cargo install cargo-machete --locked && cargo machete` | `.github/workflows/machete.yml` |
| CodeQL data-flow analysis (#488) | GitHub-only, weekly + PR | `.github/workflows/codeql.yml` |

`./scripts/dev-test.sh` bundles most of these into a single "run-before-push" command.

**First PR?** `gh issue list --label "good first issue"` surfaces issues curated for newcomers. Or propose your own fix via a new issue first (step 1 above).

## Extending the CLI

Adding a new subcommand touches **four** places; the CI `cli-drift` gate will reject partial wiring:

1. `crates/aegis-cli/src/<subcommand>.rs` — implementation
2. `crates/aegis-cli/src/main.rs` — dispatch table + `print_help()` entry
3. `crates/aegis-cli/src/bin/cli_docgen.rs` — `SUBCOMMANDS` registry
4. `docs/CLI.md` + `man/aegis-boot.1.in` — prose companion + man section

After editing, regenerate the synopsis (picked up by the CI drift-check):

```bash
cargo build --release -p aegis-bootctl --bin aegis-boot
cargo run -q -p aegis-bootctl --bin cli-docgen --features docgen -- --write
```

## Security issues

**Do not file public issues for vulnerabilities.** See [SECURITY.md](./SECURITY.md) for the private reporting path.

## Code of conduct

This project follows the [Contributor Covenant 2.1](./CODE_OF_CONDUCT.md). Be kind, be specific, assume good intent.

## License

By contributing, you agree your contributions are dual-licensed under [Apache-2.0](./LICENSE-APACHE) OR [MIT](./LICENSE-MIT) at the user's option, matching the rest of the project.
