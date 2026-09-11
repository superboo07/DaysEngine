# Dependency policy

DaysEngine reads a commercial game's data files off the user's own disk. A
compromised build dependency would run with the user's privileges on a machine
that has 12 GB of their personal game install on it, so the dependency set is
deliberately small and deliberately boring.

## Rules

1. **Every direct dependency is pinned exactly.** Workspace manifests use
   `=X.Y.Z`, never `^X.Y.Z`. Cargo's default caret range means a clone six
   months from now silently compiles code nobody in this repo has read.
2. **`Cargo.lock` is committed** and pins the whole transitive graph. This is the
   part that actually matters — `=` pins only bind our direct edges.
3. **Every build uses `--locked`.** Without it Cargo will happily rewrite the
   lockfile to satisfy a changed manifest and the pin becomes decorative. The
   `just` recipes and CI all pass it; `just` will fail rather than update a lock.
4. **No git or path dependencies on third-party code**, enforced by
   `deny.toml [sources]`. A git dependency is a mutable reference — the commit a
   tag points at can change. crates.io releases are immutable.
5. **No build-script-heavy crates without a specific reason.** `build.rs` runs
   arbitrary code on every developer machine before any of our code executes.
   The two we accept are called out below.
6. **Adding or bumping anything is a reviewed change** with the checklist below
   filled in, and `cargo deny --locked check` passing.

## Audit checklist for a new or bumped dependency

- [ ] Is it actually needed, or is this 40 lines we could write and own?
- [ ] Who maintains it, and is it maintained? (last release, open advisories)
- [ ] Download count and reverse-dependency count — is anyone else auditing it?
- [ ] Does it have a `build.rs`? If so, read it.
- [ ] Does it use `unsafe`? How much, and is it in our hot path?
- [ ] What does it pull in transitively? Run `cargo tree -p <crate>`.
- [ ] License compatible with `deny.toml [licenses] allow`.
- [ ] `cargo deny --locked check` clean afterwards.

## Current direct dependencies

| Crate | Version | Why we need it | Notes |
|---|---|---|---|
| `miniz_oxide` | 0.9.1 | GPK index and entry inflate | Pure Rust, no C. Same inflate backend `flate2` uses by default; depending on it directly avoids `flate2`'s optional zlib-sys C backends. |
| `thiserror` | 2.0.20 | `derive(Error)` | Proc macro, compile-time only, zero runtime surface. dtolnay; among the most-reviewed crates in the ecosystem. |
| `log` | 0.4.34 | Logging facade | rust-lang owned. |
| `env_logger` | 0.11.11 | Log backend | Binary crates only; never linked into the libraries. |
| `serde` + `serde_json` | 1.0.229 / 1.0.151 | Route graph and config (de)serialization | Proc macro in `derive`. Note `serde_derive` historically shipped a precompiled binary; that was reverted and the current release builds from source — re-check this on every bump. |
| `clap` | 4.6.6 | CLI parsing for the offline tools | Binary crates only. |
| `sdl3` | 0.20.0 | Window, input, GPU present, audio output | Thin bindings over the **system** libSDL3; has a `build.rs` that locates and links it. Reviewed: it probes pkg-config and does not download anything. Younger crate than the rest of this list — the riskiest entry here, revisit on each bump. |
| `rusty_ffmpeg` | 0.17.0 | WMV3/VC-1 video and Vorbis audio decode | Generates bindgen bindings against the **system** ffmpeg at build time, so it tracks whatever libav* the host ships rather than lagging releases like `ffmpeg-next`. Has a `build.rs` running bindgen. Chosen deliberately: we link the distro's ffmpeg, which the distro already patches for CVEs, instead of vendoring a frozen copy. |
| `png` | 0.18.1 | PNG decode for backgrounds and UI art | image-rs owned, pure Rust. Used instead of routing PNGs through ffmpeg so the image path has no C in it. |

## Deliberately *not* depended on

- **`ffmpeg-next`** — lags upstream ffmpeg releases; would have blocked us on
  libavcodec 63 (ffmpeg 8.1), which is what this machine ships.
- **`cpal`** — SDL3 already gives us an audio device and stream mixer. One
  fewer dependency and one fewer platform backend matrix.
- **`lewton` / `symphonia`** — ffmpeg already decodes the Vorbis the game ships.
- **`flate2`** — wraps `miniz_oxide` for our use case anyway, but also exposes
  C zlib backends we do not want reachable.
- **`pefile`-equivalent crates (`goblin`, `object`)** — we need exactly one
  thing from the PE format (a named resource blob), which is ~150 lines in
  `days-gpk/src/pe.rs`. Not worth a general-purpose binary parser.

## Third-party native libraries

`libSDL3` and `libav*` are linked from the system, not vendored. This is a
deliberate trade: it means we inherit the distribution's security updates for
two large C codebases that parse untrusted media, rather than freezing a copy
that goes stale. It also means the build depends on the host having them —
see the README for the per-distro package names.

Note that ffmpeg will be parsing media out of the user's own game install, which
is not attacker-controlled in the normal case. Keep it that way: never point the
decoder at a file the user did not supply themselves.

## Verifying a checkout

```bash
just audit          # cargo deny --locked check
cargo tree --locked --duplicates
git diff --exit-code Cargo.lock   # must be empty after a build
```
