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
| UI rendering — save/load | **Works** — the game's own LOAD/SAVE screen, its ten rows, ten page buttons and route-map button, from the player's art; picking a row loads or writes that slot. Its chip table needed the atlas matcher to believe a long exact run rather than a majority of regions, because ten of its rows are one sprite behind two hit regions each — and that one sprite is what both halves light, so pointing at a comment highlights the whole row. The ten records that are three rows tall are not hover art at all: they size the panel of the expanded-comment tooltip, which works — pointing at a row opens its whole comment over the list, wrapped the way the DLL wraps it, opening upwards on the last two rows so it cannot run off the bottom. The comment column and the tooltip are both gated on `FILMENGINE.INI [TextInput]`, which is what host `+0xd8` answers. Naming a save works: the original opens a Win32 dialog from the executable's own resources, and this draws that template — caption, prompt, button captions and every rectangle out of the player's exe — with SDL text input behind it so an IME works. Every row shows its timestamp, chapter and comment: the DLL rasterises all thirty columns into one 2048x1024 off-screen surface and gives each column a sprite that cuts it, and both the cut and the placement are recovered — including the fact that the second band of ten records is not a duplicate of the first but where the comment goes |
| UI rendering — replay play-data list | Not started — choosing it leaves the menu where it was |
| UI input handling / screen state machine | **Works** — title, settings and replay are live: pointer and keyboard, each screen's own widget-to-action table out of the DLL, both popups, and the mode graph out of `SystemInit` |
| Settings | **Works** — `Config.DAT` is read and written back, volumes reach the mixer, and the Option screen's three tabs drive it |
| Replay | **Works** — the 41 scenes, their unlock flags and their scripts are recovered from the user's own `SysMenuSDHQ.dll`; picking one plays it. Chained replay playback is not implemented |
| In-game control bar | **Works** — a drop-down over the top 75 pixels, translucent over the frame, ramping in over 300ms and out over 1000ms exactly as the original does; all 25 widgets, their enabled rules, their resting and hover art and their captions, out of the DLL's own dispatch; pause, the auto flag and restart act; the five rate buttons set the rate the bar draws but do not yet fast-forward, because the decoders run at their own rate and scaling only the timeline would run it ahead of the audio. The buttons that move to the next script hand over to the branch graph; which menu each one opens is not recovered. `days bar` prints the table |
| Choice boxes (`[SetSELECT]`) | **Works** — raised and decided on the script clock, so an ignored choice still times out; the shipped hit maps where they exist and the game's own screen split where they do not, pointer and keyboard, and a random pick while skipping, as the original does. `days select` prints the map and metrics |
| Subtitles | **Works, the game's own way** — broken by `FUN_0043f600` (62 columns, word-wrapped at spaces, English only, `\n` as a hard break, ruby marks recognised), spaced by the recovered pitch and kerning table rather than by measuring the glyph, and placed by `FUN_0044bf30`: centred on each line's own width, anchored to the bottom, at the per-resolution scale, with `[LeftArrangement]` switching to a left-aligned block. The speaker name is not drawn, because the original never hands it to the text layer, and the whole block is behind the `TextView` setting |
| Text box art, backlog | Not started |
| Route / branch graph | **Recovered** — the 55 routes, their 1,857-entry script tables and all 55 transition state machines come out of the user's own `RouteProcSDHQ.dll`, the tables by content and the machines by decoding the handlers, with no address embedded. Scripts chain: a choice moves the player through the graph and credits what it earns. `days route --edges` prints every edge and checks the graph against the tables |
| Affection gauge | **Recovered** — the five counters, both tables, the relative test that 25 routes branch on and the 13 absolute thresholds, plus the gauge's own geometry. `days route` shows a save's counters and which way the test falls. The gauge's three sprites are not composed: their source rectangles are not recovered |
| Save data | **Works, both ways** — `Save/SaveFileNNN.DAT` is a log of where the player is, every story point they reached with the state they reached it in, and the choice they made at every script. Read and written, along with `GlobalFlag.DAT` and the line the save screen shows. Every one of the 22 files in the test install reads and writes back **byte for byte identical**, so a save this engine writes is a save the original game reads. `days save --slot N` decodes one; `days save --roundtrip` is that check |

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
`SCHOOLDAYS HQ.exe` rather than hardcoded here, and the route tables are read
out of their `RouteProcSDHQ.dll` the same way.

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
`RouteProcSDHQ.dll`, which the executable defers to entirely.

Half of it turned out to be data. Progress is two integers in the save,
`ROUTE` and `SCENE`, and each of the 55 routes has an array of script names
indexed by `SCENE`. Those arrays are found in the user's own DLL by content —
runs of pointers to strings shaped like a script path — and cross-check exactly
against the code, against each other and against the script pack.

The other half is not data. Each route's "given this scene and the player's
choice, which scene next" is a compiled `switch` with the answers as immediate
operands, spread over 55 functions, and there is no table of edges anywhere in
the file. So `crates/days-route` **decodes them**, out of the user's own DLL, at
run time. It finds the 55 handlers through the PE export table and the export's
own dispatch switch, then walks each one symbolically with `SCENE` fixed to the
scene being asked about — which folds away the two different shapes the
compiler gave those switches and leaves only the branching that depends on
something a save knows. Anything outside the decoder's instruction subset is
reported as not recovered rather than guessed past.

It reproduces a Ghidra decompilation of all 55 handlers exactly, arm for arm,
across 1,840 of them. All 1,857 scenes get a transition, no edge names a scene
no table has, and every scene is reachable from the first. `SetFeeling` — a
second, separately compiled 55-way switch that decides what a choice earns —
decodes independently and agrees with the branch graph on 1,426 of the 1,435
pairs where they overlap. The nine that differ are a bug in the shipped game:
at three choices it credits the branch you did not take, which the engine
reproduces. Nothing about any of it is embedded here.

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
days settings                        # DaysEngine's own settings, and where they come from
days media Movie00/00-00/00-00-A00/00-00-A00-001 --at-size 1920x1085  # can this machine hold 24 fps?
days replay                          # the replay scene table, and what the save has unlocked
days save --slot 0                   # decode a save slot: position, story points, choices
days save --roundtrip                # read every save file, write it back, compare bytes
days dialog -o /tmp/dlg.png          # the save-comment dialog, from the exe's own template
days bar --pointer 400,40 -o /tmp/bar.png   # the control bar, widget by widget
days render 00-00-A00 --at 00:39:00 --bar -o /tmp/frames   # ...over a real frame
days select "I'm happy" "This is bad" --at 0.5,0.75   # a choice box's map and hit test
days route                           # the 55 routes, the affection tables, the save's counters
days route 00-00-A04                 # where one script sits, what follows it, what it credits
days route --edges                   # every recovered edge, and the graph's own cross-checks
```

`days ui` composites a UI screen without a display, the way `days render` does
for playback, so the UI can be checked as an image diff on a machine with no
GPU. `--table` prints the widget-to-sprite table recovered from the DLL instead
of drawing.

`days bar` prints every control-bar widget with its box, whether the engine's
own rules make it live, its caption and what it asks the host for, and renders
the strip as the engine composites it — as an RGBA layer, with `--pointer`
driving the drop-down and `--after` catching the fade part way through.
`days render --bar` blends that layer over a real playback frame, which is what
shows whether the translucency is right. `days select` reports which of the six
shipped hit maps a resolution really gets, the boxes in it, the label wrap
limits, and where a normalised point lands.

`days menu` replays a script of menu events — `down`, `up`, `left`, `right`,
`enter`, `esc`, `at:X:Y`, `click:X:Y` — against the real screens and reports
where each one lands. Every decision the menus make happens there, so the state machine is
testable on a machine with no GPU even though the SDL player draws through one.

## Settings of our own

Everything the engine reads is the player's, with one exception:
`DaysEngine.ini`, **beside the `daysengine` binary**. It holds the choices the
original never had to make, because it handed its frames to Direct3D and took
whatever filter the driver gave:

```ini
[Video]
; fast_bilinear, bilinear, bicubic, lanczos, spline, gaussian, neighbour, area
Scaler = bicubic

[UI]
; Whole-number scaling: no resampling at all, at the cost of a border
PixelPerfect = off
; bspline, mitchell, catmull_rom — one cubic family, softest first
Scaler = bspline
```

The file is optional, never written by the game, and a value it cannot read is
a line in the log and nothing more. `days settings --template` prints a
commented copy to start from, and `days settings` says which file is in force
and what it currently means.

The two scalers are separate because the two jobs are. A movie frame is scaled
by libswscale inside the colour conversion it already goes through, so the
filter costs only its own width; the game's own art goes through the engine's
resampler in `playback::scale`. A libavfilter graph — a debander before the
scale, say — is **not implemented**; the decoder is where it would go.

### Pixel-perfect

The game is authored at 800x450 and ships nothing larger, so on a modern window
every pixel of it becomes more than one. Fitting a 1920x1200 panel wants a scale
of 2.4, and the 0.4 is where softness comes from: two source pixels in five fall
between destination pixels, and no filter can do anything about that but blur
across the gap.

`PixelPerfect = on` scales by a whole number instead — the largest that fits,
centred, with a border around the rest. 1920x1200 takes ×2, so the game draws at
1600x900 and every source pixel becomes the same exact 2x2 block. Nothing is
resampled: the composite goes to the GPU at its own size and is point-sampled
into an exact multiple, which is the one case where point sampling invents
nothing. `[UI] Scaler` is unused while it is on, because nothing runs it.

This is not nearest-neighbour scaling, which is what turning the filter off at
2.4 would give — that lands some source pixels on two destination pixels and
some on three, and looks worse than either. Movies are still filtered on their
way into the same box; they are photographic and they want it.

The mode also takes the **native** art set rather than the one the display mode
names, which is a documented departure from the recovered rule in
`Resolution::for_display`. The four sets are one layout at four sizes — the
1280x720 hit map is the 800x450 one scaled by 1.6, and there is only ever one
`.PNG` behind them — so composing at 1.6 and then multiplying by a whole number
would put a resample back in the middle of the one path whose point is not
having one.

## Layout

The engine is one ordinary crate. Only the readers for the game's shipped file
formats are split out, because those are the parts another project could use on
their own — a modding tool wanting the archives has no business pulling in SDL
and ffmpeg to get them.

    src/install/      the player's install: packs, INI, settings, save lookup
    src/media/        audio and video decoding through system ffmpeg
    src/playback/     timeline stage, mixer, lip sync, text layout, compositor
    src/ui/           the game's menus: title, save/load, option, replay
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
