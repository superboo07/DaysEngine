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

    cd "/path/to/School Days HQ" && ./daysengine

That reads the start script out of the game's own `STARTSCRIPT.INI` and plays
it. Space pauses, Left/Right seek five seconds, R restarts, Esc quits. Name a
script to play that one instead (`./daysengine 01-00-A00`), and pass
`--game <dir>` if you would rather keep the binaries elsewhere.

`./days` is the companion inspection tool — `key`, `list`, `extract`, `verify`,
`scripts`, `script`, `assets`, `media`, `timing`, `font`, `render`.

## Status

Early. What works today:

| Area | State |
|---|---|
| `.GPK` archive reading | **Done** — all 69,936 entries across 30 packs decode and verify |
| Archive key recovery | **Done** — read from the user's own executable, not embedded |
| `.ORS` script format | **Decoded** — 14 commands, documented in `docs/FORMATS.md` |
| `.CMAP` UI hit maps | **Decoded** |
| `_CHIP` sprite atlases | **Decoded** — the widget table is recovered from the user's own `SysMenuSDHQ.dll` by content, not by a hardcoded address |
| `FONTDATA.DAT` glyph store | **Decoded** — all 22,420 glyphs render |
| Video / audio decode | **Done** — matches ffmpeg's own output |
| Playback / windowing | **Plays a scene** — video, audio, timeline, dialogue |
| UI rendering — title, menubar, options, replay grid, backlog, route maps | **Composites** — the game's own art, at all four resolutions; `days ui` renders any screen headlessly |
| UI rendering — save/load, replay play-data list | Not started — their slot rows are laid out by a loop at runtime, so there is no table to recover; choosing them leaves the menu where it was |
| UI input handling / screen state machine | **Works** — title, settings and replay are live: pointer and keyboard, each screen's own widget-to-action table out of the DLL, both popups, and the mode graph out of `SystemInit` |
| Settings | **Works** — `Config.DAT` is read and written back, volumes reach the mixer, and the Option screen's three tabs drive it |
| Replay | **Works** — the 41 scenes, their unlock flags and their scripts are recovered from the user's own `SysMenuSDHQ.dll`; picking one plays it. Chained replay playback is not implemented |
| Text box, word wrap, backlog | Not started |
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

## Inspecting an install

`days` runs from inside the game directory (it finds the game by sitting in it,
or takes `-g <dir>`):

```bash
days list System --filter title      # what is in a pack
days script 00-00-A00                # parse one script's timeline
days render 00-00-A00 --at 00:39:00 -o /tmp/frames
days ui System/Title/Title -r full --active 1 -o /tmp/title.png
days menu -e "down,down,enter" -o /tmp/menu.png    # drive the menus headlessly
days config                          # the player's settings, as the Option screen reads them
days replay                          # the replay scene table, and what the save has unlocked
```

`days ui` composites a UI screen without a display, the way `days render` does
for playback, so the UI can be checked as an image diff on a machine with no
GPU. `--table` prints the widget-to-sprite table recovered from the DLL instead
of drawing.

`days menu` replays a script of menu events — `down`, `up`, `left`, `right`,
`enter`, `esc`, `at:X:Y`, `click:X:Y` — against the real screens and reports
where each one lands. Every decision the menus make happens there, so the state machine is
testable on a machine with no GPU even though the SDL player draws through one.

## Layout

The engine is one ordinary crate. Only the readers for the game's shipped file
formats are split out, because those are the parts another project could use on
their own — a modding tool wanting the archives has no business pulling in SDL
and ffmpeg to get them.

    src/              the engine: vfs, media, screen compositing, mixer,
                      timeline stage, text layout, menus, INI, save lookup
    src/main.rs       `daysengine` — the game
    src/bin/days.rs   `days` — offline inspection tools

    crates/days-gpk    GPK archive reader + minimal PE resource parser
    crates/days-script .ORS timeline parser
    crates/days-font   FONTDATA.DAT glyph store
    crates/days-save   Save/GlobalFlag.DAT flag store
    crates/days-ui     CMAP hit maps and _CHIP atlas recovery

    docs/FORMATS.md   reverse-engineered file format notes
    docs/DEPENDENCIES.md  dependency policy and audit checklist

`src/media` is the one module that uses `unsafe`, and every occurrence is a call
into the system libav — Rust requires the keyword on all FFI. The crate root
denies `unsafe_code` and that module carries the only `allow`.

## Legal

DaysEngine contains no game assets, no game code, and no decryption key. It is
an independent reimplementation of a file format and a runtime. You must own a
copy of the game to use it.
