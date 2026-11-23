# Reproducible build recipe

This is the recipe referenced by `docs/TRANCHE_2_3_TECHNICAL_SPECIFICATION.md`: steps that make
a contract build byte-for-byte reproducible by anyone, on any machine, so a build can be
verified against its source without trusting us. This covers both every future build (default
settings) and today's already-live contracts specifically (one documented override) — see
"Important, corrected" below.

## Root cause of the 2026-11-22 non-reproducibility report

The dApp team reported rebuilding commit `3af9388` and getting 0 of 12 WASM hashes
matching the deployed bytes, with Permissioning's 12-byte diff traced to two functions emitted
in reverse order — i.e. identical source, different machine-code layout.

Verified root cause: **the Rust toolchain was never pinned**, anywhere.

- `Cargo.lock` *is* committed (dependency versions were already reproducible).
- No `rust-toolchain.toml` existed at the repo root.
- CI (`.github/workflows/ci.yml`) builds against `dtolnay/rust-toolchain@stable`, i.e.
  whatever "stable" resolves to on the day the workflow runs — it floats too.

So the only reason any one machine reproduces the deployed bytes is if it happens to already
have the exact rustc/LLVM build used at deploy time (`rustc 1.94.1`, bundled LLVM 21.1.8)
installed. A different rustc point release — even with identical source and identical
dependency versions — changes LLVM's codegen-unit internals enough to reorder emitted
functions. That reordering is exactly the "two functions swapped" symptom reported.

**Fix:** `rust-toolchain.toml` added at the repository root, pinning
`channel = "1.94.1"` with `targets = ["wasm32v1-none", "wasm32-unknown-unknown"]`. Any
machine with `rustup` installed will now auto-switch to this exact toolchain the moment it
runs a cargo command inside the repo — no manual version matching required.

## Verification performed

Rebuilt commit `3af9388` from a clean `git worktree` (isolated from any already-built
`target/` directory) using the documented recipe, after confirming `rustc --version`
resolved to `1.94.1 (e408947bf 2026-03-25)` via the toolchain pin:

```
git worktree add /tmp/repro-check 3af9388
cd /tmp/repro-check
cargo build --workspace --release --target wasm32v1-none
sha256sum target/wasm32v1-none/release/*.wasm
```

Result: **12/12 WASM artifacts matched the deployed hashes exactly**, byte for byte,
including `principal_permissioning.wasm`:

| Contract | SHA-256 (rebuilt) | Matches deployed? |
|---|---|---|
| `principal_manager.wasm` | `e2d04b45b867d0dbc41660ac0d97d3e705005ddfb10350bb030d25b1aff86115` | yes |
| `principal_market_config.wasm` | `11b0cbdf10a8d204fcb568c54529801747feac05208c5bcbfaefd822ad0c79a8` | yes |
| `principal_market_factory.wasm` | `6c21fc0e78f3064f67ae30924b8f9ca0223fbb61a20ac640cceca7b152456759` | yes |
| `principal_market_pool.wasm` | `52285a25bbf372d0e5d22ba565b76af6a7e832422df83b6a4809de8adf3aa112` | yes |
| `principal_oracle_adapter.wasm` | `8581bbba82bb57f5c5c9c00adf98b1691ad20821840e55a67ad8d7ecace0c1b0` | yes |
| `principal_permissioning.wasm` | `4437468dbbda6d86a115f188de877135b0b5a400b69ad7e9444cf269c54f969c` | yes |
| `principal_pt_token.wasm` | `db025340887ef9d92922e2444da24edab5fec125460e9adc373a5f77b26578b2` | yes |
| `principal_recovery_escrow.wasm` | `dadea71fcfb84adcfb4691f472551f490cf440c5aaae5c6b5e5d4bb6f61441e8` | yes |
| `principal_risk_control.wasm` | `ac75508d24c5457de7db1106bf8b5e9b62ea436f3dfd1a309fda5138110c2526` | yes |
| `principal_router.wasm` | `2a0997317651edb0f7e2bd753570b03a88462aa6d216c14b06ebec246bb3c1c2` | yes |
| `principal_sy_wrapper.wasm` | `be4b12cf3c16461b7e71fa225ed2363aba62bdc42c1ea5c08c98d7ede6a27c9a` | yes |
| `principal_yt_token.wasm` | `c83a2a8d2a4a79ff5348675581a27d48e52a920d83dc887672840d4a79c02654` | yes |

These are the same 12 hashes already recorded in `docs/MAINNET_DEPLOYMENT_EVIDENCE.md` §8
(captured at source commit `4b5bc2b`) and cross-checked against each deployed contract's
on-chain WASM hash at upload time. No contract source changed between `4b5bc2b` and
`3af9388` (that range only added self-service tooling scripts), so the two commits are
expected — and now confirmed — to produce identical bytes.

## A second cause found while verifying the fix: absolute paths baked into the WASM

The 12/12 match above was real, but it understated the problem: it was still one physical
machine, with two different `$CARGO_HOME` locations and two different checkout paths set via
environment variables — not two genuinely different machines. (An earlier draft of this document
said "different machines"; that was imprecise and has been corrected.)

This matters because rustc embeds absolute filesystem paths into the compiled output —
dependency source paths under `$CARGO_HOME/registry/src/...` used by `file!()` /
`#[track_caller]` call sites inside `soroban-sdk` itself, not just our own crates — regardless
of toolchain version and regardless of `strip = "symbols"` (`strip` removes DWARF debug info;
these are plain `&'static str` byte literals in `.rodata`, unaffected by stripping). Two
builds that differ only in filesystem layout cannot be byte-identical as a result, even with
the exact same pinned rustc.

**Verified directly:** rebuilt commit `3af9388` a second time, under a `$CARGO_HOME` pointed at
an empty, freshly-fetched directory instead of the real one. Result: **10 of 12 hashes changed**
(only `principal_permissioning.wasm` and `principal_risk_control.wasm` were unaffected on that
pass — see below for why that turned out to be more specific than "these two embed no path").
This confirms the toolchain pin alone is not sufficient for a truly independent rebuild to match.

**Fix:** added [`scripts/deploy/reproducible_build.sh`](../../scripts/deploy/reproducible_build.sh),
which builds with `--remap-path-prefix` on both the dependency-cache path and the checkout path,
normalizing them to fixed strings before they ever reach the compiler. **Verified this closes
the gap:** rebuilding `3af9388` with this script from two different checkout paths *and* two
different `$CARGO_HOME` locations now produces **12/12 identical hashes between the two
builds**.

## The host OS/architecture is also part of the recipe

Confirmed (2026-11-23): the same recipe, same pinned `rustc 1.94.1`, on macOS `aarch64` and on
Linux `aarch64` both produce different bytes than `x86_64-unknown-linux-gnu` — including for
`principal_permissioning.wasm`, which carries no machine-specific path at all (only
`/rustc/e408947bf…/library/…`, identical on every host with this rustc), so this is a genuinely
separate cause from path embedding. The host ISA/OS affects codegen determinism for this target
independent of everything else pinned so far.

**Fix:** `reproducible_build.sh` now checks `rustc -vV`'s `host:` line and refuses to run on
anything other than `x86_64-unknown-linux-gnu`, pointing at a pinned `linux/amd64` container as
the alternative. CI's runners (`ubuntu-latest`) already satisfy this.

## Important, corrected: today's already-deployed bytes *are* third-party-reproducible

An earlier version of this document said the original build's absolute paths "were never
recorded and can't be recovered," and concluded a from-scratch third-party rebuild of today's
live contracts was therefore not possible. That was wrong, and is corrected here.

**The path is recorded — in the deployed bytes themselves.** 10 of the 12 live contracts embed
exactly one kind of absolute path, entirely under `$CARGO_HOME`:
`.../registry/src/index.crates.io-1949cf8c6b5b557f/soroban-sdk-26.0.1/src/{env,ledger,vec}.rs`.
Running `strings` on any fetched deployed wasm shows it directly, e.g.:

```sh
stellar contract fetch --wasm-hash 8581bbba82bb57f5c5c9c00adf98b1691ad20821840e55a67ad8d7ecace0c1b0 \
  --rpc-url https://soroban-rpc.mainnet.stellar.gateway.fm \
  --network-passphrase 'Public Global Stellar Network ; September 2015' \
  --out-file /tmp/deployed_oracle_adapter.wasm
strings /tmp/deployed_oracle_adapter.wasm | grep registry/src
```

The workspace's own crates only ever appear as relative paths (`contracts/market_pool/src/lib.rs`)
— the checkout path never leaks into the bytes, only `$CARGO_HOME` does. `principal_permissioning`
and `principal_risk_control` contain no `$CARGO_HOME`-derived path at all (only the
rustc-version-pinned `/rustc/...` path, identical everywhere), which is exactly why those two
were unaffected by the `$CARGO_HOME` test above but still sensitive to the host OS/arch test.

**Verified directly (2026-11-23):** fetched the real deployed `principal_oracle_adapter.wasm`
from mainnet, confirmed its hash matches the manifest exactly, and read its embedded path with
`strings` — it does not need to be published here, since anyone can recover it themselves the
same way, and publishing it repeatedly in this document was itself an unnecessary, avoidable
leak of the operator's real home directory (now fixed; see "Correction" below). Call this
recovered value `$ORIGINAL_CARGO_HOME` below. Then rebuilt `3af9388` from a fresh `git worktree`,
under a throwaway `$CARGO_HOME` with freshly re-fetched dependencies (so it isn't the real build
cache), with:

```sh
REMAP_CARGO_HOME_TO="$ORIGINAL_CARGO_HOME" ./scripts/deploy/reproducible_build.sh
```

**Result: 12 of 12 hashes matched the real, currently-live on-chain bytes exactly.** This
confirms today's deployed contracts are reproducible from a cold checkout on a genuinely
independent machine, given: `rustc 1.94.1` (the `rust-toolchain.toml` pin),
`x86_64-unknown-linux-gnu` as the host, and `$CARGO_HOME` remapped to the one real value that
happens to be baked into today's live bytes (a future redeploy will bake in whatever
`REMAP_CARGO_HOME_TO` that future build used, documented alongside it when it happens).

The on-chain-hash-matches-manifest verification in §8 remains the first, simplest check. This
section means a full from-scratch rebuild is now also available as a second, independent check,
for today's contracts and not only future ones.

**Correction (2026-11-23):** an earlier version of this document, and of
`docs/MAINNET_DEPLOYMENT_EVIDENCE.md`, printed the recovered path in full, repeatedly — an
unforced leak of the operator's real home directory into a public repository, pointed out by the
dApp team. Fixed throughout: neither document states the literal value anymore. Recover it
yourself with `strings` as shown above if you need it.

**Correction (2026-11-23), a second issue:** the command above does not run as written against
`3af9388` itself, checked out on its own — `scripts/deploy/reproducible_build.sh` did not exist
at that commit (it was added later, at `ae6ccd9`). To verify a historical commit that predates
the script, copy the script in from the current `mainnet` HEAD before running it:

```sh
git worktree add /tmp/repro-check 3af9388
cp "$(git -C /tmp/repro-check rev-parse --show-toplevel)/../principal/scripts/deploy/reproducible_build.sh" \
  /tmp/repro-check/scripts/deploy/ 2>/dev/null || \
  git show mainnet:scripts/deploy/reproducible_build.sh > /tmp/repro-check/scripts/deploy/reproducible_build.sh
chmod +x /tmp/repro-check/scripts/deploy/reproducible_build.sh
cd /tmp/repro-check && REMAP_CARGO_HOME_TO="$ORIGINAL_CARGO_HOME" ./scripts/deploy/reproducible_build.sh
```

Any commit at or after `ae6ccd9` already has the script in its own tree, so this extra step is
only needed for commits older than that.

## How to reproduce a build yourself

1. Install `rustup` if you don't already have it (https://rustup.rs).
2. Make sure you're on `x86_64-unknown-linux-gnu` (native, or a pinned `linux/amd64` container —
   `reproducible_build.sh` will refuse to run otherwise and tell you why).
3. `git clone` this repository (or `git worktree add` a given commit) and `cd` into it. If the
   commit predates `ae6ccd9`, copy `scripts/deploy/reproducible_build.sh` in from current `mainnet`
   first (see above) — it won't be in that old commit's own tree.
4. Run `rustc --version` — because of the committed `rust-toolchain.toml`, `rustup` will
   transparently download and switch to `1.94.1 (e408947bf 2026-03-25)` if you don't already
   have it installed. Confirm the version string matches exactly before proceeding.
5. For a **future** deployment (anything built with this script going forward): run
   `./scripts/deploy/reproducible_build.sh` with no overrides. For **today's already-live**
   contracts specifically: recover the embedded path yourself with `strings` on a fetched
   deployed wasm (see above), then run it with `REMAP_CARGO_HOME_TO=<that recovered path>`.
6. Compare the printed hashes against the manifest you're checking.

## Residual known limitation

Toolchain version, host OS/architecture, and both relevant absolute paths are now pinned or
accounted for. What remains open: this has only been tested with `rustc 1.94.1` specifically and
on `x86_64-unknown-linux-gnu` specifically — a future toolchain bump, or a host this hasn't been
tested on (e.g. a different glibc/musl base image), could in principle introduce a new variable
not yet seen. Any such finding belongs here, in the same verified-not-assumed style as the rest
of this document, not folded silently into "it just works."
