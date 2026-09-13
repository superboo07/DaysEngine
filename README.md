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
| UI rendering — replay play-data list | **Works** — the player's own save slots, three columns a row, the expanded comment on hover; picking one plays that save back by the answers it recorded |
| UI input handling / screen state machine | **Works** — title, settings and replay are live: pointer, keyboard and controller, each screen's own widget-to-action table out of the DLL, both popups, and the mode graph out of `SystemInit` |
| Settings | **Works** — `Config.DAT` is read and written back, volumes reach the mixer, and the Option screen's three tabs drive it |
| Replay | **Works** — the 41 scenes, their unlock flags, their scripts and the branch tables eleven of them walk are recovered from the user's own `SysMenuSDHQ.dll`; picking one plays it through, following the player's choices |
| In-game control bar | **Works** — a drop-down over the top 75 pixels, translucent over the frame, ramping in over 300ms and out over 1000ms exactly as the original does; all 25 widgets, their enabled rules, their resting and hover art and their captions, out of the DLL's own dispatch, reachable by pointer or by a controller selection that walks the live ones; pause, the auto flag and restart act; the five rate buttons fast-forward, scaling the script clock and retiming the audio with it — resampled at 1x, 2x and 4x and muted above, which is `FUN_004433d0`'s own rule. The buttons that move to the next script hand over to the branch graph. Three of the four menu buttons open the screen they ask for — save, load and Option; the third asks for a code `setSystemInit` has a case for but whose screen is **not recovered**, so that one button is the one this engine cannot answer. The right-hand box is the replay-mode indicator's transparency slider: its ten cells set how solid the `REPLAYMODE` sign on the picture is drawn, and the whole box lights up only while playback is following a save's recorded answers. `days bar` prints the table |
| Choice boxes (`[SetSELECT]`) | **Works** — raised and decided on the script clock, so an ignored choice still times out; the shipped hit maps where they exist and the game's own screen split where they do not, pointer, keyboard and controller — the four navigation slots the original's own `+0x148` carries and `FUN_0044de50` reads — and a random pick while skipping, as the original does. `days select` prints the map and metrics |
| Subtitles | **Works, the game's own way** — broken by `FUN_0043f600` (62 columns, word-wrapped at spaces, English only, `\n` as a hard break, ruby marks recognised), spaced by the recovered pitch and kerning table rather than by measuring the glyph, and placed by `FUN_0044bf30`: centred on each line's own width, anchored to the bottom, at the per-resolution scale, with `[LeftArrangement]` switching to a left-aligned block. The speaker name is not drawn, because the original never hands it to the text layer, and the whole block is behind the `TextView` setting |
| Text box art, backlog | Not started |
| Route / branch graph | **Recovered** — the 55 routes, their 1,857-entry script tables and all 55 transition state machines come out of the user's own `RouteProcSDHQ.dll`, the tables by content and the machines by decoding the handlers, with no address embedded. Scripts chain: a choice moves the player through the graph and credits what it earns. `days route --edges` prints every edge and checks the graph against the tables |
| Affection gauge | **Works** — the five counters, both tables, the relative test that 25 routes branch on and the 13 absolute thresholds, and the gauge on the control bar drawn from the game's own art: the strip that slides green towards Kotonoha and orange towards Sekai, and the two full-length bars either side takes once it is more than 83 points ahead. A delta raises it and it runs the original's ramp — `SeUp` or `SeDown` for the direction the counter moved, then a slide to the new lead over 1.5s, a 2s hold and down again — opaque throughout, over a bar that has otherwise faded away, and carrying on across the cut into the next scene the way the original does. `days route` shows a save's counters and which way the test falls; `days bar --feeling 001,002` draws the gauge at any pair, and `--gauge --feeling-was` runs the ramp between two pairs |
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
days replay                          # the replay scene table, its branch tables, and what the save has unlocked
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
; libavfilter chains over every movie frame, before and after the scale
Filters = deblock=filter=weak:block=8,deband=r=16,gradfun=1.2:16
FiltersAfterScale =
; A very light grain over the finished frame, in levels of 255; 0 is off
Grain = 2

[UI]
; pixel (the default), bspline, mitchell, catmull_rom
Scaler = pixel
; Whole-number scaling: no resampling at all, at the cost of a border
PixelPerfect = off

[Input]
; Every control, rebindable. A trigger is a key by SDL's name for it, a
; pad button (`pad:a`), or a pad axis pushed one way (`pad:-lefty`).
Confirm = return, keypad enter, space, pad:a
Cancel  = escape, pad:b
; Confirm with nothing focused pauses, so Pause only needs its own button
Pause   = pad:start
; ...and seventeen more; `days settings` lists them all

[Rumble]
; How much of the level a `MoveSom` asks for reaches the motor, as a
; percentage. 0 turns it off.
Strength = 100
```

A first run writes the file out with every default already filled in, and every
run after that writes in anything the file does not yet mention — a section it
has never heard of arrives as the whole commented block, a missing key as one
line inside the section it belongs to. That is additive and only additive:
nothing is reordered, reworded or re-valued, so a file written by an older build
gains the twenty rebindable actions without losing a thing. A value it cannot
read is a line in the log and nothing more. `days settings
--template` prints the same copy to standard output, and `days settings` says
which file is in force and what it currently means.

### Playing it with a controller

The original is keyboard and mouse, and it has no input API at all — the menu
DLL imports none. A controller is this engine's own addition, and it is here
because a game that can only be played with two hands on a keyboard and a mouse
cannot be played by everyone.

The d-pad or the left stick moves the selection, A confirms, B backs out. During
playback A pauses: the bar is a strip the pointer hovers rather than something
that holds a selection, and most of a script has no choice box up, so a confirm
with nothing to confirm pauses — one rule, and the reason Space has always done
it. The shoulders seek, the triggers change speed, and Up (or the right stick
button) puts the selection on the control bar — a strip the
original can only be reached with a pointer, so without this half of it a
controller would leave most of the game out of reach. The right stick drives a
pointer for anything a selection cannot land on.

Every one of those is a line in `[Input]`, and naming an action replaces its
list rather than adding to it, so a player who wants one binding gets one
binding. `days settings` prints the table in force.

### Rumble, which is the game's own feature

The scripts carry `[MoveSom]` statements — 281 of them across 46 scripts — with
five intensities for SOMCON, a toy the game drives over a COM port. The device
is long discontinued and its protocol is proprietary to it, but the intensities
are levels, and `FUN_00438470` maps them to a fifth of full scale apiece. A
rumble motor takes exactly that.

So the levels go to a controller. The switch is the game's own: the Option
screen's SOMCON tab, unchanged — nine `Port number` buttons, a find button, a
release button and the `SOMCON test` — where a port is now a connected
controller. Every recovered condition is followed, including the two that are
easy to miss: the device is silent while playback is suspended, and silent
above 1x. `[Rumble] Strength` is the only knob that is ours.

The engine talks to it through one trait, `playback::som::Device`, which is the
five operations the menu DLL's serial object has. An
[Intiface](https://intiface.com/) backend — where the toys this was written for
still live — is another implementation of it and no change anywhere else.

The two scalers are separate because the two jobs are. `[Video] Scaler` is the
picture — movie frames *and* still backgrounds, both through libswscale. A
movie frame is scaled inside the colour conversion it already goes through, so
the filter costs only its own width, and a still goes through the same library
with the same filter: the two are ways of filling the same 800x452 stage, and a
still that was filtered differently did not match the clip it cut to.
`[UI] Scaler` is the menus and the control bar, which go through the engine's
own resampler in `playback::scale`.

### Filtering the movies

The movies are WMV3 at 800x452 and about 3 Mbit/s, and on a modern window every
frame is blown up more than twice — the 8x8 transform blocks and the banded
gradients the encoder left come up with it. Two libavfilter chains and a dither
are the answer, and which side of the scale each one runs on is the point:

* `Filters` runs **before** the scale, at the clip's own size, where those
  artifacts are still the size the encoder made them. A debander cannot
  recognise a band the scaler has already stretched. The default is `deblock`
  for the blocks and then both debanders, because they fail differently:
  `gradfun` fits a gradient and repairs a shallow ramp, `deband` replaces a
  pixel from references a radius away and breaks a step. On a close-up of dark
  hair, where this encode's banding is worst, the fraction of the frame in flat
  runs of eight pixels or more goes 5.9% unfiltered, 5.1% with `gradfun` alone,
  2.7% with `deband` alone, and 1.5% with both.
* `FiltersAfterScale` runs **after** it, on the frame at the size it will be
  seen. Anything *added* to the picture belongs here; laid down earlier it
  comes out magnified along with everything else. Empty by default.
* `Grain` is a very light neutral dither over the finished frame — the engine's
  own, because libavfilter's `noise` on a packed RGBA frame means a conversion
  to planar and back, gives each colour an independent pattern, and with `alls`
  noises the alpha channel too. It covers the last of the banding: what the
  debander judged too wide to touch, and the contouring the upscale adds.

1080p is the size this is built for, and there the whole default pipeline is
about 7ms of a frame's 41: 10.4ms a frame becomes 18.5, measured with `days
media <clip> --at-size 1920x1085`, which is what that command is for. Above
that it is the scale itself that costs, not the filtering — 31ms of the 41 at
3840x2170 before a filter runs at all. Empty chains and `Grain = 0` give the
original's path exactly.

### The pixel filter

The default for the game's own art is not a cubic. The art is 800x450 of flat
colour and hard edges, a 1920x1200 window wants 2.4x of it, and a cubic answers
that by blending everywhere — which is what "soft" means here.

`Scaler = pixel` is the **band-limited pixel filter**, which is what gamescope's
`GamescopeUpscaleFilter::PIXEL` is: `sampleBandLimited` in its
`src/shaders/composite.h`. It warps the bilinear phase so the inside of a source
pixel comes out exactly its own colour and only the boundary between two of them
is blended, over about the one output pixel that straddles it. Crisp like point
sampling, but with the edge band-limited to the output grid instead of falling
wherever rounding puts it — so it works at **any** scale, with no border and no
stair-stepping. `playback::scale::band_limited` has the derivation, and a test
checks it against the shader's own formula.

It runs **once**. A screen is composited straight at the size the window will
show it — `Screen::fit_to` — and not into its hit map's size and then again onto
the window, because filtering twice band-limits the edges onto one grid and then
re-bands them onto another, which is exactly the artefact this filter exists to
avoid. `days ui --at-size 1920x1080` and `days menu --at-size 1920x1080`
composite and hit-test the way the player's window does.

### Pixel-perfect

The game is authored at 800x450 and ships nothing larger, so on a modern window
every pixel of it becomes more than one. Fitting a 1920x1200 panel wants a scale
of 2.4, and the 0.4 is where softness comes from: two source pixels in five fall
between destination pixels, and no filter can do anything about that but blur
across the gap.

`PixelPerfect = on` is the other answer: scale by a whole number — the largest that fits,
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
