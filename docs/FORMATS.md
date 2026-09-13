# FILMEngine file formats

Reverse-engineered notes for the formats used by School Days HQ. Everything here
was derived by inspecting a retail install; no game data is reproduced.

Sizes below are from the English School Days HQ release (`SCHOOLDAYS HQ.exe`,
Aug 2012): 30 packs, 69,936 entries, ~12 GB.

---

## `.GPK` — STKFile0 archive

See the module docs in `crates/days-gpk/src/lib.rs` for the byte layout. The
points worth repeating:

- The index is XOR'd with a repeating 16-byte key and then zlib-compressed. The
  key ships as a `CODE` / `CIPHERCODE` resource inside the game executable.
- **The inline header trap.** Each entry's index record ends with a `header_len`
  byte and that many bytes of payload. Those bytes are the *first* bytes of the
  entry's compressed stream and they live only in the index — the data region
  holds `size - header_len` bytes at `offset`. Extractors ported from GARbro
  read `size` bytes at `offset` and prepend the header, which overruns into the
  next entry. Entries still inflate (zlib stops at the stream end) but the
  extraction is wrong in general and the offsets no longer chain.
- Entry names are UTF-16LE with `\` separators. **Case is inconsistent between
  packs**: `Ini.GPK` stores `DX9GRAPHIC.INI` while the `.INI` files reference
  `System/Title/TitleBase.png`. All lookups must be case-insensitive.
- `method` is a FourCC; `"DFLT"` (0x544C4644 LE) is the only one observed.
  `unpacked_size == 0` means stored uncompressed.
- Archives begin with a leading stub before the first entry's data (5,120 bytes
  in `Ini.GPK`); never assume data starts at 0.

### Packs

`Ini`, `Script`, `System`, `SysSe`, `BGM`, and then six chapters each of
`Event0N` (PNG stills), `Movie0N` (WMV), `Se0N` and `Voice0N` (OGG).

---

## `.ORS` — script

Plain text, one statement per line, blank-line separated:

    [Command]=<start>\t<arg>\t...\t<end>;

Timecodes are `MM:SS:FF` at **24 fps** — confirmed against the movies, which are
24 fps, and against `[Next]` totals. Files are UTF-8 (English) or UTF-16LE
with a BOM (Japanese); `FILMENGINE.INI [UnicodeFile]` selects this.

Exactly 14 commands exist across all 1,857 scripts:

| Command | Arguments after start timecode | Count |
|---|---|---|
| `PrintText` | speaker, text | 30,486 |
| `PlayVoice` | path, male-voice flag, speaker tag | 30,413 |
| `CreateBG` | `BGS`, image path | 13,024 |
| `PlaySe` | slot (1-5), path | 3,552 |
| `PlayMovie` | path, loop flag | 2,042 |
| `SkipFRAME` | *(one timecode)* — where the skip control jumps to | 1,857 |
| `Next` | *(one timecode)* — end of script | 1,857 |
| `PlayBgm` | path | 1,488 |
| `BlackFade` | `IN` / `OUT` | 847 |
| `SetSELECT` | choice 1 label, choice 2 label (or `null`) | 287 |
| `MoveSom` | intensity (int) | 281 |
| `WhiteFade` | `IN` / `OUT` | 272 |
| `EndBGM` | path | 67 |
| `EndRoll` | movie path | 67 |

Notes:

- Every statement carries both a start and an end timecode, so the script is a
  **timeline**, not a program counter. The engine is closer to a video editor's
  EDL than to a VN bytecode interpreter.
- `PlaySe` slot 5 is sometimes handed a `Voice...` path — the game reuses the SE
  mixer for voice that is not meant to drive a mouth overlay.
- **166 voice references point at clips that do not exist**, out of 50,653 asset
  references across all scripts. They cluster on `PlayVoice` statements whose
  male-voice and speaker-tag fields were left blank, which reads as a scripter
  marking "no clip for this line". One background PNG is likewise absent
  (`Event01/01-00/01-00-T00/01-00-T00-009`). A missing asset must not be fatal.
- `MoveSom` drives a toy; the retail engine no-ops it without hardware.
- **`Next` and `SetSELECT` carry no targets.** The branch graph is not here.
- `PrintText`'s text field is not plain: it may carry a `\n` **escape** — two
  characters, a backslash and an `n` — as a hard line break, and the ruby marks
  `｜` and `《…》`. Only 68 of the 30,485 statements use the escape and **none of
  the English ones use ruby**. Everything else is wrapped by the engine; see
  *Dialogue layout* below.

### Retail data quirks the parser must absorb

All 1,857 scripts parse once these are handled. Each was found by running the
parser over the real packs, not by reading the format spec:

| Script | Quirk |
|---|---|
| `05-KC-F00` line 241 | A **semicolon inside dialogue** (`I know; I am, too.`). Statements have no escaping, so a `;` only terminates when the next non-whitespace character is `[` or the file ends. |
| `05-KI-OP1` | Written with **`, ` separators instead of tabs**, plus a stray whitespace-only ` ;` statement. Fall back to comma splitting only when a statement contains no tab, so commas in ordinary dialogue stay literal. |
| `03-KB-D10` line 29 | A `PrintText` with a **trailing empty field**. |
| `05-SE-C08` line 81, and 129 others | `PlayVoice` with **empty male-voice and tag fields** but the tabs still written. Fields must be read by position with defaults, not matched against an exact arity. |
| `01-00-E01` | Two timecodes with a **frame field of 26** in a 24 fps script. Fold the overflow in rather than rejecting. |

### The male-voice flag

`PlayVoice`'s second field is a 0/1 flag for "this line is a male character",
not a lip-sync switch. The engine stores it on the voice object
(`FUN_0044e3b0`, at `+0x48`) and `FUN_0044e800` refuses to start the clip when
the flag is set and `SysMenuSDHQ.dll`'s `GetMenVoice` export returns zero. That
export is a thunk to `FUN_100070c0`, which returns offset `+0xa8` of the menu
config object — the field `FUN_10006ce0` loads from and `FUN_10006e40` saves to
the `MenVoice` key of `Config.DAT`, and that the Option screen's widgets 10 and
11 set.

The data agrees: across all 1,857 scripts the flag is 1 for `mak` (9,848 lines)
and `tai`, and 0 for `sek`, `kot`, `hik` and the rest. A handful of tags carry
both values, `xxx` narration most of all.

Nothing in the statement switches lip sync on or off. The speaker tag alone
decides, by whether the current background ships overlays for it.

### Lip sync — mouth overlays

In a still-background scene the engine flaps a speaker's mouth by patching
three small PNGs into the background. They sit in the `EventNN` pack beside the
background frame they belong to, named `<background stem><TAG>.<A|B|C>.PNG`:

```text
Event00/00-00/00-00-A02/00-00-A02-001B.PNG        the background
Event00/00-00/00-00-A02/00-00-A02-001BMAK.A.PNG   Makoto's mouth, closed
Event00/00-00/00-00-A02/00-00-A02-001BMAK.B.PNG   open
Event00/00-00/00-00-A02/00-00-A02-001BMAK.C.PNG   wide
```

Every overlay is a full 800x452 RGBA canvas, almost entirely transparent. The
set belongs to the **background**, not to the voice: `-001`, `-001B` and `-001C`
each carry their own, sometimes for a different speaker, so a background change
mid-line changes which mouths are available. 8,923 sets ship across the six
`EventNN` packs.

The background object is a `FILMOBJ::ImageChar` (vftable `0x004d535c`,
constructed at `FUN_00443900`), and the rules are:

- **Registration.** The dispatcher `FUN_00438de0` calls vtable slot `+0x90` on
  the current `BGS<n>` object with the statement's speaker tag; its `[CreateBG]`
  arm walks the live voice list and registers every tag already speaking on the
  new background, which is what carries a mouth across a background change.
- **Ownership.** The three images are loaded onto the background object's own
  slots, so they live and die with it. `FILMOBJ::MovieChar` carries ten slots of
  its own at `+0xe4` (`FUN_0044a2c0`) and nothing in the retail install fills
  them: no `MovieNN` pack holds a single `.A`/`.B`/`.C` overlay. A movie
  therefore has no mouths — it does not inherit the background's, and a player
  who sees one over a movie is looking at a bug. 483 of the shipped scripts
  start a movie while a tagged line is still speaking, so the case is common.
- **Path.** `FUN_004453e0` (slot `+0x90`) appends the tag to the background's
  own path, then `FUN_00444f70` appends `.A`, `.B` or `.C` and `.png` (literals
  at `0x004d5268`, `0x004d5270`, `0x004d5278`). The tag goes on exactly as the
  script spells it — lowercase — and the case-insensitive pack lookup finds the
  uppercase name. If any of the three is missing the tag goes into a
  per-background reject set and that speaker never flaps on that background;
  `xxx` narration lines simply have no art and so do nothing.
- **Patch rectangle.** `FUN_00445240` derives it from the `.A` image's alpha,
  and not as a bounding box: it takes the first opaque pixel in raster order as
  the corner, counts the opaque pixels on that row for the width, and counts how
  far that column stays opaque for the height. `FUN_00444b80` then copies that
  rectangle into the background's surface with `memcpy` — no alpha blending —
  and it does so *into that surface*, before anything composites or scales it.
  The patch and the face around it are one image from then on, which is what
  keeps them on one pixel grid however large the window is.
  Both only work because the shipped overlays are a solid opaque rectangle on a
  transparent canvas, which the real files confirm: the scan yields exactly
  x=392 y=160 48x43 for `00-00-A02-001BMAK`, matching the true opaque region,
  and every alpha byte is 0 or 255.
- **Cadence.** `FUN_00444cf0`, called from the object's update `FUN_00443e30`,
  keeps ten slots (one per tag) with a phase counter. The counter steps on every
  frame whose number is divisible by three — 8 Hz, phase-locked to the engine
  clock rather than to the line — and the image is the counter modulo three. It
  steps only while the clip is audible, with one exception: once it has left
  `.A` it keeps stepping through silence until it comes back round to `.A`, so
  the mouth always closes rather than freezing open. When the voice object is
  gone it snaps to `.A`.
- **Audible.** `FUN_0041ba90` builds the flags as the clip decodes, appending
  one per tick and calling a tick silent when a single 16-bit sample lies in
  `-59..=60`. `FUN_0041a830` looks a flag up by elapsed frame converted to 100ns
  units by `FUN_00428140`, whose divisor `DAT_0050c468` is the engine-wide 24 —
  written once at `0x0044a623`, and the same constant the `MM:SS:FF` parser
  multiplies by.

**Which sample the retail build tests is not reproducible.** `FUN_0041ba90`
indexes relative to the decoder's current packet
(`packet_bytes * n / 23 - consumed_samples`), so the answer depends on how the
shipped Ogg reader chunks the stream; the same expression also advances the tick
counter at 23 Hz while labelling the entries at 24. `src/playback/lipsync.rs` samples at
the frame's own position and keeps the threshold.

`FILMOBJ::MovieChar` carries the same ten slots at `+0xe4` (`FUN_0044a2c0`), but
no `MovieNN` pack in the retail install contains a single `.A`/`.B`/`.C`
overlay, so nothing can drive it.

---

## `.CMAP` — UI hit map

```text
width   u32 LE
height  u32 LE
pixels  [u8; width * height]   region ID, 0 = no region
```

One byte per pixel naming the widget under it. Hit-testing is a pixel lookup,
not a geometry test, which is what lets the route map screens use non-rectangular
widgets. Region IDs are dense from 1.

Each screen is `NAME.PNG` (base art), `NAME_CHIP.PNG` (widget state sprites) and
`NAME*.CMAP`, shipped at four resolutions by filename suffix:

| Suffix | Size | Scale |
|---|---|---|
| *(none)* | 800x600 | 1.0, offset 75px |
| `_WIDE` | 800x450 | 1.0 |
| `_WIDE_NOTE` | 1024x576 | 1.28 |
| `_WIDE_FULL` | 1280x720 | 1.6 |

### The UI is authored at 800x450

`_WIDE` is the native layout and the other three are it transformed. The
`_WIDE_NOTE` and `_WIDE_FULL` maps are the 800x450 map scaled by 1.28 and 1.6;
the 4:3 800x600 map is the same 800x450 content **offset 75 pixels down**,
between letterbox bars. Region 1 of `TITLE` is at `(112, 353)` in `_WIDE`,
`(112, 428)` in 4:3 (`353 + 75`), `(143, 452)` in `_WIDE_NOTE` (`x1.28`) and
`(179, 565)` in `_WIDE_FULL` (`x1.6`).

The engine does not need those constants. Both fall out of comparing the two
maps the user already has:

```text
scale     = display_map.width / native_map.width
letterbox = (display_map.height - native_map.height * scale) / 2
```

which gives 75px for 4:3 `TITLE` and 0 for the 800x75 `MENUBAR` strip with no
special case for either. This also settles the playback geometry: stills are
800x450, movies are 800x452 (encoder rounding), and 4:3 letterboxes both.

### The art is filtered on the way up, not point-sampled

The maps come in four sizes but the art does not: there is one `NAME.PNG` and
one `NAME_CHIP.PNG` per screen, authored at 800x450, and a 1024x576 or 1280x720
screen is that art scaled by 1.28 or 1.6. `FUN_0044a3d0` is what the engine does
about that. It is the render-state init, and for each of the eight sampler
stages it calls `SetSamplerState` (`IDirect3DDevice9` vtable `+0x114`) with:

| `D3DSAMPLERSTATETYPE` | value |
|---|---|
| `D3DSAMP_ADDRESSU` (1) | `D3DTADDRESS_CLAMP` (3) |
| `D3DSAMP_ADDRESSV` (2) | `D3DTADDRESS_CLAMP` (3) |
| `D3DSAMP_ADDRESSW` (3) | `D3DTADDRESS_CLAMP` (3) |
| `D3DSAMP_MAGFILTER` (5) | `D3DTEXF_LINEAR` (2) |
| `D3DSAMP_MINFILTER` (6) | `D3DTEXF_LINEAR` (2) |

So every sprite the menus draw is bilinear-filtered by the GPU on its way to the
screen, with the texture edge clamped rather than wrapped. Scaling the art by
taking the nearest source pixel instead is not what the game looks like: at 1.6x
it duplicates every other row and column, which steps the diagonals and hardens
the text. This engine resamples the art with `daysengine::playback::scale`
instead — a cubic B-spline rather than the driver's bilinear, and its edge clamp
is the `ADDRESSU`/`V` above.

---

## `_CHIP` sprite sheets — the widget table in `SysMenuSDHQ.dll`

`NAME_CHIP.PNG` holds *replacement* art for individual widgets, drawn over the
base. On the title screen the base is the logo and five plain text labels; chip
row 1 is those same five labels inside a rounded button frame (the selected
state) and row 2 holds a greyed-out `REPLAY` (the disabled state).

**Nothing in the data says which sprite belongs to which widget.** The packing
is not recoverable from the sheet: `MENUBAR`'s 25 region widths sum to 920
against a 799-wide sheet, in four widget sizes, and its widget 1 draws from
`src_y = 67` while widgets 2..15 draw from rows 1 and 29.

The mapping is a table of 32-bit floats in `SysMenuSDHQ.dll`, **six per widget,
24 bytes**:

```text
dst_x  dst_y  width  height  src_x  src_y
```

in 800x450 space, one record per region ID in order. The screen setup walks it
and per record calls a destination-rect setter with
`(dst_x, dst_y + letterbox, width, height) * scale` and a source-rect setter
with `(src_x, src_y, width, height)`. Destination and source share one size, so
a chip sprite is never scaled relative to its widget.

Records after the per-region run are alternate states — disabled art, alternate
captions — reached by fixed address from code, so how many there are and what
each means is per-screen knowledge the bytes do not carry.

### How this was established

By decompiling `SysMenuSDHQ.dll`, not by studying the sheets. The DLL holds the
path strings for every UI screen; `Ghidra` decompilation of the title setup at
`0x10020140` shows the `0x18`-stride walk, and the tables themselves live in
`.data` from about `0x1004cc30`. Reading the bytes back as six floats reproduces
the `.CMAP` region boxes exactly, which is the cross-check.

Relevant addresses in the retail DLL (imagebase `0x10000000`):

| Address | Role |
|---|---|
| `0x10020140` | title screen setup — the authority for the record layout |
| `0x10020cf0` | re-places the same records on a resolution change |
| `0x100207a0` | title action table: entry 0..4 -> Start, Load, Replay, Option, Exit |
| `0x100206e0` | per-entry enabled test — entry 2 (Replay) is conditional |
| `0x1001ff10` | picks the `.cmap` for the current resolution and loads it |
| `0x1004cc30` | `TITLE` widget table; `0x1004ccc0` `TITLE_AC`, `0x1004cd68` `TITLE_CLEAR` |
| `0x1004ce20` | `MENUBAR` widget table |

### Finding the table without hardcoding an address

The engine does **not** embed those offsets. It searches the user's own DLL for
the table by content: the first four floats of record *i* are the bounding box
of region *i + 1* in the screen's native `.CMAP`, so taking the boxes from the
user's `.CMAP` and looking for records that reproduce them at a 24-byte stride
finds the table and validates it at the same time.

The search takes the longest run of records it can anchor at region 1, carries
on from wherever that run stops, and repeats — see *A table is not always one
run* below for why one run is not enough. Within a run:

- An anchor is an **exact** box match, so a run can never be started by the
  looser rule that extends it.
- A run extends while each next record either matches its region's box exactly
  or sits inside it. A region's box can legitimately differ from its sprite rect
  — regions that abut have their boxes clipped by the neighbour. The route map's
  episode tabs do this, hence e.g. 11/15 boxes matching a table that is correct.
- A region no run reproduces is one laid out at runtime. Its record is taken at
  the stride from the nearest run instead; the save/load screen's slot rows are
  why that exists.
- Below a threshold — 3 exact matches, a majority of regions, and an average of
  at least 3 regions per segment — the search reports failure rather than
  drawing sprites from a wrong offset.

### Screens whose layout is not in a table

`SAVELOAD` and `REPLAY_PLAYDATA` are refused by the search, correctly: their
slot rows are laid out by a loop at runtime — ten rows at a 33px pitch — so
those rects exist nowhere in the binary. Those two screens need their layout
reproduced in code. Every other screen recovers: title (all three variants),
menubar, all three options pages, both backlogs, the exit popup, the replay
popups, `REPLAY_HSCENE` and all 15 route maps.

### A table is not always one run

`TITLE` and the three `OPTION` screens keep one record per region, in region
order, back to back. `REPLAY_HSCENE` does not: its nineteen regions are three
separate stretches of one larger table — headers and back button, then the four
page buttons eight records later, then the twelve thumbnails eight records after
that — because the records in between are those widgets' other states.

Insisting on a single run there does not fail cleanly. It lands on a stretch
that reproduces fifteen of the nineteen boxes, passes the match threshold, and
draws every sprite from the wrong offset — a table that fits the bytes and is
wrong. The search is therefore segmented: longest run anchored at region 1,
continue from where it stops, repeat. Screens whose table really is one run come
out as one segment. A screen that needed a segment per region would be matching
individual records anywhere in the DLL, so that is refused.

Segmentation fixes the anchor, not everything. `REPLAY_HSCENE`'s page buttons
appear in the table three times over with **identical destination rectangles**
and different source rows — resting, hovered, current page — so geometry alone
cannot say which row a widget's hover sprite comes from. Only `FUN_1001a460`
can, and that is per-screen knowledge rather than something the search can
derive.

A few screens do not name their base art after the stem, because they share one
chip sheet and hit map across several backgrounds: `Exit/Popup` uses
`Popup_Exit.png` or `Popup_Title.png`, and `SaveLoad/SaveLoad` uses `Save.png`
or `Load.png`.

---

## The menu state machine — `SysMenuSDHQ.dll` exports

The menus are not a tree the engine walks. The executable asks the DLL for a
**mode** — one small integer — and `_SystemInit@8(mode, &module)` turns that
number into one of eight singleton screen modules, so the entire menu graph is
a single `switch`. Mode 1 is the one value `SystemInit` has no case for, which
is exactly why it means "stop showing menus and start playing".

| mode | module object | screen assets |
|---|---|---|
| 1 | — | not a menu: play the script |
| 2 | `0x1004ff08` | `System/Title/{Title,Title_AC,Title_Clear}` |
| 3 | `0x1004f350` | `System/SaveLoad/SaveLoad`, base `Save.png` / `Load.png` |
| 4 | `0x1004fb00` | `System/Option/Option_{Def,Sound,SomCon}` |
| 5 | `0x1004f620` | `System/Replay/Replay_%s` |
| 6 | `0x1004e7c0` | `System/RouteMap/%02d/RouteMap%02d` |
| 7 | `0x1004f2b0` | `System/Option/Pop_Som` |
| 8 | `0x100506b0` | `System/Replay/Pop_Replay_%s` |
| -1 | `0x1004f570` | `System/Exit/Popup` |

Those objects live in `.data` and are built by C++ static initializers, so the
DLL *file* holds no vtable pointer for them and Ghidra reads `.data` as zeros.
They were named instead through the CRT static-init thunks, which name each
class's constructor, and then by the `System/...` path literals in the
neighbouring code: mode 2 is `FUN_1001fc80`, 3 `FUN_100111f0`, 4 `FUN_10005ba0`,
5 `FUN_10019c10`, 6 `FUN_1000c0c0`, 7 `FUN_1001f3a0`, 8 `FUN_10018b50`, and
-1 `FUN_1000a380`.

`_getNextMode@8(mode, module)` reads back where to go, defaulting to 2 — the
title — whenever a screen simply finishes:

    3  picked a save -> 1,  -> 6 route map, cancelled -> -1, else -> 2
    4  -> 7 som popup,      cancelled -> -1,             else -> 2
    5  -> 1 or -> 8 popup,  cancelled -> -1,             else -> 2
    6  cancelled -> -1,     else -> 3 or -> 1
    7  always -> 4
    8  -> 5 or -> 1

Cancelling does not leave directly: it opens mode -1, the confirm popup. That
popup is also why `SystemInit` ends with
`if (mode != -1 && mode != 7 && mode != 8) FUN_100018b0(&popup, mode)` — it
records the screen currently being opened at popup member `+0xa4`, so when the
popup is later raised it knows both which question to ask and where to return.
From the title that is `Popup_Exit.png`, "End this game?"; from anywhere else
`Popup_Title.png`, "Return to Title Screen?". `_getNextMode@8(-1)` just returns
`+0xa4`.

The popup's own dispatch (`FUN_1000a8f0`) settles which of its two regions is
which: **widget 0 is YES**, which records the affirmative answer at `+0xa0`,
and **widget 1 is NO**, which returns the player to the remembered mode.

### Title

`FUN_10020140` picks the variant and the widget count together, and the count
is literally `all_clear + 5`:

    all_clear            -> Title_AC     6 widgets, table 0x1004ccc0
    else trial           -> Title        5 widgets, table 0x1004cc30
    else route 0 cleared -> Title_Clear  5 widgets, table 0x1004cd68
    else                 -> Title        5 widgets, table 0x1004cc30

Each table is followed by exactly **one** further 24-byte record — the greyed
`REPLAY` caption, at `0x1004cca8`, `0x1004cd50` and `0x1004cde0` respectively,
which is each table's base plus `count * 0x18`. This is the ground truth behind
the warning in [`days_ui::Atlas::extras`]: only the first trailing record
belongs to the screen, and the run after it is the next screen's table.

Clicking a widget runs `FUN_100207a0`, gated by the enablement switch
`FUN_100206e0`:

| widget | label | mode | enabled when |
|---|---|---|---|
| 0 | START | 1 | always |
| 1 | LOAD | 3 | always |
| 2 | REPLAY | 5 | not a trial build **and** route 1 cleared |
| 3 | OPTION | 4 | always |
| 4 | EXIT | -1 | always |
| 5 | *(commentary)* | 1 | all-clear only, so `Title_AC` only |

Keyboard navigation (`FUN_10020910`) wraps `0..=4` and, when it lands on a
disabled `REPLAY`, keeps moving the way it was already going — so the locked
entry is stepped over in both directions and never selected. Widget 5 is
outside that range: it is reached by pointer, or by entering from outside
`0..=4`. Selection lives at `+0xb0` and starts unset.

### Option — `MENU::ConfigMenu`, mode 4

One class, three sets of art, chosen by `+0x184` (0 `Def`, 1 `Sound`, 2
`SomCon`) and reloaded by `FUN_100076c0`. `Option_SomCon` swaps its background
for `Option_SomCon_Set.png` once a serial port is held. Widgets 0, 1 and 2 are the
tab headers and widget 3 is the close button on all three; widget 3 flushes the
settings and then tells the host to leave the menus with mode 0, whose meaning
is **not recovered**.

Dispatch is `FUN_10007e80`, switching to `FUN_10007ef0` / `FUN_10008260` /
`FUN_100089a0`. Enablement is `FUN_10007ca0`, which delegates the same way; the
only widget it can disable outright is header 2, hidden in a trial build.

| Tab | Widgets | What they are |
|---|---|---|
| Def | 4,5 | aspect: wide / 4:3 |
| | 6,7 | window mode: window / full screen |
| | 8,9 | `Skip` on / off |
| | 10,11 | `SuperSkip` on / off |
| | 12,13 | `TextView` on / off |
| Sound | 4,5 | `BgmVolume` − / + |
| | 6,7 | `SeVolume` − / + |
| | 8,9 | `VoiceVolume` − / + |
| | 10,11 | `MenVoice` on / off |
| | 12,13 | `Mute` on / off |
| | 14–23 | `BgmVolume` = widget − 13, so 1 to 10 |
| | 24–33 | `SeVolume` = widget − 23 |
| | 34–43 | `VoiceVolume` = widget − 33 |
| SomCon | 4,5 | find a port / let it go |
| | 6–15 | `Port number` 1 to 10 |
| | 16,17 | `SOMCON test` start / stop |

The arrows step by one and clamp at 0 and 10 (`FUN_10007140`), so only they
reach silence — the cells start at 1. The Sound tab's ten cells have **no chip
sprite**: its record table declares only 14 per-widget sprites, and the level is
drawn instead as one record stretched to the width `FUN_100070e0` computes,
from the first cell's left edge to the right edge of the cell at the current
level.

The Def tab's two display rows do not change anything themselves. Each checks
the host (`+0xb8` aspect, `+0xbc` full screen) and, if the mode would really
change, sets a flag: `+0xac` for full screen and `+0xb0` for wide.

The executable reads them back through the module's **exports**, which is why a
sweep of the DLL's own code finds writes and no reads. `_GetFullFlag@0` returns
`+0xac` and `_GetWideFlag@0` returns `+0xb0`, and `_SetFullFlag@4` /
`_SetWideFlag@4` write them. Each getter has exactly one caller in the
executable, and both are one-shot appliers polled from the main loop:

    FUN_004279e0   if (_GetFullFlag@0()) {
                       FUN_00429b10 / FUN_0042a030      release the surfaces
                       FUN_0040e860(!FUN_0040e830())    toggle the window
                       if (engine+0x220) FUN_0042c520   re-place the screen
                       _SetVistaDisplay@4(0 or 1)
                       _SetFullFlag@4(0)                clear the request
                       FUN_00429b40 / FUN_0042a080      rebuild
                   }

    FUN_00427a90   if (_GetWideFlag@0()) {
                       FUN_0040ed10()                   apply the layout
                       if (engine+0x220) FUN_0042c520
                       _SetWideFlag@4(0)
                   }

So each flag is a **request**, not a setting: the engine acts on it once and
clears it. Both appliers toggle rather than assign, which is safe only because
the widgets fire nothing when the value is already in force.

`_GetResFlag@0` is **not** part of this, despite the name. It returns the popup
module's `+0xa0`, which `FUN_1000a8f0` sets to 1 on the popup's YES and 0 on its
NO — it is the confirm popup's answer.

The mark showing which value is in force is a per-tab switch — `FUN_10009fd0`,
`FUN_1000a190`, `FUN_1000a250` — indexing a run of records that begins 27, 51
and 35 records into each tab's table respectively (`+0x190`). `FUN_1000a190`
tests `widget == 10` in **both** of its first two arms, so the mark for
`MenVoice` is stuck on widget 10 and widget 11 never gets one; that is a bug in
the shipped DLL and the engine reproduces it. The SOMCON tab's selected port is
reported as the current value but draws no sprite at all.

Keyboard navigation is a hand-written transition table per tab —
`FUN_10008da0`, `FUN_100092d0`, `FUN_100098a0` — on `+0x50`/`+0x54`/`+0x58`/
`+0x5c` for up/down/left/right. The Sound tab's never visits the level cells.

See `daysengine::options`.

### SOMCON — `MENU::SomconSet`

SOMCON is the peripheral toy the game can drive, not a gamepad. The tab's own
art says `Port number` and `SOMCON test`, and the DLL imports no input API of
any kind — no DirectInput, nothing. What it imports is `CreateFileA`,
`GetCommState`, `SetCommState`, `GetCommTimeouts`, `SetCommTimeouts`,
`SetCommMask`, `WaitCommEvent`, `ClearCommError`, `GetOverlappedResult`,
`ReadFile` and `WriteFile`. The toy is a **serial device** and a "port number"
is a COM port.

`FUN_10021070` picks from a table of nine ASCII names — `COM1` to `COM9`, eight
bytes apart in `.rdata` — and opens it:

```text
CreateFileA(name, GENERIC_READ|GENERIC_WRITE, 0, NULL, OPEN_EXISTING,
            FILE_FLAG_OVERLAPPED|FILE_ATTRIBUTE_NORMAL, NULL)
DCB:            9600 baud, 8 data bits, no parity, 1 stop bit
COMMTIMEOUTS:   500 / 10 / 500 / 10 / 500
SetCommMask:    EV_RXCHAR
```

Then a two-command ASCII protocol, each followed by a read of up to 256 bytes
of reply:

| Command | Written by | Meaning |
|---|---|---|
| `s%02x` | `FUN_100214d0` | set level, hex. The `SOMCON test` sends `0x96`; taking a port sends `0x00`. |
| `b` | `FUN_100215d0` | stop |

`FUN_10007850` is the search behind the tab's find button: it tries port
indices 0 to 8 in turn, opening each and sending `s00`, and keeps the first
that answers.

**Nothing in this engine drives a toy, and nothing is planned to.** The tab is
a working screen and `UseSOM` is stored with the rest of the settings, but no
port is opened and no byte is written. The protocol above is proprietary to one
discontinued device; if this engine ever moves a toy it should do it through
[Intiface](https://intiface.com/) rather than reimplement this. It is recorded
here because recovering it is what established that SOMCON is not an input
device — not because it is a plan.

One mismatch is the DLL's own: the screen has ten `Port number` buttons and the
port-name table has nine entries. The tenth button selects index 9, and the
string that follows `COM9` in `.rdata` is the `s%02x` format itself, so the
shipped build would ask `CreateFileA` to open a file called `s%02x` and fail.
This engine refuses the tenth button rather than reproducing an out-of-bounds
read.

### Replay — `MENU::SceneView`, mode 5, and `MENU::SceneCheck`, mode 8

`+0x2b0` chooses `Replay_HScene` (the thumbnail grid) or `Replay_PlayData`.
On the grid, widgets 0 and 1 are the tab headers, widget 2 is back, widgets 3
to 6 are the four page buttons and widgets 7 to 18 are twelve thumbnails —
`page * 12 + widget - 7` is the scene (`FUN_1001de10`). Forty-one scenes over
forty-eight slots, so the last page is short.

Three `.data` runs carry the table, none of them at an address this engine
knows; they are found by content:

- a run of 41 pointers to UTF-16 **save-flag names** like `REP02_2S_W03`. A
  thumbnail is live exactly when that flag is set: the DLL asks the host's
  `+0x18`, which is `FUN_00428770`, a lookup in `Save/GlobalFlag.DAT` by name.
- a run of 41 pointers to per-scene **script lists** like `02/02-2S-W03`. Three
  of them — scenes 11, 22 and 30 — point into zero-filled `.data` and carry
  nothing, because those scenes ask a question first.
- a run of 8 pointers to **version flags**, each a scene's own flag plus a
  trailing letter (`REP03_KB_N00A`…`D`), and a matching run of 8 script lists.

A scene's flag implies its script path: `REPnn_XX_Ymm` is `nn/nn-XX-Ymm`. That
rule holds for all forty-one and agrees with all thirty-eight table entries that
exist, and it is used to *place* the script window, because the scene run and
the version run are adjacent in `.data` and read as one.

Clicking a thumbnail hands the scene's **first** script to the host (`+0xa4`).
The rest of a list are the steps after it, fetched by `FUN_1001f0d0`, which
asks `FUN_1001ee20` for the next step index and reads the name out of the
scene's list, ending the scene on -1 and on the list's NULL terminator.

`FUN_1001ee20` is the branch:

```text
column = host->+0x4()                      // the choice made in playback
column = (column == -2) ? 0 : column + 1
switch (scene at +0x2a8) {
  case 1, 6, 7, 10, 11, 12, 16, 25, 28, 29, 36:
      next = table[step * 3 + column]      // 0xc bytes a row, three wide
  default:
      next = step + 1                      // straight down the list
}
```

Eleven scenes have a table; scene 11 has four, one per version, picked by the
version index at `+0x2b4`. A table is one row per script in the scene's list and
a row is three `i32` step indices — column 0 for a player who has answered no
choice box, columns 1 and 2 for the two answers.

The column comes from the host's vtable slot `+0x4` (`FUN_0042c080`), which
reads the film object's `+0x1f8` through `FUN_0042c0c0`. That member is written
in exactly two places in the executable: `FUN_004388c0` sets it to -2 when the
film object is constructed, and `FUN_0043f330` stores the index a choice box
settled on — -1 when the box was dismissed or ran out of time (`FUN_00431740`
resolves it). Nothing puts it back, so **the value outlives the script it was
made in and the scene that reads it**: a replay's first step is walked by
whatever the player last answered, possibly long before the replay started. Both
writes were found by a byte scan of `.text` for stores at displacement `0x1f8`;
the other hits belong to other classes.

A branch table is a run of small integers and cannot be found by its contents
the way the runs above are. It is found by the code that reads it: `FUN_1001ee20`
is a dense MSVC switch — a byte map from scene index to case, a table of case
addresses — whose arms are all the same five instructions.

```text
MOV  r1, [EBP+this]
MOV  r2, [r1 + 0x2ac]          the step index
IMUL r2, r2, 0xc               twelve bytes a row
MOV  r3, [EBP+column]
MOV  r4, [r2 + r3*4 + table]   four bytes a column
```

The engine scans the image for a switch of that shape and takes the table
address out of each arm; scene 11's arm is a second switch on `+0x2b4` whose own
arms have the same shape. Exactly one switch in the retail DLL matches, and it
yields the eleven scenes and the four version tables the decompile shows. Each
table is then read as one row per script and refused unless every entry is -1, a
step its list has, or the one past the end that reads as the list's NULL
terminator — all fourteen pass, and each one's length lands exactly on the next
table or on the alignment padding before it.

Scenes 11, 22 and 30 raise `Pop_Replay` instead (`FUN_10001830`, which sets
`+0xc8` and `+0xc4` on the popup singleton). `+0xc8` picks `Pop_Replay_2` or
`Pop_Replay_4` — two or four versions. A version is pickable only once its own
flag is set (`FUN_100195d0`), and picking one sets the step index to zero and
plays element zero of its list (`FUN_1001f270`) — which is the **same script in
every version**, so the choice cannot change what starts. For scene 11 it
selects which of four branch tables the rest of the scene walks; scenes 22 and
30 have no table and differ only in the list the version names.

The thumbnails themselves come from `System/Replay/Replay_Thm%02d.png`, one
sheet per page, page number plus one (`FUN_1001b3f0`). A second record table
holds the twelve slot rectangles **twice over** — the first twelve are what a
selected slot draws, the last twelve what every live slot draws — and that
doubled shape is what tells it apart from the chip sheet's run, which
reproduces the same twelve boxes once and then carries on.

The grid's keyboard transition table is `FUN_1001e3a0`. It is decompiled but
**not transcribed**: one arm guards the horizontal move on `c % 4 != 0` over
widgets 8 to 17, which blocks the second column rather than the last, and
widgets 7 and 18 fall through every arm. Until that reads consistently the grid
gets a plain walk over its widgets rather than a table that looks recovered and
is not.

`Replay_PlayData` is the save/load list over again. `FUN_1001b6d0` asks the host
`+0x9c` — `FUN_0042a980`, the very call the save/load screen uses — for each
slot of the page, so the rows are the player's own hundred save slots, ten to a
page over ten pages, `page * 10 + row`. The three columns are the timestamp, the
chapter and the comment, rasterised into one 2048x1024 surface and cut by a
sprite each, and the comment column is gated on host `+0xd8`
(`FILMENGINE.INI [TextInput]`). Every layout constant is the same global the
save/load screen reads, at the same width.

Its records are the same table the grid's are, at `DAT_1004c430`:

| records | what |
| --- | --- |
| 0 .. 2 | the two tab headers and CLOSE |
| 3 .. 6 | the tabs marking the view showing, and that with the pointer on it |
| 7 .. 16 | the ten page buttons |
| 17 .. 36 | the page button marking the page showing, and that with the pointer on it |
| 37 .. 46 | the row bars, 799x31 — the highlight, and where the timestamp and chapter go |
| 47 .. 56 | the comment column, 472x97 — three rows tall because it doubles as the expanded comment's panel |

The hit map reproduces none of the row records: a row is two hit regions, the
left band (widgets 3 to 0xc) and the comment band (widgets 0x17 to 0x20), each
half the width of the record. So the table is anchored on the thirteen records
that *are* reproduced — the three at the top and the ten page buttons — at their
own indices, and the rows are read off from there. The tabs alone will not do
it: `REPLAY_PLAYDATA` and `REPLAY_HSCENE` share their first seven records byte
for byte and sit back to back in `.data`.

Either band picks the same row (`FUN_1001dfe0`) and lights the same full-width
bar (`FUN_1001a670`); only the comment band raises the expanded comment
(`FUN_1001a060` calls `FUN_1001bc80(this, selection - 0x17)`). Nothing on the
screen is ever greyed out — `FUN_1001dd80` answers true for every widget — and a
row whose slot has no file simply does nothing.

Three things it does **not** do that the save/load screen does: `FUN_1001c850`,
`FUN_1001b6d0` and `FUN_1001bc80` never ask host `+0x5c`, so no column is
shifted for English, no comment is centred, and every column is cut at twenty
characters. It also rasterises two pixels above where it cuts — the pen is
`row * 0x30` and `row * 0x30 + 0x200` while the sprites cut at `row * 48 + 2`
and `row * 48 + 514`.

Picking a row hands the host `+0x48(slot)` — the same load the save/load screen
asks for — then `+0x94(1)` and `+0x4c(8)`. `+0x94` (`FUN_0042bf10`) raises the
flag `+0x98` reports, and `FUN_00431740` reads it at every choice box: a box
nobody answers takes `FUN_00428a80`, the answer that slot recorded at the script
in play, and the moment the player answers one themselves `+0x94(0)` puts the
flag down. The same flag gates the write — `FUN_00428a50` runs only with it
clear — so a followed playthrough does not overwrite the recording it is
reading. That is what "replay data" means: play a save back by its own answers.

See `daysengine::replay`.

### The system sounds, and the index they are asked for by

The seven sounds `FILMENGINE.INI` names are stored by the executable as seven
separate `std::wstring` members on a `0x1c` stride, written by `FUN_00422170`
at `base + 0x5a0 + 0x1c * n` in the INI's own declaration order:

| index | key | index | key |
|---|---|---|---|
| 0 | `SeCancel` | 4 | `SeDown` |
| 1 | `SeSelect` | 5 | `SeView` |
| 2 | `SeClick` | 6 | `SeOpen` |
| 3 | `SeUp` | | |

Host vtable slot `0x50` — `FUN_00429c80` — is how the menu modules and the
in-game UI ask for one, and it is a seven-arm switch reading
`this + 0x5a4 + 0x1c * index`: the same run, at the same stride, in the same
order. The four-byte difference is the host subobject offset: `base` is the
host plus four, which falls out twice, once from the member run lining up and
once because `FUN_00422170` hands `base + 0x304` to the choice box that
`FUN_00431740` reaches as `engine + 0x334` with the host installed at
`engine + 0x2c`. The switch has no default arm, so an eighth index would play
whatever the path pointer last held.

Three call sites agree with the names: the control bar plays index 2 on any
live click, and the choice box plays 1 when a choice is taken and 0 when it is
declined or times out. Which index each menu *screen* plays is per-screen and
still open; what `daysengine::ui::menu` plays where is this engine's choice.

Of the three questions the title asks the host, only one is answered out of
`Save/GlobalFlag.DAT`. Route *n* cleared (`+0xec`, `FUN_0042baf0`) reads the
`EndClear` flag together with object `+0x21c`. All-clear (`+0xe8`) and trial
build (`+0x34`) are **plain members**, `+0x7a4` and `+0x7a8`, that the only
constructor anything calls zeroes and nothing else writes — so both are always
false in the retail build, and `Title_AC` is unreachable. See the Title section
above and `daysengine::menu::SaveState::from_flags`, where the reasoning is kept
next to the code that depends on it.

---

## The in-game UI — the control bar and the choice box

Neither of these is one of `SystemInit`'s modes. They are the UI the engine puts
*over* playback, and they are owned differently from the menus and from each
other.

### `System/MenuBar` — the control bar

The art and the 25-region widget table are in `SysMenuSDHQ.dll` like any other
screen, but the module is not in the mode switch. `_SetMenuBar@4` is a one-line
export — `*param = &DAT_10050790` — so the executable is handed a pointer to a
static `FILM::MenuBar` (constructor `FUN_100216e0`, which writes the vtable at
`0x1003d804`) and drives it through that object's own slots:

| slot | role |
|---|---|
| `+0x08` | release textures |
| `+0x0c` | load the cmap and chip sheet, lay the sprites out |
| `+0x10` | re-place on a resolution change |
| `+0x14` | draw — `FUN_10024ca0` |
| `+0x1c` | take the renderer and host pointers |
| `+0x20` | update: hit test, fade, dispatch — `FUN_10024100` |
| `+0x2c` | re-place the play/pause widget — `FUN_100258f0` |
| `+0x30` | widget 2's action — `FUN_10025b90` |
| `+0x34` | widget 0's action — `FUN_10025cf0` |

Widget index `n` is region `n + 1` and table record `n`. `FUN_10024100`'s
dispatch, `FUN_10023fb0`'s enabled test and `FUN_100262e0`'s caption switch all
bracket the 25 into the same twelve groups:

| widgets | asks the host for | live when |
|---|---|---|
| 0 | `+0x120`, flip the auto flag and save the settings | always |
| 1 | `+0xf4`, toggle pause | always |
| 2 | `+0xfc(1)`, then `+0xfc(2)` on a second press | always |
| 3 | `+0xfc(2)` | always |
| 4 | `+0x12c(1)` then `+0xfc(5)` | `!+0x104 && !+0x110 && SuperSkip` |
| 5..9 | `+0x8c(0..4)`, the playback rate | `!+0x110 && +0x88` |
| 10..12 | `+0xf8(4)`, `+0xf8(5)`, `+0xf8(3)` | `!+0x104` |
| 13 | `+0xf8(2)` | always |
| 14 | `+0x100(1)` | always |
| 15..24 | `FUN_10026ed0`, the replay indicator's transparency | `+0x98` |

A press on a widget that is not live is swallowed *and silent*: the dispatch
asks the enabled test before playing SE index 2.

### What widget 4 actually skips to

`+0xfc` (`FUN_0042a4a0`) does not move the timeline itself. It queues a request:
state 4 with the code in `+0x2a8`, which the engine's state machine
`FUN_00425bf0` picks up. Its case 2 routes code 5 — and only code 5 — to
**case 6**; codes 1 and 2 go to case 3, and everything else ends the script.

Case 6 compares two members of the timeline object:

| Member | Getter | Written by | Meaning |
|---|---|---|---|
| `+0x22c` | `FUN_004315c0` | `[SkipFRAME]` in `FUN_0043b640` | the skip target |
| `+0x21c` | `FUN_004315a0` | `[Exit]` / `[Next]` in `FUN_0043b640` | the end of the script |

If the target is ahead of the clock (`+0x208`) and not equal to the end, it
seeks to **`+0x22c` less `DAT_0050c468`** — `0x18`, 24, set at `0x0044a623` —
clears the skip flag with `+0x12c(0)` and carries on playing. So the button
lands one second *before* the choice, and the run-up plays rather than the box
appearing out of a cut.

Otherwise there is nothing to skip to in this script, and it seeks to the end
and falls into **state 7** — which is the other half of the button, and is
reachable from nowhere but case 6, so this chaining is the skip and never an
ordinary end of script. State 7 asks `_GetNextScriptFile@12` for what follows
and loads it, then compares the **new** script's `+0x22c` and `+0x21c`:

- they differ — the new script raises a choice — so it positions at
  `+0x53c + 1`, the new script's own start, clears the skip flag, sets
  `+0x5ca` (which suppresses the intro effect in case 4 and routes case 0 to
  case 2) and goes to state 4. The script plays from its beginning, *not* from
  its choice.
- they are equal — no choice there either — and nothing assigns `+0x224`, so
  the state stays 7 and the next iteration loads the script after it.

So the button chases a choice across scripts, passing over whole ones without
playing them. In chapter 1 a press during the opening runs `00-00-A00` →
`A01` → `A02` and stops at `A03`, the first of route 0's 21 scenes that raises
one.

The chase is gated by host `+0x88` (`FUN_00427490`): when that answers 0 the
new script is played whether or not it has a choice.

The target is the choice. Across all 1,857 retail scripts, `[SkipFRAME]` equals
`[Next]` in the 1,570 that raise no choice, and in all 287 that do it is exactly
the `[SetSELECT]` start — no exceptions either way. So `[SkipFRAME]` is **not**
the script's length, which is `[Next]`; the two coincide only when there is
nothing to skip to.

`+0x12c` (`FUN_0042c000`) sets the skip flag at the engine's `+0x5c9`, which
`FUN_004401c0` reads back. `FUN_0043b640` only records `[SkipFRAME]` into
`+0x22c` while that flag is set.

**`+0x88` is not the `Skip` setting**, though it starts with it.
`FUN_00427490` is

    if (_GetSkipFlag@0() == 0) return host->+0x18(engine + 0x188);
    else                       return 1;

and host `+0x18` is `FUN_00428770`, a lookup of that path in the pack index the
engine keeps at `+0x3c`. `engine + 0x188` holds **the script being played**:
`FUN_00423a70` takes the next entry off the pending queue at `+0x154`, stores it
there and hands the same string to the timeline loader `FUN_00430d20`, and
`FUN_00425bf0` state 7 refills it from `_GetNextScriptFile@12` when a script
chains. A playing script resolves, so `+0x88` is true and the speed row and the
skip button are live for every player — the setting is only a short circuit
ahead of the lookup. The one case that answers false is no script loaded at
all.

**The rate table is `1, 2, 4, 12, 24`** — `DAT_004f99f0`, indexed by host slot
`+0x8c` in `FUN_00424f90`. The English chip sheet labels the last two buttons
`▶×16` and `▶×32`; the art is not the authority.

`FUN_00424f90` does three things in order, and the order is the behaviour.
Pressing the rate already in force is the one case that does none of them: it
compares against `engine + 0x530` first and, when they match, writes only the
lit index at `+0x534`. Otherwise it suspends playback (`FUN_00424910`, which folds
the frames run so far into the base with `+0x540 = +0x544` and stops the sound
with it — see below), stores the rate as a float at `+0x538`, and starts it
again (`FUN_00424a10`). An index
outside the table is clamped to 1x at index 0 rather than ignored —
`(param_1 < 0) || (4 < param_1)` stores `0x3f800000` and rewrites the index.

The rate is what the clock runs on. `FUN_00422f70` reads the frame playback is
at as

    frame = +0x540 + ROUND((timeGetTime() - +0x550) * DAT_0050c468 * +0x538) / 1000

so the base frame plus the elapsed wall time scaled by the rate, in whole
milliseconds, with `+0x544` holding a high-water mark so the frame never goes
backwards. Re-basing on every rate change is what keeps time already played
from being re-scaled.

`FUN_00424f90` retimes the audio with the picture, and **mutes it above 4x**.

It hands the rate to two things. The movie object gets it through
`FUN_00431c90`, that object's vtable slot `+0x38`. The script's audio stream
gets it through `FUN_00429500`, which passes it to `FUN_004433d0` on the sound
object at `engine + 0x30c`. `FUN_004433d0` does two things with it:

* it forwards the rate to the stream with `FUN_0041a050`, which is a bare rate
  message (`0x8005`) on the sound object. Nothing time-stretches, so the
  original simply resamples and the pitch rises with the speed.
* before that, it compares the rate against the **double** at `0x004d5080` —
  the instruction is `FCOMP double ptr`, and the eight bytes there are `4.0`.
  Above it, `+0x40` is latched and `FUN_00443650(this, 1)` mutes the stream; at
  or below it, the stream follows whatever `_GetMute@0` says.

So of the five rates in the table the first three are heard and **12x and 24x
are silent**, which is also why dropping from 12x or 24x back below 4x re-primes
the media through `FUN_00431cd0` — guarded on the old index being above 2, the
new one below 3, and `engine + 0x22c` clear. Read as a 4-byte float those eight
bytes are `0.0`, which would mute every rate; the operand width is the
authority.

The engine keeps two script sound streams, `engine + 0x304` and
`engine + 0x30c`, both taking their volume from `_GetMasterVolume@4` category 2
or 3 (`FUN_10006fd0` reads the Option module's `+0x9c` for 2 and returns a fixed
level 2 for 3). Only `+0x30c` is rate-adjusted. **Which of the two is which has
not been recovered**: nothing that opens them with a path has been found, and
the `+0x304` that `FUN_00422170` sets up is a different base — that one is
`base + 0x304` with `base = engine + 0x30`, the choice box.

### Suspending playback stops its sound, and only playback resumes it

`FUN_00424910` and `FUN_00424a10` are a refcounted pair — `InterlockedIncrement`
and `InterlockedDecrement` on `engine + 0x230`, doing their work on the
transition to and from 1 — wrapped by `FUN_00424e20` and `FUN_00424eb0`, which
hold the critical section at `engine + 0x234` and latch `engine + 0x22c`.

`FUN_00424910` pauses **everything the script owns**: the two streams at
`engine + 0x304` and `+0x30c`, and, through the timeline object at
`engine + 0x1e4` (`FUN_00431c30` -> `FUN_0043edd0`), its eight
`FILMOBJ::BgmSound` slots at `+0x39c`, the one at `+0x3bc`, and the movie.
Pausing one stream is `FUN_00443280` -> `FUN_004431e0`, which sends `0x800b` to
the buffer and latches the object's `+0x28`; the position is kept, and
`FUN_00443340` (`0x800c`) starts it again from there. A stream whose start frame
at `+0x34` is still ahead of the clock is skipped, because it has not begun.
SomCon stops too (`_SomStop@0`, restored with `_SomMove@4`).

The class name is the constructor's own: `FUN_00442a80` stores
`FILMOBJ::BgmSound::vftable`, and takes the start frame `+0x34` and end `+0x38`
as its first two arguments.

**Everything that suspends playback pauses the sound first.** `FUN_00424e20`
has seven call sites: host `+0xf4` (the bar's pause widget, `FUN_00424f40`),
`+0xf8` (open a menu over the script, `FUN_0042a430`), `+0xfc` (skip,
`FUN_0042a4a0`), `+0x100` (leave playback, `FUN_0042a500`), and the three
engine-side `FUN_004250b0`, `FUN_0042a380` and `FUN_0042a560`.

The speed widgets are the one suspension that does not go through the latching
wrapper: `+0x8c` (`FUN_00424f90`) calls `FUN_00424910` and `FUN_00424a10`
directly, around the rate change, so the sound is stopped and started again
within the one call and the player never hears the gap.

**Nothing on the way out of playback resumes it.** `FUN_00424eb0` has six call
sites — Ghidra's reference index and a raw scan of `.text` for `E8`
displacements agree on the same six — and all of them are paths back into
playback: `FUN_00425550` case 8 (a menu the bar opened was closed),
`FUN_00426620` cases 7 and 9, `FUN_00426bd0` case 6, `FUN_00425bf0` and
`FUN_00424f40`. `FUN_0042a500` sets state 5, which is `FUN_00426620` — and its
case 3 hands the screen to `setSystemInit` and its case 5 releases the module,
neither resuming anything. So the sound a script was making stops when the
player leaves it and does not come back; the title screen that follows has only
its own `[TitleBGM]`.

For a menu the bar opened, the same pause is a pause and not a stop: the script
is held on the frame it was interrupted on, and case 8 of `FUN_00425550` starts
it again there. The menu module's own sounds are separate objects and are not
on the rate-adjusted stream, so they are unaffected by either.

Where the host slots land is the executable's own state machine. `FUN_00427300`
switches on `engine + 0x220`, which is host `+0x1f4` — an independent
confirmation that the host interface sits at `engine + 0x2c`. State 1 plays,
state 3 opens a menu (`FUN_00425550`), state 4 moves the timeline
(`FUN_00425bf0`, where code 1 restarts the script in place and codes 2 and 5
chain to whatever `_GetNextScriptFile@12` names) and state 5 leaves. The
chaining codes are route-system territory.

`+0xf8`'s numbers are `setSystemInit`'s own codes, so 4 is the save screen, 5
the load screen and 2 the Option screen — the same three the title menu reaches.
Code 3 has a case too, selecting the module object `DAT_1004ffc8`, but **which
screen that object is has not been recovered**.

### Leaving a menu the control bar opened

The executable has **two** menu drivers, and which one is running is the whole
of where Close goes.

`FUN_0041d410`, `FUN_0041d9c0` and `FUN_0041dfa0` are the title-rooted shell.
Between them they hold **every** call to `_getNextMode@8` — 14 of them and no
others, from Ghidra's reference index on the import thunk at `0x004a1ef6` — so
the mode graph above is that shell's, and its sink is mode 2, the title.

`FUN_00425550` is the other one, and it never consults `_getNextMode@8` at all.
Host slot `+0xf8` (`FUN_0042a430`) puts the playback object into state 3 and
stores the module's code at `engine + 0x260`; `FUN_00425550` case 2 hands that
code straight to `setSystemInit` and runs the one module it names. Host slot
`+0x4c` (`FUN_0042c230`) is what writes `engine + 0x260`, and the code **0**
means leave: case 6 sees it, falls through cases 7 and 8, and case 8 sets
`engine + 0x220 = 1` — the state `FUN_00427300` dispatches to `FUN_004253f0`,
the playback tick.

So a screen the bar opened resumes playback when it is closed, and both the
screens the bar can open close the same way: the save/load screen's Close is
`+0x4c(0)` in `FUN_10014990` widget `0x14`, and the Option screen's is
`+0x4c(0)` in `FUN_10007ef0` widget 3, after that widget flushes the config
object through its own `+0x2cc` vtable slot `+8`.

`_SetReMenu@4` is the exported half of the same idea and it is *not* this rule.
It is a menu-DLL export (`FUN_10001500`) that writes the popup module's `+0xa4`
— the same member `SystemInit` records the outgoing screen in, and the same one
`_getNextMode@8(-1)` returns. `+0xf8` calls it with the code it was given and
`+0x100` (leave playback) calls it with 0, so the popup, if it is raised, knows
which screen to go back to. **What the over-playback driver does with a popup
answer has not been recovered**: `FUN_1000a8f0` answers `+0x4c(+0xa4)` on the
negative button and `+0x4c(1)` or `+0x4c(9)` on the positive one, and 1 and 9
are the popup module's own `setSystemInit` codes.

One member is worth writing down because it looks like this rule and is not.
The DLL reads `engine + 0x2d0` through host slot `+0x44` and writes it through
`+0x48`; `_getNextMode@8` case 3 returns mode 1 when it is not -1, which reads
like "resume what was running". It is not: `FUN_10011d50` stores
`page * 10 + row` into it when the player picks a filled row, and the module's
own open, `FUN_100135c0`, clears it to -1 — so it is the **save slot the player
picked**, -1 for none. `FUN_00423130` initialises it to -1 and
`FUN_004253f0` ends the playback loop when it is anything else, which is how
loading a slot from the menus stops the script that was running.

### Where a load goes, wherever it was asked for

Stopping the script is only half of it, and the other half is why a load never
lands on the title. `FUN_0041d7f0` is the mode that plays a film, and it is
built around that same member:

```text
case 0   slot = host +0x44 ;  FUN_00427850(engine, slot)   start on it
case 1   wait while FUN_004278a0 says the engine is running
case 2-3 stop it and wait for the stop
case 4   host +0x104 or +0x98 set  ->  mode 5
         host +0x44 == -1          ->  mode 2, the title
         otherwise                 ->  mode 1, itself
```

So the mode repeats itself whenever a slot is waiting, and comes back round to
its own case 0, which reads the slot and starts the film engine on it.
`FUN_0041e600` is the dispatcher that does the repeating: it stores the
returned mode in `+0x24c` and, for 1, skips the fade every other mode takes.
The title is where the **absence** of a slot goes and nothing else.

`FUN_00427850` stores the slot at `engine + 0x1c0` and puts the engine into
state 1, and `FUN_00423a70` is where that number is spent:

```text
+0x1c0 < 0       play the script named at +0x188
0 .. 99          host +0x98 clear  ->  FUN_0042b250   load the slot
                 host +0x98 set    ->  FUN_00428ab0   load it, answers followed
>= 100           FUN_00428400
```

`FUN_0042b250` and `FUN_00428ab0` are the same function twice over — format
`[SaveFileName]` with the slot number, open it, hand the stream to the store at
`engine + 0xac` — differing only in which reader they hand it to, `FUN_004336c0`
against `FUN_00434020`. Both then set the script flag `BackSel` to -1 and drop
the loading picture. So `+0x98`, the flag the play-data list raises with
`+0x94(1)`, is the whole difference between a plain load and one that follows
the slot's own answers, and both are otherwise the load the Load screen asks
for.

The Load screen asks for it with `FUN_10011d50`: host `+0x48(page * 10 + row)`
and then `+0x4c(8)` — and only for a filled row, which it knows from
`this + 0x11c + row * 4`. Code 8 is the leave code the over-playback driver
reads at `FUN_00425550` case 6, which raises `FLAG_LOGO`, goes to its case 9 —
`FUN_0042bd20`, the loading picture, drawn only when `+0x2d0` is not -1 — and
puts the engine into state 0, the state `FUN_004278a0` reports as stopped.

The slot is spent exactly once because **a film run is a thread**.
`FUN_00427850` calls `FUN_0046b070`, which is a `_beginthreadex` on
`FUN_00427780` — and `FUN_00427780` opens with `FUN_00423130`, the initialiser
that writes `engine + 0x2d0 = -1`, before `FUN_00423a70` reads the slot out of
`+0x1c0`. So case 4 of the next time round sees -1 and goes to the title, and a
loaded film does not reload itself. Mode 1's cases 2 and 3 are the other half
of that lifecycle: `FUN_004278f0` posts `0x8000` to the thread and
`FUN_00427930` waits on its handle.

### Loading restores the position, and the script name is only a file name

The slot's first record is the script name, the version, and a variable map.
`FUN_004336c0` hands that map to `FUN_00428a20`, which is `engine + 0x40` —
the same store the route DLL questions through host `+0x08`/`+0x0c`, and
`ROUTE` and `SCENE` are two of its names. **So restoring the store is
restoring the position**, and nothing re-derives it from the script name:
`FUN_0042a760` takes that name to `FUN_00430d20` and opens the file, and
touches neither name on the way.

This matters because the two could disagree, and the store is what wins. They
cannot disagree in a save this game wrote — all 1857 script names across the 55
route tables are unique, so a name resolves to exactly one scene, and every one
of a player's slots round-trips through `Progress` byte for byte with the
position taken from the store.

`FUN_0045fcd0`, which is how that map goes in, is a **merge** — it walks the
slot's entries and calls `FUN_004603f0`, find-or-insert, for each — but the
load is still a replacement, because `FUN_004336c0` empties the destination
first. Its two opening calls are `FUN_00432850`, which empties the marks and
the recorded choices at `engine + 0xac`, and host `+0x28` — `FUN_004289b0`,
which is `FUN_0045f690` on `engine + 0x40`, the store. Nothing of the
playthrough that was running reaches the loaded one.

### A film run starts from nothing

The same two halves are emptied at the start of every run, by `FUN_00423130`:
`FUN_0045f690` on the store and `FUN_00432850` on the marks. `FUN_00423a70`
then calls `_ZeroReset@4`, which walks the counter names the
`FeelingScript.ini` head declares — `FUN_10006150` is what filled that list,
from `_LoadInitScript@4`, once per process — setting each to zero, and finishes
with `ROUTE` and `SCENE` in `FUN_10006660`.

So a New Game after a finished route starts with that route's gate flags down,
and its first save carries the five counters at zero rather than not carrying
them at all. The player's own slots show it: every one the original wrote has
`000`, `003` and `004` present and zero.

`FUN_00432850` -> `FUN_00432870` skips its clear when `_GetRouteLoad@0` is set,
and the one thing that sets it is the route map: `FUN_1000e8b0`, picking a
story point, hands host `+0x48` an index of `(chapter + 1) * 100 + row` and
raises the flag. That index is the `>= 100` arm of `FUN_00423a70`'s dispatch —
`FUN_00428400`, the third loader — so jumping to a story point keeps the marks
and the recorded choices that a plain load replaces. `_GetRouteLoad@0` and
`_ResetRouteLoad@0` are `SysMenuSDHQ.dll` exports over one word,
`DAT_1004e7c0 + 0x4e8`.

### The bar is a drop-down, and translucent

The bar is on screen only while the pointer is inside its strip.
`FUN_10024100` keeps `lookup(pointer) - 1` at `this+0xd4` and branches on it
being **-2**:

    if (this+0xd4 == -2) {                       // pointer is off the strip
        if (DAT_100508c8 == 0) this+0xbc = 0;    // faded right out: bar is off
        else FUN_100255c0(this, 0, 1000);        // ramp out over 1000ms
    } else {
        if (DAT_100508c8 != 0xff) FUN_100255c0(this, 1, 300);
        this+0xbc = 1;
    }

-2 is the hit map's doing, not a sentinel the bar invents. The map object is the
executable's `ClickableMap` — vtable `0x004d70c4`, stored by its constructor
`FUN_00465830`, which the DLL obtains through host factory slot `+0xac` case 5,
and which is the **same class the choice box uses**. Its lookup `FUN_00465bc0`
returns **-1 for a point outside the map's own rectangle** and the region id — 0
for no region — for one inside it. So off the strip gives `-1 - 1 = -2` and the
bar goes away, while anywhere on the strip, over a widget or not, gives -1 or
better and it stays up.

The strip is 800x75 at the top of the screen: `ClickableMap`'s origin members
(`+0x14`, `+0x18`) are zeroed by that constructor and nothing in the bar's path
sets them, and its own extent (`+0x1c`, `+0x20`) is the map's width and height,
set by the loader `FUN_004659d0`. That loader also settles a format question in
passing — it reads the file's **bytes** from offset 8 and widens each to a `u16`
cell in memory, so one byte per pixel on disk is right and the widening is the
loader's, not the format's.

`this+0xbc` gates every resting sprite in `FUN_10024ca0`. `DAT_100508c8` is a
0..255 alpha that `FUN_10025690` applies to all of the bar's sprites at once as
an ARGB modulation, so the whole strip fades as one; `FUN_100255c0` ramps it,
and clears its start tick **only when a ramp completes**, so a pointer leaving
mid-fade-in does not restart the clock — the direction flips against the old
start and the alpha jumps. One sprite escapes the modulation: `FUN_10025690`
skips widget 0's animation while `_GetAutoDraw@0` is non-zero. **What that
export returns is not recovered.**

`MENUBAR.PNG` is **RGBA**, not opaque: the strip's panels are semi-transparent
and the frame underneath shows through them, which is why the bar is composited
as a layer and blended over the picture rather than drawn onto black.

`MENUBAR.PNG` holds two buttons and nothing else, so the bar cannot be
composited from hover states: `FUN_10024ca0` draws about fifteen sprites from
the chip sheet every frame and `FUN_10021c20` is where each is given a record.
Records 48..52 are the five menu buttons' resting art, 47/66 the rate row live
and dead, 46/65 widget 4's, 44/45 widget 1's, 42 widget 0's, 69/70 the
transparency slider's trough live and dead, 68 the `REPLAYMODE` indicator,
67 the gauge bed, and 53..64 the twelve captions
(`0x1004d318`, which is record 53). Records at chip row `y = 127` are the live
variant and those at `y = 290` the dead one — established twice over, because
each pair is picked by the same question that makes its widget pressable.

Widget 0 animates through records 29..41 while its flag is set, at
`((now - started) / (1000 / (rate_index + 1))) % 13`, so it runs faster the
faster playback is. The bar fades in over 300ms and out over 1000ms
(`FUN_100255c0`). Widget 1's hover art is inverted on purpose — it offers
`pause` while playing — and `FUN_10024100` and `FUN_100258f0` pick the same pair
independently.

See `daysengine::ui::bar`, and `days bar` to print the whole table against a
real install.

### `System/Select` — the choice box

`[SetSELECT]` is an ordinary timeline statement and **nothing about it pauses
the script**. `FUN_00431740`, the per-frame playback tick, raises the box the
first frame at or past the start, polls it while `frame + 1 < end`, and once the
window is spent decides without the player. Parsing is in `FUN_00438de0`: the
second label being the literal `NULL` or `null` is what makes a one-choice box.

The box has **no base art and no chip sheet**. `System/Select/` ships six
`.CMAP`s and nothing else, and only at 1024x576 and 1280x720 — there is no
choice map at either 800-wide size. `FILMENGINE.INI`'s `[Select1]` and
`[Select2]` name the `_Full` ones; `FUN_0040f0d0` swaps in `_Note`, and the
two-choice loader appends `_H` when `[SelectType]` and `[UseEnglish]` are both
non-zero, which is what the shipped English install has.

`FUN_0044d450` hit-tests in **normalised** coordinates — host slot `+0x144`
hands back a pair of floats, and the bounds are the doubles `0.0`, `0.5` and
`1.0` at `0x004d13d8`, `0x004d4fb0` and `0x004d13d0`. The same pair goes to the
`ClickableMap` lookup, which indexes whole pixels, so something scales them in
between; that is `FUN_00465c40`, whose decompilation loses its x87 arguments, so
**the scaling step itself is not recovered** — multiplying by the map's size is
what reproduces the shipped maps' geometry. It uses the `.CMAP` only
when `FUN_0040e830()` and `FUN_0040ea90()` both return 1, and otherwise splits
the screen itself: **on x by default, on y for the `_H` layout**. The shipped
maps agree exactly — `Select_2_Full.cmap` is two 640x720 halves and
`Select_2_Full_H.cmap` two 1280x360 ones — so the `_H` is a stacked layout, not
a horizontal one.

`FUN_0044de50` is the input, through host slot `+0x148`'s eight buttons: 0 picks
what the pointer is on, 1 cancels with 0 up, 4 and 5 walk the highlight with
wrapping, 6 confirms it and 7 cancels. It returns the choice, `-1` for declined
or `-2` for undecided, and `-2` is what the tick gates on.

**A press is an edge, not a level.** The window procedure `FUN_00466ce0` latches
a button *down* into a global — `DAT_0050c64c` for the left button, the four
after it for the others — and has no `WM_LBUTTONUP` case at all. `FUN_00467540`
hands a latch out and zeroes it in the same breath, and its one per-frame caller
is `FUN_0042b770`, which takes the whole input snapshot — cursor, two buttons,
wheel, six keys — into the members host slot `+0x148` then reads. So holding the
button down is not a press held down, every consumer in a frame sees the same
one press, and nothing has to consume it to stop it repeating.

Three things can answer instead of the player:

- with the auto flag (host `+0x134`) set, a spent window is **drawn at random**:
  `srand(GetTickCount()); rand() % (count + 1) - 1`, over a range that includes
  `-1`, so skipping can still decline;
- in a replay (host `+0x104`), `FUN_0043f3b0` overrides the pick with what was
  recorded, and falls back to a random pick among the choices
  `_GetSelectRead@4` says have been seen;
- host `+0x98` replaces the answer outright with `FUN_00428a80`.

A decided choice plays SE index 1, a declined one index 0, and the box going up
plays index 5.

**An answered box fades out; it does not vanish.** Each label lives at
`box + 0x128 + n * 0x50` and carries its own state in `entry + 0x44`, a colour
in `entry + 0x0c` and a stamp in `entry + 0x4c`. `FUN_0044dcc0` raises them all
when the box goes up. On the answer `FUN_00431740` calls `FUN_0044ddb0` on the
label that was picked — `entry + 0x4a` — and then `FUN_004316b0`, whose
`FUN_0044ddd0` flags every label spent (`entry + 0x49`) and stamps the current
frame into `entry + 0x4c`; `box + 0x1cc` is that frame, written each tick by
`FUN_004320c0`. `FUN_0044ced0` then draws, with `t` the frames since the stamp
and `D` the 24 in `DAT_0050c468` (`FUN_0044a620` sets it and nothing else
writes it — one second at 24 fps):

| state | which label | colour |
|---|---|---|
| 1 | live | `entry + 0x0c`, which `FUN_0044d3f0` sets to `0xfffe4a1f` under the pointer and `0xfff0f0f0` elsewhere |
| 2 | spent, not picked | `0xf0f0f0` with alpha `0xff - t * 0xff / D`, gone at `t = D` |
| 3 | spent, picked | `0xfffe4a1f` held to `t = D`, then `0xfe4a1f` with alpha `0xff - (t - D) * 0xff / D`, gone at `t = 2D` |

The first label to run out sets `box + 0x1c8` — the choice count — to `-1`, and
`FUN_0044d6e0` draws nothing for a count that is neither 1 nor 2. So on a
two-choice box the unpicked label ends the whole box at `t = D` and **the picked
label's own fade-out is never reached**: it holds lit and is cut. Only a
one-choice box reaches state 3's second ramp. That is the retail behaviour.

The highlight is that colour and nothing else — with no art under a label there
is nothing to light.

`FUN_0044ca10` lays the labels out. The line limit is 11 characters by default
and, with `[UseEnglish]`, 33 when `[SelectType]` is zero on a two-choice box or
66 otherwise; the per-character budget is 36 or 16 to match. **Only the English
path word-wraps**, breaking at a space when the next word would pass the limit,
so a single over-long word is never broken. Each label's anchor is the object's
scale times one of `533.4` (one choice), `266.7`/`800.0` (two) or, stacked,
`-268.0` and `-418.7`/`-118.0`; the anchor is x in the sideways layout and y in
the stacked one. `FUN_0044ca10`'s tail sets each line's **source** rectangle,
where in the shared text texture it was drawn; the **destination** is
`FUN_0044ced0`:

    x = block->0x18[n] * scale + block->0x10
    y = n * 48.0 * scale + 568.0 * scale + base
    w = scale * (533.4, or 1066.8 stacked)
    h = scale * 48.0

with `base` centring the block on its anchor — `anchor - lines * 48 * scale / 2`
in the stacked English case. `daysengine::ui::select` records that formula but
does not use it yet: it centres each label in the box the shipped `.CMAP` gives,
which is exact data and agrees with the hit testing.

See `daysengine::ui::select`, and `days select` to print the map and metrics a
resolution really gets.

---

### Dialogue layout — where a line breaks and how it is spaced

The scripts do not pre-wrap. `FUN_0043dbe0`'s `[PrintText]` arm hands the text
field straight to **`FUN_0043f600`**, which walks it a character at a time,
appending to the line it is on, and after every space looks ahead to the next
space or end of string:

    if (column + word > 0x3e) { column = 0; line += 1; }

So the limit is **62 columns, counted in characters**, and the break happens
*after* the space, which stays on the line it ends. Two details are shipped
behaviour rather than tidiness:

- the test is gated on `[UseEnglish]`, so a Japanese install never breaks a line
  this way at all;
- the look-ahead is `while ((look = look + 1, text[look] != ' ' && ...))`, which
  starts one past the word's first character, so it measures `word - 1` and a
  line can finish one column past the limit.

`｜` sets a ruby anchor, `《…》` is a ruby group handed to `FUN_0043f880`, and
`\` followed by `n` is a hard break. None of the three counts as a column.

Each line lands at `this+0x234 + line*0x1c` and the count at `this+0x2ac`.
**There is room for two lines.** The ruby array begins at `this+0x26c`, which is
two `0x1c` strides along — anchored by `FUN_0043f880` writing ruby to
`this + line*0x1c + 0x26c`, so the boundary is established from the other side
rather than assumed. Under the rule above, **49 of the 30,485 shipped English
statements wrap to three or four lines**, which is past the end of that array;
what the retail executable does with those is not established here.

`FUN_0044c740` draws one line per call, line `n` at `n * 0x30`, and advances

    local_c + FUN_0044c660(c)

per character, where `local_c` is `0x24` and drops to `0x10` under
`[UseEnglish]`. `0x30` is also the cell the font is configured with —
`FUN_00422170` calls `FUN_00436b00(font, 0x30, 0x30)`.

`FUN_0044c660` is a kerning table, not a measurement — it never looks the glyph
up. It returns 0 for everything unless `[UseEnglish]`, and then, in the order it
tests:

| characters | delta |
|---|---|
| `j` `i` `l` | -7 |
| `m` `w` | +8 |
| `M` `W` `Q` | +11 |
| `I` | -7 |
| any other `A`..`Z` | +4 |
| everything else | 0 |

The `A`..`Z` gate comes *after* `i`, `j`, `l`, `m` and `w`, so every other
lowercase letter, every digit and all punctuation get nothing. ### Dialogue placement — centred, bottom-anchored, and behind one setting

`FUN_0044bf30` is the text layer's draw, and it places each line

    x = (this->0x1dc - width * this->0x1e4) / 2.0 - 0.5
    y = this->0x1e0 - (this->0x1e8 + 40.0) * this->0x1e4 + this->0x1ec
    w = this->0x1e4 * width
    h = this->0x1e4 * 42.0

counting **down** from the last line and lifting by `39.0 * scale` each step, so
the block grows upwards from the bottom. The divisor in the x is
`_DAT_004d13c0` = **2.0**, which is what makes it a horizontal centring — each
line on its own width, not the block on the widest.

Unless `FUN_0044e2e0` answers non-zero, in which case the loop keeps the running
**minimum** of those x values and gives every line the last one, so the block
shares the widest line's left edge. That function returns `engine+0x98`, which
is the member `FILMENGINE.INI`'s `[LeftArrangement]` is read into — shipped as
`0`, so the retail build centres per line.

`FUN_0044bc90` sets the screen size and scale together:

| mode | `0x1dc` x `0x1e0` | scale `0x1e4` |
|---|---|---|
| full screen, `FUN_0040f0d0() == 0` | 1280 x 720 | 1.2 |
| full screen, otherwise | 1024 x 576 | 0.96 |
| windowed | 800 x 450 | 0.75 |

which is `0.75 x screen_width / 800` — the same 1.0/1.28/1.6 ladder the rest of
the UI scales by. `0x1ec` is `-0.5` widescreen and **`+74.5`** otherwise, which
is the 75-pixel letterbox of the 800x600 mode: in 4:3 the block sits at the
bottom of the *picture*, not of the window. `0x1e8` is `48.0` full screen and
outside `[UseEnglish]`, else 0.

The source row is `_DAT_004d2ce8` = 48 units tall and the destination 42, so a
line is squashed vertically by 42/48 before the scale.

**The whole block is behind one gate.** `FUN_0044bf30` draws it only when
`_GetDrawMessage@0` answers non-zero, and that export forwards to
`FUN_10007050(0x1004fb00)` — `+0xa4` of the Option module — whose setter
`FUN_10007070` persists the same member under the key **`TextView`**. So
subtitles are that setting, off included.

**The speaker is not drawn.** `[PrintText]` carries a speaker field and it never
reaches the text layer: `FUN_0043dbe0`'s arm hands only the text field to
`FUN_0043f600`, and `FUN_00431740`'s tail hands `FUN_0044c740` the line, its
ruby and the ruby flag. The speaker goes to `FUN_00432cc0`, which wraps it and
the text into a `0x10`-byte record and pushes it onto the list at `engine+0xac`
— the backlog. `FUN_0044bf30` draws the lines, the ruby and the choice blocks
and nothing else, so there is no name box.

`daysengine::playback::text` carries all of this.

---

## `FONTDATA.DAT` / `FONTDATA_ENG.DAT` — font

    offsets  [u32; 65536]   absolute file offset of each glyph, 0 = undefined
    glyphs   ...            RLE streams

The table is indexed **directly by Unicode BMP code point**, so it is
`65536 * 4 = 0x40000` bytes and the first glyph starts at `0x40000`. The retail
English font defines **22,420 glyphs**: ASCII, kana, CJK punctuation, all of CJK
Unified Ideographs, and fullwidth forms. No Latin-1, Greek or Cyrillic.

Each glyph paints into a fixed **48x48** cell, and there are **two planes**: a
luminance plane and an alpha plane. The alpha plane is a dilated version of the
shape, which is how the game keeps dialogue legible over moving video — a soft
halo around a bright core. Latin glyphs sit at roughly half width inside the
full-width cell, the usual arrangement for a CJK font.

The stream is a byte RLE terminated by `0x00`:

| Byte | Meaning |
|---|---|
| `0x00` | end of glyph; the rest of the cell stays transparent |
| `0x01..=0x7f` | skip that many pixels |
| `0x80..=0xff` | a run; a second byte `n` follows |

For a run with control byte `c`:

    length     = (n >> 4) + 1
    luminance  = (c << 1) & 0xff
    alpha      = (n & 0x0f) * 0x11

A glyph may end before filling its cell — the decoder clears both planes first.
So decoded pixel counts vary between glyphs even though the cell is fixed, and
the pixel count is **not** a usable way to infer the cell size from the data.
(Trying to do that is a dead end: the maximum across all 22,420 glyphs is
exactly 2304, but almost every individual glyph falls short of it.)

### How this was established

By decompiling the shipped reader rather than guessing at the bytes. The exe
carries an RTTI name `.?AVFontData@FILM@@`, and the relevant functions are:

| Address | Role |
|---|---|
| `FUN_004368c0` | the RLE decoder above — the authority for this section |
| `FUN_00436b00` | allocates both planes from a `(width, height)` pair |
| `FUN_00422170` | calls it as `(0x30, 0x30)` — this is where 48x48 comes from |
| `FUN_00436b90` | blit: look up `table[codepoint]`, decode, copy to `dst + y*pitch + x*4` |
| `FUN_004367d0` | the same blit but max-blending, used to composite the outline |
| `FUN_00436700` | composites the planes as `alpha<<24 \| lum<<16 \| lum<<8 \| lum` |

Guessing at the encoding from the data alone had produced a self-consistent but
wrong answer (skip runs of `c + 1` rather than `c`, and literal bytes rather
than `(length, value)` pairs); it decoded without overrunning and still rendered
noise. The decompiled function settled it in one pass.

**Still open:** per-character advance width. The blit takes an explicit `x`, so
the caller decides spacing; that caller has not been traced yet. The engine
currently measures the advance off each glyph's luminance plane (*not* the alpha
plane, which is dilated and would space text several pixels too wide) and adds a
fixed gap. That spaces proportionally and looks right, but is not guaranteed to
match the original pixel for pixel.

## Configuration

`Ini.GPK` holds eight files, and nothing in the install says which is which:
the master list is compiled into `SCHOOLDAYS HQ.exe` as a block of INI text of
exactly the same shape, naming every file the engine reads.

```text
[Directory]="Packs\"                   [ConfigFile]="/Config.DAT"
[FileExtend]=".GPK"                    [FILMEngine]="Ini/FILMEngine.ini"
[DXGraphicBase]="Ini/DX9Graphic.ini"   [StartScript]="Ini/StartScript.ini"
[DXSoundBase]="Ini/DX8Sound.ini"       [DebugInfo]="Ini/DebugInfo.ini"
[EndingList]="Ini/EndList.ini"         [Dummy]="Ini/Dummy.ini"
```

Those names are spelled in mixed case and the packs store them upper-cased, so
every lookup has to be case-insensitive; `Ini/EndList.ini` is `ENDLIST.INI`.

Seven of the eight are plain ASCII. `STARTSCRIPT.INI` is the odd one out, UTF-8
with a BOM, so an INI reader has to sniff rather than assume.

The ones that matter:

- `STARTSCRIPT.INI` — entry point (`00/00-00-A00`), title/system BGM, logo.
- `FILMENGINE.INI` — save paths, system SFX, font, select graphics, fade timings.
- `DX9GRAPHIC.INI` — the four resolutions and pixel formats.
- `ENDLIST.INI` — the 22 endings and their title cards.
- `FEELINGSCRIPT.INI` / `STANDERDSCRIPT.INI` — per-script affection deltas,
  keyed by script name, feeding the route logic.

### `Config.DAT` — the player's settings

Next to the executable, and a `DFLT` + zlib container: magic `DFLT` followed by
a raw zlib stream. The save files do **not** use it. Inflated it is an ordinary
engine INI with a banner line:

```text
< Config.dat >
[Format]="22"
[WindowWidth]="800"
[MasterVolume]="-1.000000"
[BgmVolume]="5"
[TextView]="-1"
```

The `Config` class in the executable writes every scalar with `%d` or `%f`
(`FUN_0046cd30` and its neighbours, reached from `FUN_0046c520`, the one
function that references the banner). The values it is handed are Windows
`VARIANT`s, so **a true bool is written as `-1`**.

The getters are the class's vtable at `0x004d73d0`:

| Slot | Getter | Reads |
|---|---|---|
| `+0xc` | `FUN_0046c9e0` | a string, through `FUN_0046e2c0` |
| `+0x10` | `FUN_0046ca70` | a bool, through `FUN_0046e080` |
| `+0x14` | `FUN_0046cae0` | an int, through `FUN_0046e140` |
| `+0x18` | `FUN_0046cb50` | a float, through `FUN_0046e200` |

All four format `[Key]="` and hand it to a helper that does three things:
**`wcsstr`** for the key (`FUN_0046e330`), so **the first occurrence wins**;
`FUN_0046dfb0` to take the characters from just past the key to the first `"`
**or `,`**, so a comma ends a value as surely as the quote does; and
`VariantChangeType` into `VT_BOOL`, `VT_I4` or `VT_R4`
(`FUN_0046de00`/`FUN_0046de90`/`FUN_0046df20`). **A bool is therefore true when
the number is non-zero**, which is how `-1` reads as true.

Two edges follow from that conversion. Each getter returns the variant's field
whether or not `VariantChangeType` succeeded, and the variant was
`VariantInit`ed, so a key that is **present but does not convert reads as zero,
not as the caller's default** — only a missing key gets the default. OLE's full
string grammar (locale words like `True`, thousands separators, a fraction
rounded into an integer) is reachable through `VariantChangeType` but nothing
the engine or the menus write uses it, and `daysengine` does not reproduce it.

The setters use `basic_string::find` (`FUN_0046d510`) rather than `wcsstr`, but
on the same literal and also from position 0: a hit is replaced through
`FUN_0046d190`, a miss is appended.

That replace is where the retail file's broken lines come from. It overwrites a
run of characters **the length of the new line**, not up to the newline, so a
value that shrinks leaves the tail of the old line behind and one that grows
eats the next line's `[`. A retail file in this install carries both spellings:
`MenVoice]="1"` at the position the old line held and `[MenVoice]="-1"`
appended at the end, because once its bracket was eaten the find stopped seeing
it. A fragment with no `[` can never be found again, so dropping malformed
lines and taking the first occurrence leaves each key exactly once and agrees
with the retail reader.

**The retail reader's limits are hard ones.** `FUN_0046c520` reads the whole
file into a 1024-byte stack buffer, checks the four magic bytes, and calls
`FUN_0046c310(buffer + 4, length - 4, out, 1024)` — `inflateInit_` against zlib
`"1.2.7"` with `windowBits` 15, one `inflate` with `Z_FINISH`, `inflateEnd` —
and the out buffer becomes a C string. Those three are zlib's own, not a
lookalike: `FUN_004a2370` is the one function in the image that references
`"incorrect header check"`, `"invalid block type"`, `"invalid stored block
lengths"` and `"unknown compression method"`, and `FUN_004a2350` is a one-line
`inflateInit2_(strm, 15, version, size)`. So the file must fit in 1024 bytes, the inflated text must
fit with room for its terminator, the stream must finish in that single pass,
and a NUL anywhere in the text ends it. `days config --roundtrip` checks a file
this engine writes against all of that.

The ten settings the Option screen loads, with the defaults it passes the
getter (`FUN_10006ce0`) and writes back (`FUN_10006e40`):

| Key | Type | Default | Set by |
|---|---|---|---|
| `VoiceVolume` | int `0..=10` | 5 | Sound tab |
| `BgmVolume` | int `0..=10` | 5 | Sound tab |
| `SeVolume` | int `0..=10` | 5 | Sound tab |
| `TextView` | bool | true | Def tab |
| `MenVoice` | bool | true | Sound tab — plays male voice lines; see the `.ORS` male-voice flag |
| `Mute` | bool | false | Sound tab |
| `Skip` | bool | false | Def tab |
| `AutoDraw` | bool | true | *(no widget recovered)* |
| `SuperSkip` | bool | false | Def tab |
| `UseSOM` | bool | false | SOMCON tab |

`MasterVolume` is a float the same write-back stores; the shipped value is
`-1.0`. A channel's volume reaches the sound layer as
`(11 - level) * MasterVolume` (`FUN_10006fd0`), which is an attenuation in
decibels — level 10 is -1 dB and level 0 is -11 dB — so louder is a *smaller*
number. Index 3 of that function is a fixed level of 2, used when muted.

`Format`, `WindowWidth`, `WindowHeight`, `DisplayType`, `TypeMiniNote`,
`WindowMode`, `UseAgate` and `Wheel` are written back untouched by the Option
screen, which asks the host about the display rather than reading them here.
The host is the executable, and it reads and writes the display ones itself.

### The display keys, read and written by the executable

`Config.DAT` is the only loose settings file an install has — there is no
separate graphics `.INI` — and the executable holds it as the object at
`0050b160`, whose getters are `FUN_0046cae0` (int) and `FUN_0046ca70` (bool)
and whose setters are `FUN_0046cea0` (int) and `FUN_0046cd30` (bool).

`FUN_0040cbb0` reads them at startup:

| Key | Default | Meaning |
| --- | --- | --- |
| `DisplayType` | the value already held | `0` selects an 800x600 back buffer, `1` an 800x450 one — 4:3 versus wide |
| `WindowMode` | `0` | the window style, below |
| `TypeMiniNote` | `0` | forces the 1024x576 art |
| `WindowWidth` / `WindowHeight` | globals at `0050b294` / `0050b2f8`, **not recovered** | overwritten outright by `DisplayType`: `0` forces 800x600 and `1` forces 800x450 |
| `WindowPosX` / `WindowPosY` | `0x7fffffff` | where a windowed run reopens |
| `Format` | `0x16` | the D3D surface format |
| `DXDeviceNo` | `0` | which adapter |

`WindowMode`'s polarity comes from `FUN_0040db00`, which is the one function
that sets the window style and which all three of its callers
(`FUN_0040dce0`, `FUN_0040ddc0`, `FUN_0040f000`) hand the live `WindowMode`
value: **`1` gives `WS_POPUP | WS_VISIBLE` at `HWND_TOPMOST` over the
monitor's own rectangle — full screen — and anything else gives the captioned
`0x90ca0000` window at the saved `WindowPosX`/`WindowPosY`.** That same
function stores its argument back under `WindowMode` before applying it.

`FUN_0040c700` writes `Format`, `WindowWidth`, `WindowHeight`, `DisplayType`
and `TypeMiniNote` back once the device has taken the mode, so the two keys a
player can change from the Option screen are stored as the change is applied
and the file is flushed when the screen closes.

`FUN_0040cbb0` can also change `DisplayType` and `TypeMiniNote` on its own: a
monitor whose aspect falls between the two constants at `004d0c28` and
`004d0c30` (the values themselves are **not recovered**) and whose height is
under `0x500` is forced to `1` for both. This engine does not do that — it
takes the panel as it finds it — which is a deliberate departure, not a gap.

See `daysengine::config`.

---

## `Save/GlobalFlag.DAT` — the global flag store

Everything persistent the player has earned: one flag per script seen, one per
replay scene unlocked, the clear flags the title screen reads, and the display
strings the save/load screen shows. It is a `std::map<wstring, VARIANT>`
written out whole — no index, no compression:

```text
"FlgH"                4 bytes. Written, but the reader only checks that four
                      bytes came back; it never compares them.
varint  count         number of entries
count x
    wstring name      enciphered, see below
    varint  flags     -1 in every entry of every file seen
    varint  vt        VARIANT type tag
    value             by vt
```

**varint** — seven bits per byte, most significant group first. The top bit of
every byte but the last is set, and `0x40` *of the first byte only* marks a
negative number, encoded as `-1 - magnitude`:

| Bytes | Value |
|---|---|
| `05` | 5 |
| `40` | -1 |
| `90 7e` | 2174 |

`VARIANT_TRUE` is `-1`, so a set flag is the single byte `0x40` — which is why
a hex dump of the file is long runs of `40 0b 40`.

**Strings** — a varint length in *characters*, then that many UTF-16LE code
units, each XORed with its own index:

```text
unit[i] ^= i
```

The index is the character position and never wraps. This is obfuscation, not
encryption: there is no key. It is why the names look like readable text with
holes punched in it — `"01-34(67%H:;"` is `"00/00-00-A00"`.

**Value types** are Windows `VARIANT` tags. Anything not in this table is
written as `VT_I4` zero, so nothing else can appear:

| vt | Type | Encoding |
|---|---|---|
| 3 | `VT_I4` | varint |
| 4 | `VT_R4` | 4 raw little-endian bytes |
| 8 | `VT_BSTR` | string, as above |
| 11 | `VT_BOOL` | varint; `-1` true, `0` false |

Recovered from `SCHOOLDAYS HQ.exe`: `FUN_0045fe90` / `FUN_004600d0` write and
read the container, `FUN_0042b490` / `FUN_0042b5f0` locate it through the
`[FlagFileName]=` INI key, `FUN_004350b0` is the varint codec, `FUN_00435010`
is the string reader that applies the cipher, and `FUN_0045c890` switches on
the value tag.

### What the title screen reads

The DLL decides nothing itself: it asks the host three questions through a
vtable and picks its art from the replies. Only one of the three turns out to
be save data.

The host interface the DLL is handed is a **secondary base subobject** — the
vtable `0x004d2894` is installed at `[object+0x2c]`, so every member offset the
executable uses is `0x2c` above the offset the DLL sees. Missing that is what
makes a scan for writes come back empty and look like a discovery.

| Question | Slot | Getter | Object member | Answer |
|---|---|---|---|---|
| all-clear | `+0xe8` | `FUN_0042c2f0` | `+0x7a4` | **always false** |
| trial | `+0x34` | `FUN_0042c120` | `+0x7a8` | **always false** |
| route *n* cleared | `+0xec` | `FUN_0042baf0` | `+0x21c` + flag | see below |

**All-clear and trial are not flags.** Exactly two functions write either, and
both are constructors:

```text
FUN_004217e0    +0x7a0 = 0   +0x7a4 = 0                    +0x7a8 = 0
FUN_00421b30    +0x7a0 = 0   +0x7a4 = <query "CrossDays">  +0x7a8 = 1
```

`FUN_004217e0` is the one the executable runs, from `FUN_0041e3a0`.
`FUN_00421b30` — the one that would answer all-clear from a `L"CrossDays"`
query and declare itself a trial — **has no callers at all**, confirmed by
Ghidra's reference index and by a raw scan of `.text` for its address. It is
the sibling-title build, dead code here.

So in the retail executable all-clear is always false, and **`Title_AC` is a
screen the game cannot reach** — along with the sixth widget painted into it,
the audio-commentary entry, whose action is `+0xe0(1)` where `START` is
`+0xe0(0)`. Trial is always false too. This is confirmed against the real game:
a save with all 22 endings seen still shows `Title_Clear`.

`Title_AC` is nonetheless still implemented, because this is a reimplementation
of the DLL's test and not of its reachable subset.

The `AllClear` **save flag** is real and is read — but by `FUN_0041fee0`, to
choose the backdrop, not the title art. Two different questions with almost the
same name; see the next section.

Route 0 and route 1 both come from the one `EndClear` flag, so clearing the
game once changes the title art and unlocks `REPLAY`. Route 0 carries one
further condition, `+0x21c`, and it is now recovered: `FUN_0041f600` sets it
from `STARTSCRIPT.INI [EndBGView]`, through the setter `FUN_00420150`.

```c
uVar5 = *(undefined4 *)(param_1 + 0x26c);     /* [EndBGView] */
pvVar3 = FUN_00440730(&DAT_0050c428);
FUN_00420150(pvVar3, uVar5);                  /* host +0x21c */
if (*(int *)(param_1 + 0x26c) != 0) {
  FUN_0041fc40(param_1);                      /* load ENDLIST.INI */
}
```

One key therefore does two things: it is the extra condition on route 0, and it
gates loading the ending list at all. Clearing it leaves the plain `Title` and
no ending backdrops however far the player has got. The shipped value is `"1"`.

### Which endings have been seen, and the title backdrop

The picture *behind* the title is not fixed. `Title.png` is transparent around
the logo, and what goes underneath is the title card of the most recent ending
the player reached — or a card of its own once every ending is seen.
`FUN_0041fee0` is the chooser and picks, in order:

1. `AllClear` set — `ENDLIST.INI [AllClear]`, `END-ALL-complete.png`.
2. `EndClear` clear — `STARTSCRIPT.INI [BaseFile]`, `TitleBase.png`, the
   fresh-install picture.
3. every ending seen — `[AllClear]` again, *and* the flag is set on the way
   past, which is how `AllClear` comes to be stored in the first place.
4. otherwise — ending number `EndNo`'s own card.

`FUN_0041fc40` loads the list from the file `[EndingList]=` names — a key that
is in no shipped INI, only in the master block inside the executable (see
[Configuration](#configuration)), where it reads `Ini/EndList.ini`:

```text
[EndingMax]="22"
[Ending01]="System/EndTitle/END-05-5H-E00.png"
...
[Ending22]="System/EndTitle/END-SETSUNA.png"
[AllClear]="System/EndTitle/END-ALL-complete.png"
```

The keys are 1-based but are pushed into a vector in order, so **slot `n` holds
`[Ending(n+1)]`** and is addressed 0-based (`FUN_00420310`, stride `0x1c`).

The per-ending flags are named for the INI key syntax, punctuation and all —
`[End00]="` through `[End21]="`, 0-based, one per ending. `FUN_00420040` counts
how many are set and compares against `[EndingMax]`; `days save --grep "[End"`
lists them. So `EndNo` is an **index**, not a tally: `EndNo = 20` means the most
recent ending was slot 20, i.e. `[Ending21]`.

`STARTSCRIPT.INI [EndBGView]` gates the whole feature: `FUN_0041f600` only
calls the loader above when that key is set. It is the same key that carries
the extra condition on route 0 — see the previous section.

Two of the 22 cards are `.wmv`, not `.png` (`Ending16` and `Ending21`), so the
title backdrop can be a movie. This engine draws their **first frame**. Whether
the original animates them is **not recovered**; `[EndBGView]` is not that
switch — it is read once, as a plain on/off, and `[MovieView]` next to it
selects `.png` or `.wmv` for the *logo*, not for the ending cards.

Branch 3 is the one that writes, and this engine does not write save data yet,
so it recomputes the answer on every launch instead. The picture is the same
either way; the player's file is left alone.

`days save` reports the whole chain, and `days menu` reports the backdrop
alongside the widget table, which is how the two halves of "which title" get
checked together.

**Trial is not recovered.** The retail executable holds no trial string and no
reachable trial branch, so there is nothing to read; a trial build would be a
different executable, not a different save. The engine answers `false`.

## `Save/SaveFileNNN.DAT` — a save slot

Not a snapshot of the engine: a **log**, which is what its magic says. It
records where the player is, every story point they have reached with the state
they reached it in, and the choice they made at every script. Loading replays
that into the engine rather than restoring a memory image.

```text
"SLog"                      4 bytes, compared on read
records, until a 0 tag:
    varint tag
    tag 1   wstring script     where the player is
            f32     version    checked against _GetVersionToRoute@4
            FlgH    store      the save's store at that moment
    tag 3   wstring script     a story point reached
            wstring story      the SP*** flag its marker set
            varint  order      how many story points preceded it
            FlgH    store      the store as it was there
    tag 4   wstring script     a script the player answered a choice at
            varint  choice     the index they chose, -1 for none
    tag 0   end of the file
```

Every other tag is refused, in the game's own words: "Undefined backlog entry."
There is exactly one tag-1 record and it comes first; the tag-3 records follow
in `std::map` order **by their story flag** and the tag-4 records in map order
by script. The strings and varints are the same primitives `GlobalFlag.DAT`
uses, cipher and all.

Where each record comes from:

- **tag 1** is the position. The loader hands the script and the version to
  `FUN_0042a760`, which is what puts the player back — so a slot whose store
  disagrees with its script follows the script.
- **tag 3** is written by the story marker, host slot `+0x00` (`FUN_00428480`)
  — the same call that sets `SP%03d` in both stores. `order` is the map's size
  when the point was **first** recorded, so it is the order the player reached
  them in and not the order they are stored in. Jumping back to a story point
  erases every entry from it onward (`FUN_004331a0`), which is why `order` has
  gaps in a save that has been rewound; one of the player's own slots does.
- **tag 4** is the recorded choice. `FUN_00431740` stores it as each box
  settles (`FUN_00428a50`) and reads it back instead of asking the player while
  the engine is replaying (`FUN_00428a80`).

**The version is checked.** `_GetVersionToRoute@4` reports a float and a slot
whose tag-1 version differs is refused with a message box — "Script version
does not match." / the Japanese equivalent, chosen by host `+0x5c`. Every slot
in a retail install carries `1.0`; what the export computes is **not
recovered**, so DaysEngine writes back whatever a slot was read with, and `1.0`
for a slot written from nothing.

### One store, not two

A slot carries one `FlgH` map per record and that is the whole of its state.
Host slots `+0x08`/`+0x0c` (integers) and `+0x10`/`+0x14` (booleans) all reach
the same member, `host + 0x14`; only `+0x18`/`+0x1c` are a different store, the
global one. So the feeling counters, the numbered gate flags, `SP***` and the
`BS****` back-bookmarks share one map, and a name holds `VT_I4` or `VT_BOOL`
depending on which setter last wrote it — a player's own save has `001` as
`VT_I4` beside `946` as `VT_BOOL`.

### The line the save screen shows

A slot file says nothing about itself. The display line lives in the **global**
store, under the key `[SaveConfig]="FILMEngine/SaveFile00%d"` formats, with the
player's comment under the same name plus `_Sub`. `FUN_0042aea0` writes both
when it writes the slot; `FUN_0042a980` reads them back and reports a slot as
present only when **the file opens**.

`FUN_10011b40` builds the line from the clock and the chapter number, stored as
one string:

```text
Japanese   "%4d年%2d月%2d日(%s)%02d:%02d"  +  "第%d話"
English    "%2d/%2d/%4d(%s)%02d:%02d"      +  "%02d"
```

The reader splits the chapter back off by character count — three for Japanese,
two for English — which is exactly the length each tail has. The chapter is
`_GetStory@4`, a 55-way switch on `ROUTE` returning 1..6; `days-route` decodes
it the same way it decodes the branch graph.

### How far this was checked

Every one of the player's 22 save files — the 63KB global store with its 2,174
flags and all 21 slots — **reads and writes back byte for byte identical**, and
each slot also survives a pass through the engine's own model of it unchanged.
That is the standard the writer is held to: a save DaysEngine writes is a save
the original game reads. `days save --roundtrip` is that check.

---

## The save/load screen — mode 3, `System/SaveLoad`

One module does both jobs, chosen by its `+0x94`. `setSystemInit` — a **second
dispatch**, with its own numbering, separate from `SystemInit`'s mode integers
— pokes it: code 4 opens the module to save, code 5 to load. Those are two of
the numbers the control bar's own menu buttons produce, so the bar's four menu
widgets are Save, Load, something `SystemInit` has no case for, and Option.

Ten slots to a page and ten page buttons, so a hundred slots. The widget table
has two bands of ten for the rows, the left of a row and the right of it, and
clicking either picks the same slot:

```text
0x00 .. 0x09   the ten rows
0x0a .. 0x13   the ten page buttons
0x14           leave
0x15           the route map, Load screen only
0x16 .. 0x1f   the ten rows again, the other band
```

`FUN_10014990` is that dispatch. The slot a row stands for is `page * 10 + row`,
which is why the shipped `[SaveFileName]="Save/SaveFile00%d.DAT"` puts slot 14
in `SaveFile0014.DAT`. Every widget is live unless the confirm popup is up
(`FUN_10014910`): an empty slot is not greyed out, picking it simply does
nothing.

**This screen's chip table does not match its hit map the way every other
screen's does.** Ten of its thirty-two regions are half of a row the sprite
covers whole, and ten more are about three rows tall, so only twelve regions
can ever reproduce a record. The twelve that do are consecutive, which is
enough to anchor the table and fill the rest in at its stride —
`days_ui::atlas` now believes a long exact run whatever proportion of the
screen it covers.

Neither mismatch is an error in the table, and `FUN_10011600` says why. Its
hover loop runs over the ten rows and lights `+0xa8 + row * 4` when
`row == selection || row + 0x16 == selection`, so **the two bands share one
sprite**: the first band's, which spans the whole row. The second band has no
hover art, and pointing at either half highlights the row entire. Drawing each
band's own record instead lights three rows at once, which is what it looks
like when you get this wrong.

The tall records are the panel of a tooltip. With the selection in the second
band, `FUN_10011600` calls `FUN_10012900`, which re-wraps the slot's whole
comment over up to three lines and shows it over the list: the panel sprite at
`+0x108`, cut from that record, and the lines at `+0x1bc + n * 4` rasterised
into the same 2048x1024 surface at `(0x400, 0x202 + n * 0x40)`. The panel's
height grows with the line count, and rows 8 and 9 borrow rows 6 and 7's record
so the panel opens upwards there and three lines cannot run off the bottom.

The wrapping is by character count and never by pixels: Japanese breaks every
twenty characters with no regard for what it cuts, English looks ahead at each
space to the end of the next word and breaks if that word would not finish
inside forty, and both stop after three lines' worth of input. A break at
exactly the cap leaves an empty line behind and the shipped loop counts it,
which matters because that count is what sizes the panel — though the panel
actually switches on the **character count** divided by the per-line cap, not
on the wrapped line count, so for English the two can disagree. The shipped
formula is kept.

The panel's sprite takes the record's full height as its source however short
it is drawn, so a one-line panel is a 472x97 cut squashed into about 31 pixels.
`_DAT_1003b0f8` is a `fdivl`, the double 3.0: the record is three rows and the
panel is that divided by the rows it needs.

One shipped bug here. The panel's y shift and the text's are written only on
the branches a row past the eighth takes, and zeroed only on the three-line
branch, so a shallow row with one or two lines reads **two uninitialised
floats**. DaysEngine uses zero, which is what the branch that does initialise
them uses and what puts the panel on its own row.

One more departure from `FUN_10014910`: the route map's sprite is drawn behind
one condition more than the enablement carries, `+0x94 == 0`. On the Save
screen the widget is still pointable and still does nothing, and lights
nothing.

### Naming a save — a Win32 dialog, not game art

The save screen does not draw the box that asks for a comment. It hands the
current text to host `+0xdc`, which stores it and posts `WM_USER` to the game
window; the window procedure's `0x400` case opens a **modal dialog from the
executable's own resources**:

```text
host +0xdc  ->  SendMessageA(hwnd, WM_USER, 0, 0)
wnd proc    ->  DialogBoxParamA(hinst, 0x73 | 0x77, hwnd, FUN_0042e4a0, 0)
                  0x73 Japanese, 0x77 English -- chosen by host +0x5c
```

Both are `DIALOGEX` templates, 280 x 62 dialog units, `MS Shell Dlg` 8pt, with
four controls: a prompt (`0x410`), an edit field (`0x40c`), `OK` (`1`) and
`Cancel` (`2`). The English one is captioned `Enter comment` and prompts
`Insert comments (120 characters or less)`; the Japanese one is
`コメント入力` and `コメント入力（６０文字以内）`.

`FUN_0042e4a0` is the procedure, and it is short:

- **`WM_INITDIALOG`** centres the dialog on the game window, or on the screen
  when the game is full screen.
- **OK** reads at most `0x79` bytes from the edit field, widens them and calls
  `_CommentSet@4`, which stores the text on the save screen's module and sets
  its `+0x98`.
- **Cancel** ends the dialog and calls nothing.

That `+0x98` is the same member the confirm popup sets, and the save screen's
next tick is what writes the slot: `if (kind != Load && +0x98) { save the
chosen row; +0x98 = 0 }`. **So the dialog is the confirmation, not a decoration
on a save already decided — cancelling means no save happens at all.**

The two prompts disagree about the limit only because the buffer is 120
**bytes** and `GetWindowTextA` is the ANSI call: 120 ASCII characters, or 60
Shift-JIS ones.

DaysEngine cannot open a Win32 dialog, so it draws one — but the caption, the
prompt, the button captions and every rectangle are read out of the player's
own executable by `install::dialog`, and the dialog units are converted by
Windows' own rule, `x * base_x / 4` and `y * base_y / 8`, against the base
units of the font being drawn with. Only the colours are ours: the original
took the player's Windows theme, so there is nothing there to recover. Typing
goes through SDL's text input, which is what carries an IME.

### Where a row's text sits

No column is drawn to the screen directly. `FUN_100135c0` builds a 2048x1024
off-screen ARGB surface — a `FrameBuffer` at `+0x10c`, wrapped by a
`DX9Texture` at `+0x110`, with the buffer's pixels cached at `+0x114` and its
pitch at `+0x118` — and `FUN_10011ec0` clears it and rasterises all thirty
columns into it at once. Each column then gets its own `DX9Sprite2D`, at
`+0x144 + row * 4`, `+0x16c + row * 4` and `+0x194 + row * 4`, with a source
rectangle cutting the surface and a destination rectangle placing it.

Those three class names are RTTI, not inference: host `+0xac` allocates by type
code, and codes 0, 3 and 4 run the constructors that install
`DX9Texture::vftable`, `DX9Sprite2D::vftable` and `FrameBuffer::vftable`. It
matters, because Ghidra renders the source-rectangle set-up as a chain of
`float10` results and it is nothing of the kind: `DX9Texture` slot `+8` is
`x / width` and slot `+0xc` is `y / height` — pixels to texture coordinates —
and `DX9Sprite2D` slot `+0x1c` takes an origin **and a size**, forwarding
`(u, v, u + du, v + dv)` to slot `+0x18`. Read as decompiled, the four
coordinates come out in the wrong order. The disassembly gives the real one,
and it is self-checking: the two `/ width` values pair with each other and the
two `/ height` values with each other.

On the surface, per row `r`:

```text
timestamp   x 0      y r * 48 + 2     548 x 48
chapter     x 1024   y r * 48 + 2     548 x 48
comment     x 0      y r * 48 + 514   986 x 48
```

`FUN_10011ec0` rasterises into exactly those origins, which is the independent
check on the cut. On screen, in the 800x450 layout space the widget records
use, where the record is the row's own — the first band of ten for the stored
line and the second for the comment, so the two bands are not duplicates:

```text
timestamp   record[r].x        + 1.0    record[r].y + 4.5   252 x 24
chapter     record[r].x        + 262.5  record[r].y + 4.5   252 x 24
comment     record[r + 0x16].x + 2.0    record[.].y + 4.5   494 x 24
```

Every one of those constants was read out of the DLL with its operand width
taken from the instruction — `flds` for a 4-byte float, `fmull` / `faddl` for
an 8-byte double — rather than from Ghidra's `(float)_DAT_...`, which narrows a
double at the use site. Half of them are doubles sitting next to a zero word,
so read as floats they come back `0.0`, which is the failure that once cost
this project a zero-width gauge.

Only the height is an exact halving, 48 to 24. The widths are not: 986 to 494
and 548 to 252, so both columns are squeezed horizontally and the stored line's
noticeably. Those are the shipped constants.

English, off host `+0x5c`, moves the timestamp 5.0 right and the chapter 15.0,
and centres the comment by `235.5 - width / 4` clamped at zero, where `width`
is the advance total the rasterising loop accumulated. Japanese moves nothing.

The glyphs go in through host `+0x58` — `FUN_00436c10`, which writes at
`dst + y * pitch + x * 4` — and each pixel is
`alpha << 24 | lum << 16 | lum << 8 | lum`, kept only where it exceeds what is
already there (`FUN_004367d0`). So the surface carries the font's luminance
plane as colour and its outline plane as alpha, which is what the engine's own
line rasteriser produces.

**The comment column's gate is `FILMENGINE.INI [TextInput]`.** The comment
sprites are built, and the comment rasterised, only when host `+0xd8` answers
non-zero. That slot is `FUN_0042bad0`, returning member `+0x74` of the
interface it is called on. The host object carries **two** interfaces, and the
constructor installs both: `movl $0x4d2894,0x2c(%edx)` and
`movl $0x4d2864,0x30(%eax)`. The menu DLL is handed the first, so the member is
object `+0xa0`.

Searching for writes to `0xa0(reg)` finds none that belong to this class, and
that is not the same as the member never being written — the writer holds the
**other** interface, so it stores at `0x70(reg)`. It is slot `+0x20` of the
`+0x30` vtable, `FUN_00422170`, which is the `FILMENGINE.INI` reader:

```text
[TextInput]   -> this+0x70  ->  object +0xa0  ->  host +0xd8
[UseEnglish]  -> this+0x74  ->  object +0xa4  ->  host +0x5c
```

The line below is the check on the line above. Host `+0x5c` is the English
question, recovered long before from the other side, and it reads the member
that the key sitting next to `[TextInput]` writes. Two interfaces, two
different deltas, one member, and the meaning agrees.

This is the trap the two-vtable rule exists for, and scanning one offset is not
enough to retire a member: a scan at `+0x74` finds dozens of unrelated writes
and a scan at `+0xa0` finds none, and the answer was at `+0x70` on a third
pointer. The shipped INI sets `[TextInput]="1"`, and the column shows.

Two more sprites, `+0xf8` and `+0xfc`, are placed from records
`(page + 0x20) * 0x18` and `(page + 0x2a) * 0x18` — the current page's
indicator. Those indices run past the thirty-two the atlas recovers for this
screen, and they are **not implemented**. The same surface also carries the
expanded comment's three lines, at `(0x400, 0x202 + n * 0x40)`, for the tooltip
described above.

---

## Media

- **Video**: ASF/WMV, codec WMV3 (VC-1 Main), 800x452, 24 fps, ~800 kb/s,
  **no audio stream**. ffmpeg decodes these natively.
- **Audio**: Ogg Vorbis, separate from the video. `DX8SOUND.INI` declares the
  mixer format as 44.1 kHz, 16-bit, stereo.
- **Background music is split into intro and loop halves.** A script asks for
  `BGM/SD_BGM/sdbgm07`; the pack holds `SDBGM07_INT.OGG` and `SDBGM07_LOOP.OGG`.
  The intro plays once, then the loop repeats until the track is replaced. Nine
  tracks ship loop-only (`sdbgm14`, `18`, `20`, `28`, `29`, `31`, `32`, `33`).
  `BGM/Vocal/SDV*` are plain one-shot files, played through `[PlaySe]`.
- **Stills**: PNG, 8-bit, colour type 6 (RGBA).

Because video carries no audio, playback is a video decoder plus an independent
OGG mixer, both slaved to one 24 fps script clock. That is considerably easier
to keep in sync than a muxed stream.

---

## `RouteProcSDHQ.dll` — the branch graph

A third shipped binary beside the executable and the menu DLL, and it owns
every branching decision in the game. The executable decides nothing: its
timeline-move state (`FUN_00425bf0` case 7) calls
`_GetNextScriptFile@12(engine + 0x2c, buf, 0x104)` and plays whatever name
comes back.

### Progress is two integers

`ROUTE` and `SCENE` are named entries in the **save's** variable store — not
`GlobalFlag.DAT`. There are two stores, reached through different members of
the host interface:

```text
host +0x08 / +0x0c   get / set an int    the per-save store   [host + 0x14]
host +0x10 / +0x14   get / set a bool    the per-save store
host +0x18 / +0x1c   get / set a bool    the global store     [host + 0x10]
```

Both are the same `std::map<wstring, VARIANT>` the flag store already
documents, and `FUN_00460810` is the lookup that creates a missing name on
demand — which is why an unset counter reads as zero. `SaveFile000.DAT` holds
`ROUTE = 39`, `SCENE = 6` in its embedded store.

`_SetRoot@12` is just `set("ROUTE", a); set("SCENE", b)`.

### 55 routes, each a table and a state machine

Four exports switch 55 ways on `ROUTE`, one case per route, `0 .. 0x36`:

```text
GetNextScriptFile   FUN_10006870   (SCENE, choice) -> next SCENE, and the name
SetFeeling          FUN_1000bba0   credits the chosen scene's feeling deltas
searchRoot          FUN_10007d50   which (ROUTE, SCENE) a script name sits at
SetScript           FUN_10008a70   plain table[ROUTE][SCENE] lookup
```

`GetNextScriptFile` loops up to ten times, re-reading `ROUTE` each pass, but in
the retail build it can never dispatch a second route: every one of the 48
`return 0` sites across the 55 handlers is immediately preceded by the same
call, which sets `ROUTE` to **-1** and copies an empty name. -1 is outside the
export's `0..=0x36` switch, so the next pass takes its `default` and returns 0.
The loop runs at most twice.

**Route transitions do not happen by falling through.** They happen because an
arm calls *another route's* emitter: route 0's last scene calls route 1's,
which sets `ROUTE` to 1 and copies out of route 1's table. The emitter carries
the destination route as a literal, which is what makes a transition visible
from the arm that takes it.

`searchRoot` takes a route to start from and walks forward, **wrapping at
`0x36` back to 0**. A name that is in no table therefore spins forever — the
`default` arm that would bail out is unreachable past the wrap. `days-route`
returns "not found" after one pass rather than reproducing that.

### The state machines, and how they are decoded

Each route's case in `GetNextScriptFile` is a `switch (SCENE)` whose arms call
a per-route emitter with a literal next-scene number. The emitter
(`FUN_1000d3d0` for route 0) sets `ROUTE` and `SCENE` and copies
`table[scene]` into the caller's buffer. Branch arms read host `+0x04` — the
choice the player just made — and a few read the feeling counters. Some arms
also record a bookmark, `set("BS<script>", SCENE)`, which is what
`GetBackScriptFile` rewinds through.

There is no table of edges anywhere: they exist only as compiled x86.
`crates/days-route` recovers them by **decoding those 55 functions out of the
user's own DLL**, with no address written down — the handler addresses come
from `_GetNextScriptFile@12`'s own dispatch switch, which is found through the
PE export table.

The decoder does not try to recognise the two shapes of `switch` the compiler
emitted (a jump table for the big routes, an `if`/`else if` chain for the small
ones). It **seeds `SCENE` with the value being asked about** and executes the
handler symbolically: the chain's comparisons then fold to constants and the
jump table's index resolves, and what survives is exactly the branching that
depends on something only known at run time. Anything outside the covered
instruction subset ends the walk as "not recovered" rather than being guessed
past.

The helpers an arm calls are classified the same way, by decoding them:

```text
sets ROUTE to a literal, and indexes a name table   an emitter
sets ROUTE to -1, no table                          the route is over
sets SCENE, writes a literal script name            a fixed script
formats "SP%d"                                      the story-number marker
formats "[End%02d]=\"" and touches EndClear          an ending
formats "[%s]=\"" and reads a counter back           a StanderdScript gate
formats "[%s]=\"" and makes no host call             a FeelingScript credit
clears a run of numbered flags                      the route's flag reset
```

The last two share a format string and are told apart by what they do with the
entry, not by their addresses.

#### What the arms branch on

Across all 55 routes, every surviving comparison is one of these:

| what is read | host slot | sites |
|---|---|---|
| the choice just made | `+0x04` | 485 |
| a numbered flag in the save's flag store (`801`-`804`, `926`-`999`) | `+0x10` | 83 |
| two feeling counters against each other (`001` vs `002`) | `+0x08` | 25 |
| a `StanderdScript.ini` gate | — | 13 |
| a `BS****` back-bookmark against a literal | `+0x08` | 2 |
| a word in the DLL's own `.data` | — | 46 |
| host `+0x34` | `+0x34` | 1 |

The 25 counter comparisons are at exactly the 25 routes the affection work
found independently, and the 13 gate sites resolve to exactly the 13 entries
`StanderdScript.ini` declares.

Two of those are not save data at all:

- One `.data` word gates the per-route flag reset at 44 sites. `DllMain`
  zeroes it on attach and sets it only after loading both feeling INIs from the
  absolute path `Z:\SCHOOLDAYSHQ\Ini\` — a developer machine's drive. On any
  install it stays 0, so **a route never clears its numbered flags** and they
  accumulate for the life of a save.
- The other is read once, by route 0. Nothing writes it: a scan of the whole
  file finds a single reference to its address, which is the `cmp` that reads
  it.

**Host slot `+0x34` always returns 0 in the retail build.** It returns a member
of the engine object; the constructor stores 0 there, and the only code that
stores 1 is `FUN_00421b30`, which nothing references — no call, no jump, no
address taken, confirmed by Ghidra's reference index and by a byte scan of the
whole image. So route 0's last scene always hands over to route 1 scene 0, and
the branch it would otherwise take — a `Notice_SDHQ` screen, or a rotation
through `PV/SEKAI-OP`, `PV/KOTONOHA-OP` and `PV/SETUNA-OP` — is unreachable.
That rotation is the one arm of the 1,840 that is **not recovered**: it selects
by `x & 0x80000001`, a signed modulo the decoder does not model. It is behind
the always-false test, so nothing reaches it.

#### How far this was checked

- Against a Ghidra decompilation of all 55 handlers, the recovered trees
  reproduce every one of their **1,840 `case` arms** exactly — same emitter,
  same literal next scene, on every path.
- All **1,857 scenes** have a transition, **no edge** names a scene no table
  has, and **every** scene is reachable from `ROUTE 0 SCENE 0`.
- The story numbers the markers assign account for every `SP***` flag in a real
  save.
- `_SetFeeling@8` works its destination out for itself rather than being told
  it. Decoded independently, the two exports agree on **1,426** of the **1,435**
  `(scene, choice)` pairs that credit a script. The nine that differ are three
  scenes where the shipped DLL has two arms swapped — see the affection section.

### The name tables, and how they are found

Each route has an array of pointers to wide script paths in `.rdata`, indexed
by `SCENE`. `crates/days-route` recovers these from the user's own DLL by
**content** — runs of consecutive pointers that resolve to a string shaped like
`NN/NN-XX-Ynn` — with no address embedded anywhere. That was checked against
the code three ways before being believed:

- the 55 `searchRoot` handlers reference exactly one `.data` address each, and
  those 55 addresses are **exactly** the 55 run starts the content scan finds;
- `_SetScript@16`'s own 55-way switch indexes the same 55 addresses;
- route 0's handler bounds its loop at `< 0x15`, and that run is 21 long.

The result is 55 routes and 1,857 names, against 1,857 `.ORS` files in
`Script.GPK`, agreeing on 1,855. The four that differ are shipped facts:
`03/03-B2-A00` and `03/03-KB-E00` are named by a table but do not ship (both
sequences begin at `A01` and `E01`), and `01/01-00-OP2` and `05/05-9O-B00`
ship but are named by no table.

Immediately after each name array sits a second array of the **same names as
narrow strings**, without the directory prefix. Nothing here reads it; it is
noted so a future scan does not mistake it for a table.

### Story numbers: the `SP%03d` flags

Beside emitting a name, each route maps some scenes to a global story number
(`FUN_1000d320` for route 0: scenes 0, 3, 0xe, 0x14 to 100, 101, 102, 103).
Host slot `+0x00` turns that into `SP%03d` and sets it true in **both** stores.
Route 0's numbers are exactly the `SP100`..`SP103` a real save carries. These
are the route-map markers `GetRouteMapPage` reads.

The DLL's own clear path formats `SP%d` without the padding, which would
disagree for a number below 100; every story number in the retail build is 100
or more, so it never does.

### Endings

`FUN_10006590` writes an ending: set `[End%02d]="` and `EndClear` in the global
store if unset, `EndNo` to the ending number, and `EndClear` in the save store.
`docs/FORMATS.md`'s title-screen section already covers how those are read.

### Affection: `FEELINGSCRIPT.INI` and `STANDERDSCRIPT.INI`

Five named counters live in the save store alongside `ROUTE` and `SCENE`. Both
tables declare the same five in their head:

```text
[Number]="5"
[flag0]="002"  [flag1]="000"  [flag2]="001"  [flag3]="003"  [flag4]="004"
```

Neither file is parsed. Given a script path the reader drops the first three
characters (`00/00-00-A04` becomes `00-00-A04`), builds the literal
`[00-00-A04]="` and **searches the whole file text for it**, then reads fields
split on `, ` and ending at a `,` or a `"`:

```text
FEELINGSCRIPT.INI   [00-00-A04]="002, 5, 000, 0"    two (name, amount) pairs
STANDERDSCRIPT.INI  [01-00-N05]="002, 11"           one (name, amount) pair
```

`_SetFeeling@8(host, 1)` is called on every decided choice, from the playback
tick `FUN_00431740` — at the moment the box settles, right after the index is
stored and long before the script has ended. It is **its own 55-way switch**,
not the branch graph's: each route's arm works out from `SCENE` and the choice
which scene the player is about to move to, takes that scene's name from the
same table, and credits its deltas. `FUN_10005c60` adds, `FUN_10005ce0`
subtracts for moving backwards, and both skip a zero amount outright.

Most scenes credit nothing — only the forks do — so the branch graph's
destination cannot stand in for it. Decoded, the two exports agree on 1,426 of
the 1,435 `(scene, choice)` pairs that credit anything.

The other nine are a **shipped bug**. At three scenes — route 4 scene `0x16`,
route 5 scenes `0x15` and `0x18` — two of `SetFeeling`'s arms are the other way
round from the branch graph's:

```text
route 4 scene 0x16   moves to     choice 0 -> 0x1c, else -> 0x15
                     credits      choice 0 -> 0x15, else -> 0x1c
```

So the player goes one way and is credited for the other. `DaysEngine`
reproduces it: `SetFeeling` is what credits, and this is what it credits.

What each counter is worth is very uneven:

| Name | Drawn | Read by |
|---|---|---|
| `001` (Sekai), `002` (Kotonoha) | both gauge bars | the relative test below, at 25 routes; thresholds at 10 scripts |
| `004` | no | thresholds at 3 scripts |
| `000` | no | nothing — it is the filler |
| `003` | no | nothing |

`000` is named in 1,144 of the 1,438 delta slots with an amount of zero,
because the reader always consumes two pairs and an entry wanting one pads the
other. Across all 719 entries there are 283 non-zero amounts over 283 entries,
so **no shipped script moves two counters at once**. Sixteen entries key on a
script no route table names.

Two mechanisms use the counters, and neither is a bar filling to a threshold:

- **Relative.** 25 of the 55 routes contain exactly one site, and all 25 are
  identical: `a = get("001"); b = get("002"); if (b < a) ... else ...`. Which
  counter is *ahead*, never by how much; a tie takes the `else`. Every other
  branch in the game is decided by the player's choice.
- **Absolute.** `FUN_10006000` answers whether a script's `STANDERDSCRIPT.INI`
  counter is past its amount. `cmp eax,[ebp-0x14]` then `jle` makes it
  **strictly greater** — `[01-00-N05]="002, 11"` passes at 12. It has exactly
  13 call sites, one per entry.

That two of the five are on the gauge is not the bar's decision. `FUN_10005c60`
ends with `if (name == "001" || name == "002") host->slot_0x30(1)`, slot `+0x30`
writes `engine + 0x79c`, and slot `+0x154` — which the control bar asks before
drawing the gauge over a faded-out bar — reads that member back. So the gauge
surfaces exactly when those two move, and `FUN_10026050` clears it again
through `slot_0x30(0)` once it has read them.

`_ZeroReset@4` walks the head's name list setting each to 0; that list, built
by `FUN_10006230`, is the only thing that ever touches `000` and `003`.

### The gauge geometry

`FUN_10026050` reads the two counters through host slot `+8` and derives a
signed lead for each side, then `FUN_10026540` sizes three sprites:

```text
lead_first  = (first  - second) * 2.5       this+0x48
lead_second = (second - first ) * 2.5       this+0x4c
```

A side's piece is up only while `lead + 208.5 > 417.0`, which needs a lead of
over 83 points; below that neither is up and a third, level piece is drawn
instead, which is what is on screen in ordinary play. Lengths clamp to 485.0.

The scale, bias and floor are **doubles** narrowed at the use site. Ghidra
prints them as `(float)_DAT_...`, and read as floats their bytes give `0.0` —
self-consistent, and wrong. The `.data` constants beside them (188.0, 9.0,
485.0, 118.0, 418.0) really are floats. `_DAT_1003d868`, the level piece's
417.0, is loaded with `flds` and so is a float; the 418.0 next to it at
`_DAT_1003d860` is a double. Both are in the same expression.

All three pieces cut `MenuBar_Chip.png`. `FUN_10023d50` loads
`System/MenuBar/MenuBar.png` into `+0x20` and `System/MenuBar/MenuBar_Chip.png`
into `+0x24`, and `FUN_10023aa0` renders those into the textures at `+0x18` and
`+0x1c` in that order, so the `this+0x1c` the gauge's source rectangles go
through is the chip sheet. In the shipped 799x408 sheet:

```text
y  57, x 1..485    the first counter's bar, flat orange
y 369, x 1..485    the second counter's bar, flat green
y 399, x 1..798    the level strip: green to x 370, orange from x 420
```

The bed sprite above it — record 67, `this+0x80` — carries `KOTONOHA` at the
left end and `SEKAI` at the right, which is what says which counter is whose:
`002` is the green one and fills from the left, `001` the orange one from the
right.

Each source goes in through `DX9Sprite2D` slot `+0x1c` as four separate calls
on the texture, `DX9Texture` slot `+8` being `x / width` and `+0xc` being
`y / height`. Ghidra chains those four into one `float10` expression and loses
their order; the disassembly's push order gives it, and it checks itself, since
the two `/ width` values have to pair with each other and the two `/ height`
values with each other. With `len` the clamped bar length:

```text
piece   source                                    destination
+0x98   (485 - len) + 1, 369,  len, 9             187.5,           8.5, len + 1, 10
+0x9c   1,               57,   len, 9             (485 - lead_first - 208.5) + 117.5, 8.5, len + 1, 10
+0xa0   188 - lead_second, 399, 417, 9            187.5,           8.5, 418,      10
```

So the level piece is a window onto the level strip whose origin slides 2.5
pixels per point of lead, walking the art's green-to-orange edge towards
whichever counter is ahead. At a tie the edge sits a pixel and a half left of
centre; at the extremes the window runs some 20 pixels off each end of the
sheet, where `D3DSAMP_ADDRESSU` clamps.

The destinations are a pixel larger than their sources in each axis and start
half a pixel back, which is the half-texel offset every other sprite on the bar
gets.

### The replay-mode indicator, and the box on the right

Widgets 15 to 24 are ten 12x17 cells in a row at x 676..796, inside a 124x19
trough at `this+0x8c`. Host `+0x98` picks the trough's sprite and makes the
cells pressable: record 69 is grey, record 70 a white-to-cyan gradient. The
caption all ten share — record 64, the twelfth strip — is the game's own words
for what they do: `Change transparency of replay mode indicator`.

The indicator is record 68, `this+0x94`, the word `REPLAYMODE`, and it goes at
`(697, 80) 97x19` in the strip's own space — **below** the 800x75 strip, so it
lands on the picture. `FUN_10024ca0` draws it outside the `this+0xbc` test that
gates every widget, and `FUN_10025690` never names it, so it neither waits for
the bar to drop down nor fades with it.

`FUN_10026ed0` is the whole of the slider. A press stores a level in
`this+0xec`, sets the indicator's colour to `round(level * 25.0) << 24 |
0xffffff`, and calls `FUN_10027030` to re-place the knob at `this+0x90`:

```text
widget   15  16  17  18  19  20  21  22  23  24
level     0   2   3   4   5   6   7   8   9  10
knob x   676 688 700 712 724 736 748 760 772 784
```

**Level 1 is unreachable**, and both halves agree on it: cell 0 stores 0 and
cell 1 stores 2, and `FUN_10027030`'s switch has no case 1 either — it would
place the knob from an uninitialised local. Its cases are
`760 + (level - 8) * 12` for 2..10 and `760 - 7 * 12` for 0, the 7 being a
double like the rest of the multipliers, one short of the 8 the pattern would
give because there is no level 1 to take the step between.

Ten levels reach an alpha of 250, not 255: `_DAT_1003d880` is `25.0`, and a
float — `flds`, not `fmull`. `FUN_10023d50` starts the bar at level 10, and
nothing saves the level; it lives and dies with the bar.

The knob's source rectangle is `(1, 379) 12x17`, set once in `FUN_10022650` and
never moved — it is one sprite that slides, which is why `FUN_10021c20` gives
`this+0x90` no record.

### What host `+0x98` is

`FUN_0042bef0` returns the film object's `+0x1e0` and `FUN_0042bf10` (host
`+0x94`) is the only thing that writes it: nothing in the executable names that
member otherwise, on either the object's offset or the interface's `+0x1b4`.
Both of the setter's callers are in the menu DLL — `FUN_1001dfe0`, a row of the
replay screen's play-data list, passes 1, and `FUN_1001d380`, an ordinary load,
passes 0. It is the same member `FUN_00431740` consults at every choice box to
take the slot's recorded answer. So `+0x98` is **playback is following a save's
recorded answers**, and the whole right-hand box is about that mode.

### When the gauge is up

`FUN_10024ca0` draws the bed and then whichever pieces are up, in the order
`+0x98`, `+0x9c`, `+0xa0`, twice over: once inside the "bar is up" test and
once outside it under host `+0x154`.

`FUN_10025690`, the fade, sets one ARGB on every sprite the bar owns **except**
those four, which it skips while `+0x154` is set.

Two more are never in its list at all: the rate readout at `this+0x88` and the
`REPLAYMODE` indicator at `this+0x94`. Nothing else sets their colour either —
only `FUN_10022650` does, once, opaque — and that null result was taken twice,
from the decompile and from a raw instruction scan of `0x10021000..0x10028000`.
`FUN_10024ca0` also draws both past the `this+0xbc` and host `+0x140` tests:
the hidden branch at `0x10024f4f` is `JNZ 0x10025205`, and `0x10025205` is the
instruction that begins the readout's own test, so this is the branch target
and not the decompiler's indentation. The consequence is that a rate of 2.0 or
more leaves one rate button on the picture after the bar has faded away.

### Nothing calls the bar's draw

The engine calls eleven slots on its MenuBar pointer at `engine + 0x330` —
`+0x04`, `+0x08`, `+0x0c`, `+0x1c`, `+0x20`, `+0x24`, `+0x28`, `+0x2c`,
`+0x30`, `+0x34`, `+0x38`, all of them landing inside the vtable, which is what
confirms the member — and `+0x14`, the draw, is not among them. Nor is it
called anywhere else: `FUN_10024ca0` is reached only through the vtable slot,
and the exe's `.text` holds no `CALL dword ptr [reg+0x14]` at all (the eleven
that match the byte pattern are `[EBP+0x14]` stack arguments in the CRT).

The bar is not drawn by being asked to. It is **registered as a graphics
module**, and the renderer walks the list:

```text
FUN_004253f0   the frame step
  FUN_004252e0   MenuBar +0x20, the update
  FUN_0040e540   Clear, BeginScene
    FUN_004144c0   DXGraphicModuleList slot +0x14
      for each module:  module -> slot +0x14      <- FUN_10024ca0
    EndScene, Present
```

`FUN_004230b0` is what puts it in: `MenuBar->+0x0c(device)` to load it, then
`FUN_0040e940(engine + 0x330)`, the global register, which forwards to
`FUN_00413f60` on the list at `DAT_0050b328`. `FUN_00423650` — slot `+0x08`,
the release — takes it back out through `FUN_0040e960`. `DXGraphicModuleList`
is an RTTI name off its vftable at `0x004d0fd0`, not an inference, and its
`FUN_004144c0` walks its modules calling each one's `+0x14` **with no test of
any kind**, ANDing the results.

So the draw runs every frame for as long as playback is loaded, whatever the
bar is doing, and the two sprites drawn past `this+0xbc` and host `+0x140`
really are on the picture with the bar gone. `DaysEngine` reproduces both. Skipping is not holding them
opaque — they keep whatever they last held. So a gauge raised while the bar is
up stays on screen at full alpha after the bar has faded away, and one raised
while the bar is already gone is pinned at nothing and never appears.

`+0x154` is raised by a delta to `001` or `002` and lowered by MenuBar vtable
`+0x38` (`FUN_10026050`), which the engine calls at the end of a script —
`FUN_00424020`, on the object at `engine + 0x330` — and again from
`FUN_00423a70` when playback starts. `_SetFeeling@8` is called from the choice
handler, so the window is from the player's answer to the end of that script.

### Exports

```text
CheckInputScript  CheckScript      CheckScriptNo    GetBackScriptFile
GetNextScriptFile GetPackFile      GetPackMax       GetPatchMax
GetReadScriptCount GetRouteMapPage GetScriptMax     GetStory
GetVersionToRoute LoadInitScript   SetDigScript     SetFeeling
SetPackName       SetRoot          SetScript        ZeroReset
searchRoot
```

`LoadInitScript` reads both affection tables into globals and parses their
heads. The rest are not recovered.
