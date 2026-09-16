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
| `rusty_ffmpeg` | 0.17.0 | WMV3/VC-1 video and Vorbis audio decode | Generates bindgen bindings at build time against whichever ffmpeg it is pointed at, rather than carrying pre-written ones for a release it lags behind, as `ffmpeg-next` does. Has a `build.rs` running bindgen. `FFMPEG_PKG_CONFIG_PATH` is how it is aimed at the vendored prefix; see *Third-party native libraries*. |
| `png` | 0.18.1 | PNG decode for backgrounds and UI art | image-rs owned, pure Rust. Used instead of routing PNGs through ffmpeg so the image path has no C in it. |

## Deliberately *not* depended on

- **`ffmpeg-next`** — lags upstream ffmpeg releases, and the vendored ffmpeg is
  pinned at `n9.0.1` (libavcodec 63, libavutil 61, libswscale 10), ahead of what
  it binds.
- **`cpal`** — SDL3 already gives us an audio device and stream mixer. One
  fewer dependency and one fewer platform backend matrix.
- **`lewton` / `symphonia`** — ffmpeg already decodes the Vorbis the game ships.
- **`flate2`** — wraps `miniz_oxide` for our use case anyway, but also exposes
  C zlib backends we do not want reachable.
- **`pefile`-equivalent crates (`goblin`, `object`)** — we need exactly one
  thing from the PE format (a named resource blob), which is ~150 lines in
  `days-gpk/src/pe.rs`. Not worth a general-purpose binary parser.

## Third-party native libraries

`libSDL3` and `libav*` are **vendored**, pinned as git submodules in
`third_party/`, and built by `just deps` into `target/sdl/linux` and
`target/ffmpeg/linux`. A developer build links those, and so does a release
archive. `.cargo/config.toml` is where cargo is told, so a plain `cargo build`
finds them without the task runner.

So the build depends on a container runtime, not on the host's libraries:

```bash
# Arch
sudo pacman -S clang pkgconf podman
# Debian / Ubuntu
sudo apt install clang pkg-config podman
```

Then, once per checkout:

```bash
just deps        # or: ./tools/build-sdl.sh linux && ./tools/build-ffmpeg.sh linux
```

Both build inside `tools/dist/`'s image, which is the only place carrying SDL3's
and ffmpeg's own build dependencies and is what decides a release archive's
glibc floor — `tools/in-container.sh` is that argument in full, and the scripts
re-exec there by themselves.

### Why this stopped being the system's copies

Linking the distribution's was the earlier decision, and it had a real argument
behind it: a distribution ships security updates for two large C codebases that
parse media, and freezing a copy gives that up. It was taken while this project
was developed on a host whose ffmpeg happened to be current.

It does not survive a host whose is not. **`media::image` calls
`sws_scale_frame` on an allocated-but-uninitialised context** — the dynamic
swscale API, which ffmpeg's own header documents as usable "without setting up
any frame properties or calling `sws_init_context()`", and which first ships in
**n8.0** (`git tag --contains 2a091d4f2e`, the commit "swscale: introduce new,
dynamic scaling API"). Debian 13 ships ffmpeg 7.1. There, that call reaches
`av_frame_ref` through `sws_frame_start` with frames the context was never
configured for, and segfaults.

The older `sws_scale` is not an alternative: `sws_scale_frame` is the entry
point that honours the `threads` option, and a movie frame scaled to a 4K window
on one thread costs more than the 41ms a 24 fps frame gets. See
`media::video::new_scaler`.

A player who wants their distribution's ffmpeg still gets it — it stays shared
precisely so it can be replaced, and `FFMPEG-SOURCE.txt` in the archive says so.

**SDL3 is linked statically and ffmpeg is not.** SDL3 is zlib, which attaches no
condition to linking it in, so it goes inside the executable; that is the
`static-sdl` feature, and it is **on by default**. ffmpeg is LGPL v2.1, which
permits static linking only against an obligation to let the player relink, so
it stays shared and is found through an rpath. Only an LGPL library is dynamic
here.

`--no-default-features` asks for a *shared* SDL3 instead. That alone is not
enough to get the distribution's: `PKG_CONFIG_LIBDIR` in `.cargo/config.toml`
names only the vendored prefix, which is static-only, so the link fails until
that variable is pointed back at the system's own pkgconfig directory. Both
halves are needed, and they are two separate decisions.

Note that ffmpeg will be parsing media out of the user's own game install, which
is not attacker-controlled in the normal case. Keep it that way: never point the
decoder at a file the user did not supply themselves.

### Release archives vendor both

A **distribution build** cannot make the assumption above. The player who
unpacks an archive may have no libSDL3 at all, and on Windows there is no
distribution shipping libav security updates to inherit from in the first place.
So an archive carries its own copies, built from the pinned submodules in
`third_party/` by `tools/build-sdl.sh` and `tools/build-ffmpeg.sh`:

| | |
|---|---|
| `third_party/ffmpeg` | FFmpeg `n9.0.1`, LGPL v2.1 |
| `third_party/sdl` | SDL `release-3.4.16`, zlib |

They are linked differently, and the licence is the whole reason. **SDL3 is
zlib, so it is built static and goes inside the executables** (the `static-sdl`
feature, which `tools/dist.sh` turns on and a developer build leaves off); no
libSDL3 ships at all. **ffmpeg is LGPL v2.1, so it stays shared**: static
linking is permitted only against an obligation to let the player relink, and
shipping it shared satisfies section 6 without one — which is also the
arrangement this project wants, since a player who needs a patched libav can
drop the library in. The ffmpeg build is `--disable-gpl
--disable-nonfree --disable-version3 --disable-autodetect`, so neither a
GPL-only component nor a library that merely happened to be installed on the
build machine can end up in a shipped binary. Every archive carries
`FFMPEG-SOURCE.txt` (upstream commit and the full configure line) and
`COPYING.LGPLv2.1` beside the libraries; that pair is the compliance artifact
and an archive must not be published without it. It also carries `SOURCE.zip` —
the engine's own source as it was when the binary was built, working-tree
changes included — and is named after the commit it came from.

These submodules are **not** in the developer path. `cargo build`, the tests and
the `daysengine` inspection subcommands never touch them, and the pins bump the same way
a dependency does: deliberately, with the diff reviewed.

`docs/WINDOWS.md` has the cross-build, the toolchain it needs, and the component
list the ffmpeg build is trimmed to.

## Verifying a checkout

```bash
just audit          # cargo deny --locked check
cargo tree --locked --duplicates
git diff --exit-code Cargo.lock   # must be empty after a build
```
