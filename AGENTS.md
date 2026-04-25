# AGENTS.md

Vendor-neutral guidance for AI coding agents working on this repo, per the
[agents.md](https://agents.md) convention. Complements the human-facing
[CONTRIBUTING.md](./CONTRIBUTING.md) — same workflow, distilled for agents
plus security-critical guardrails that matter more here than in a typical
Rust project.

If you are an agent reading this for the first time, read it end-to-end
before making changes. The "non-negotiables" and "sensitive surfaces"
sections are not style preferences — they encode the trust model.

---

## 30-second orientation

aegis-boot is a **signed UEFI Secure Boot rescue environment** that lets
operators pick any ISO from a USB stick's data partition and `kexec` into it
without leaving the chain of trust. The boot path is:

```
UEFI firmware → shim (MS-signed) → grub (Canonical-signed) → rescue kernel
  → our initramfs → rescue-tui (ratatui) → kexec_file_load(2) → selected ISO
```

Trust is enforced at every hop. **The whole point of this project is that
no code path bypasses signature verification.** Changes that weaken or
short-circuit that chain are not bugs to fix — they are the threat we are
defending against. See [ADR 0001](./docs/adr/0001-runtime-architecture.md)
and [THREAT_MODEL.md](./THREAT_MODEL.md) for the full rationale.

---

## Where to look first

| Question | File |
| --- | --- |
| What does this project do? | [README.md](./README.md) |
| Why does the runtime look like this? | [docs/adr/0001-runtime-architecture.md](./docs/adr/0001-runtime-architecture.md) |
| What's the trust model? | [THREAT_MODEL.md](./THREAT_MODEL.md) |
| Architecture + crate dependencies | [docs/ARCHITECTURE.md](./docs/ARCHITECTURE.md) |
| How do I run tests locally? | [docs/LOCAL_TESTING.md](./docs/LOCAL_TESTING.md) — start with `tools/local-ci.sh quick` (~9s) |
| What's the workflow for PRs? | [CONTRIBUTING.md](./CONTRIBUTING.md) |
| What ships in CI? | [CONTRIBUTING.md §CI gates](./CONTRIBUTING.md#ci-gates-your-pr-must-pass) |
| Branch protection state | [docs/governance/CI_REQUIRED_CHECKS.md](./docs/governance/CI_REQUIRED_CHECKS.md) |
| Why was X deferred? | [docs/adr/](./docs/adr/) — every deferral has an ADR with re-entry criteria |

---

## Non-negotiables

These are not style preferences. Violating any of them produces a
security regression, even when the code compiles and tests pass.

1. **No `unsafe` outside `crates/kexec-loader/`.** The workspace lint is
   `unsafe_code = "forbid"` everywhere else. The single `unsafe` block in
   `kexec-loader` wraps `kexec_file_load(2)` and is reviewed against the
   Linux man page on every change.

2. **No bypassing signature verification.** This includes `verify_iso_hash`,
   `aegis_trust::TrustAnchor::verify_with_epoch`, the `KEXEC_SIG`
   kernel-side check, and the cosign verification in `fetch-image`.
   Patterns to refuse: `unwrap_or(true)`, `if cfg!(test) { skip }`,
   `// TODO: re-enable verification`, `let _ = verify(...)`.

3. **No silencing crypto / verification errors.** A failed verification
   must surface as a typed error (`HashVerification::Mismatch`,
   `TrustVerdict::HashMismatch`, `KexecError::SignatureRejected`, etc.)
   that flows through the existing TUI / CLI surfacing. Don't `?`-it-away
   with a generic "io error" string.

4. **No new wire-format fields without bumping `schema_version`.**
   `crates/aegis-wire-formats/` carries `Manifest`, `Attestation`,
   `BundleManifest`, `DoctorReport`. Each has a `*_SCHEMA_VERSION`
   constant. Additive optional fields are OK at the same schema; any
   structural change bumps the version.

5. **No edits to `.github/workflows/release.yml`, `scripts/mkusb.sh`'s
   boot-chain assembly, or signing flows without explicit maintainer
   consent.** These produce signed artifacts. A subtle regression here
   ships to operators as a "this binary is signed but doesn't do what
   the signature claims" bug — the worst kind of trust bug.

6. **No removing tests to make CI pass.** If a test fails, either the
   code is wrong (fix it) or the test's expectation has drifted (update
   it with explicit reasoning in the commit body, not silent deletion).

7. **No `unwrap()` / `expect()` outside tests.** The clippy lints
   `expect_used` and `unwrap_used` enforce this. In tests they're allowed
   via `#[allow(clippy::unwrap_used, clippy::expect_used,
   clippy::missing_panics_doc)]` at the `mod tests` level.

---

## Coding standards

- **Rust toolchain:** pinned to **1.88.0** via CI. Run with `cargo +1.88.0`
  locally to match CI's lint set exactly. `tools/local-ci.sh` does this
  automatically. Newer rustc adds new lints — running on the default
  stable can produce green-here / red-on-CI surprises.
- **Workspace lints:**
  - `unsafe_code = "forbid"` (except `kexec-loader`)
  - `missing_docs = "warn"` on public items
  - clippy `-D warnings` in CI; `-D` clippy::expect_used / unwrap_used /
    panic outside tests
- **Errors:** `Result` + `?`. `thiserror` for typed errors at module
  boundaries; `Result<_, String>` is acceptable for tooling internals
  (e.g. `save_error_log`). Do not use `panic!()` in production paths.
- **Logging:** `tracing::{info, warn, error, debug}!` with structured
  fields. `eprintln!` only for messages targeted at a human terminal
  (e.g. operator-facing CLI errors that the user reads, TUI startup
  banner). Do not `eprintln!` for things you'd want to grep in logs.
- **Subprocess:** wrap `Command::new()` in module-local helpers when
  reused (see `cmd_path::which`, `subprocess-adapter`-style patterns).
  Always handle non-zero exit + stderr capture; never `.unwrap()` an
  `output()`.
- **Doc comments:** explain WHY (constraint, invariant, ADR reference),
  not WHAT (well-named identifiers already say what). Reference the
  issue or ADR that motivated the code where applicable.
- **Comments inside functions:** sparingly, for non-obvious WHY.
  Don't narrate the implementation. Don't reference current PRs or
  callers — those rot. Reference durable artifacts (issue numbers,
  ADRs, threat-model sections).
- **Tests:**
  - Function names describe the assertion: `read_nics_linux_skips_lo`,
    `parse_smart_health_warning_status`, not `test_1`
  - Cover happy path + ≥1 edge case + error path for behavior changes
  - Test modules use `#[allow(clippy::unwrap_used, clippy::expect_used,
    clippy::missing_panics_doc)]` at the `mod tests` level
  - Use `tempfile::tempdir` + sysfs/file fixtures rather than mocking
    when feasible — the assertions stay closer to the real shape
- **Conventional commits:** `feat`, `fix`, `refactor`, `docs`, `test`,
  `chore`, `perf`, `build`, `ci`. Include the issue number in the
  subject when there is one. Body explains WHY.
- **One concern per PR.** Don't bundle a security fix with a refactor;
  don't bundle a feature with a CI change. Easier to review, easier to
  revert if the post-merge week reveals a regression.

---

## Sensitive surfaces — consensus required before changes

For changes in any of these paths, surface the proposal in the PR body
or an issue first and wait for explicit maintainer approval. The agent
should not make non-trivial edits autonomously here.

| Path | Why sensitive |
| --- | --- |
| `crates/aegis-trust/` | Trust-anchor + epoch verification (ADR 0002) |
| `crates/iso-probe/src/signature.rs` | Hash + minisign verification |
| `crates/kexec-loader/` | Only `unsafe` in workspace; raw syscall |
| `crates/aegis-wire-formats/` | Schema-versioned wire formats; format changes ripple |
| `crates/rescue-tui/src/verdict.rs` | Six-tier `TrustVerdict` — drives boot gating |
| `.github/workflows/release.yml` | Signing + attestation + cosign |
| `scripts/mkusb.sh` boot-chain assembly | Shim/grub/kernel layout under SB |
| `crates/aegis-cli/src/attest.rs` | Attestation manifest emit + verify |

For doctor-row additions, kernel-cmdline edits, TUI polish, and
similar leaf-feature work, autonomous changes are fine — the trust
boundary is well-defined and these paths sit firmly inside it.

---

## Common agent traps

Specific anti-patterns LLM agents reach for that produce real
regressions in this project. If you find yourself about to do any of
these, stop and re-read the relevant module's existing pattern.

- **"Let me just `unwrap_or_default()` the signature error."** No.
  Trust failures must surface. Use the existing typed-error +
  TrustVerdict / FailedIso / Error-screen routing.
- **"I'll add `if !verified { panic!() }` to make the test green."**
  No. Production code does not panic on trust failures — it routes
  through the verdict surface so operators see a remediable error.
- **"The test was failing so I deleted the assertion."** Revert.
  File an issue if the test's expectation seems wrong; don't merge
  the deletion.
- **"Let me regenerate the docgen output by hand."** No. The
  `constants-docgen`, `cli-docgen`, `tiers-docgen`, and
  `aegis-wire-formats-schema-docgen` binaries are the source of
  truth. Run them via the commands in CONTRIBUTING.md §CI gates.
- **"I'll silence this clippy lint with `#[allow]`."** Sparingly,
  with a documented reason. Most lints catch real bugs in this
  codebase. If you must allow, comment why.
- **"I'll add a `--no-verify` flag so the test can run without crypto."**
  Almost never the right answer. Use a fixture-based unit test that
  exercises the parser/verdict logic without touching the real signing
  path (see `parse_smart_health_*_status` for the shape).
- **"Let me bump dependencies broadly while I'm here."** No. One
  concern per PR. Dep bumps go through dependabot or a focused
  upgrade PR.

---

## Workflow

- **Branch off `main`:** `feat/<issue>-...`, `fix/<issue>-...`,
  `docs/<topic>`, `chore/<topic>`. Match the issue number in the
  branch name.
- **Local sanity before push:** `tools/local-ci.sh quick` (~9s).
  See [docs/LOCAL_TESTING.md](./docs/LOCAL_TESTING.md) for the full
  decision tree of which subcommand to run when.
- **Opt-in pre-push hook:** `tools/install-hooks.sh` wires
  `local-ci.sh quick` into `git push`. Bypass with `--no-verify` for
  WIP.
- **PR body explains the WHY.** The diff explains the what. Include
  the test plan: what you ran locally, what you couldn't, and why.
- **CI is the merge gate.** See
  [CONTRIBUTING.md §CI gates](./CONTRIBUTING.md#ci-gates-your-pr-must-pass)
  for the full table. Docs-only PRs skip the expensive QEMU suite via
  `paths-ignore` (#592).

---

## When to ask, not act

Escalate to the maintainer (open an issue, ask in the PR body, don't
merge autonomously) when:

- A change would touch any path in §Sensitive surfaces
- A test's expectation seems wrong and you'd want to update it
- Verification or signature behavior would change observably
- A wire format would gain a field, lose a field, or change a type
- Any release-pipeline file (`.github/workflows/release.yml`,
  `crates-publish*.yml`, `scripts/mkusb.sh`) needs editing
- Conflicting evidence about the right approach (run a
  `consensus_vote` if your tooling supports it)
- You're about to run a destructive command on shared state
  (force-push to main, drop a database, delete a branch)

---

## Maintained by

The aegis-boot maintainers. This file evolves alongside the project —
PRs that change the trust model, the workflow, or the lint policy
should also update this file in the same PR.

**Last updated:** 2026-04-25
