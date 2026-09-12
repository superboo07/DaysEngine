# DaysEngine

A cross-platform reimplementation of Overflow's **FILMEngine**, the engine behind
*School Days HQ*.

DaysEngine ships **no game data**. You drop it into your own original install and
run it, and you get School Days HQ — same archives, same scripts, same UI art,
same save format, same routes. The target is a full reimplementation of the
engine *and* the game shell on top of it, not a decompilation and not a
reinterpretation: every menu, button, fade and font glyph comes out of the
game's own `.GPK` archives at runtime, so playing it should be indistinguishable
from playing the original.

Drop the binaries into your existing game folder — next to `SCHOOLDAYS HQ.exe` —
and run them. No arguments, no configuration, no separate asset extraction step:

    cd "/path/to/School Days HQ" && ./days key

(Pass `--game <dir>` if you would rather keep the binaries somewhere else.)

## Status

Early. What works today:

| Area | State |
|---|---|
| `.GPK` archive reading | **Done** — all 69,936 entries across 30 packs decode and verify |
| Archive key recovery | **Done** — read from the user's own executable, not embedded |
| `.ORS` script format | **Decoded** — 14 commands, documented in `docs/FORMATS.md` |
| `.CMAP` UI hit maps | **Decoded** |
| `FONTDATA.DAT` glyph store | **Decoded** — all 22,420 glyphs render |
| Video / audio decode | **Done** — matches ffmpeg's own output |
| Playback / windowing | In progress |
| UI rendering | Not started |
| Route / branch graph | **Blocked on reverse engineering** — see below |
| Save file compatibility | Not started |

## Building

You need a Rust toolchain (pinned by `rust-toolchain.toml`, rustup installs it
automatically) and two native libraries from your distribution:

```bash
# Arch
sudo pacman -S sdl3 ffmpeg clang
# Debian / Ubuntu
sudo apt install libsdl3-dev libavcodec-dev libavformat-dev libavutil-dev \
                 libswscale-dev libswresample-dev clang pkg-config
```

Then:

```bash
just build       # or: cargo build --locked --workspace
just check       # fmt, clippy, tests
just audit       # cargo-deny: advisories, licenses, bans, sources
```

Every recipe passes `--locked`. If a build tells you the lockfile needs
updating, that is deliberate — see `docs/DEPENDENCIES.md`.

## Design constraints

**No bundled game data.** Anything derived from the game is derived at runtime
from the user's own files. The GPK decryption key is read out of their
`SCHOOLDAYS HQ.exe` rather than hardcoded here, and the route graph will be
extracted from their `RouteProcSDHQ.dll` the same way.

**The game's own UI.** FILMEngine's interface is fully data-driven: each screen
is a base PNG, a `_CHIP` sprite sheet of widget states, and a `.CMAP` — a
per-pixel region-ID map used for hit testing, shipped at all four of the game's
resolutions (800x600, 800x450, 1024x576, 1280x720). DaysEngine reimplements that
renderer rather than substituting its own widgets, so the interface is the
original one.

**Small, pinned, audited dependency set.** See `docs/DEPENDENCIES.md`.

## The hard part

The branch graph is not in the scripts. `.ORS` files are linear timelines:
`[Next]` carries only an end timecode and `[SetSELECT]` carries only the two
choice labels, with no targets. Routing lives compiled inside
`RouteProcSDHQ.dll` (1,825 script names, exports `GetNextScriptFile`,
`SetScript`, `SetFeeling`, `GetStory`).

The plan is an offline extractor that reads *the user's own* DLL and emits a
route-graph JSON on first run, so the logic is recovered from their install
rather than redistributed by us.

## Layout

    crates/days-gpk    GPK archive reader + minimal PE resource parser
    crates/days-vfs    one case-insensitive namespace over all 30 packs
    crates/days-script .ORS timeline parser
    crates/days-media  WMV3 / Vorbis decoding over the system ffmpeg
    crates/days-font   FONTDATA.DAT glyph store
    crates/days-cli    `days` — offline inspection tools
    docs/FORMATS.md   reverse-engineered file format notes
    docs/DEPENDENCIES.md  dependency policy and audit checklist

## Legal

DaysEngine contains no game assets, no game code, and no decryption key. It is
an independent reimplementation of a file format and a runtime. You must own a
copy of the game to use it.
