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
24 fps, and against `[SkipFRAME]` totals. Files are UTF-8 (English) or UTF-16LE
with a BOM (Japanese); `FILMENGINE.INI [UnicodeFile]` selects this.

Exactly 14 commands exist across all 1,857 scripts:

| Command | Arguments after start timecode | Count |
|---|---|---|
| `PrintText` | speaker, text | 30,486 |
| `PlayVoice` | path, channel, speaker tag | 30,413 |
| `CreateBG` | `BGS`, image path | 13,024 |
| `PlaySe` | slot (1-5), path | 3,552 |
| `PlayMovie` | path, loop flag | 2,042 |
| `SkipFRAME` | *(end timecode only)* — total script length | 1,857 |
| `Next` | *(end timecode only)* — end of script | 1,857 |
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
  mixer for non-lipsynced voice.
- **166 voice references point at clips that do not exist**, out of 50,653 asset
  references across all scripts. They cluster on `PlayVoice` statements whose
  lipsync and speaker-tag fields were left blank, which reads as a scripter
  marking "no clip for this line". One background PNG is likewise absent
  (`Event01/01-00/01-00-T00/01-00-T00-009`). A missing asset must not be fatal.
- `MoveSom` drives a toy; the retail engine no-ops it without hardware.
- **`Next` and `SetSELECT` carry no targets.** The branch graph is not here.

### Retail data quirks the parser must absorb

All 1,857 scripts parse once these are handled. Each was found by running the
parser over the real packs, not by reading the format spec:

| Script | Quirk |
|---|---|
| `05-KC-F00` line 241 | A **semicolon inside dialogue** (`I know; I am, too.`). Statements have no escaping, so a `;` only terminates when the next non-whitespace character is `[` or the file ends. |
| `05-KI-OP1` | Written with **`, ` separators instead of tabs**, plus a stray whitespace-only ` ;` statement. Fall back to comma splitting only when a statement contains no tab, so commas in ordinary dialogue stay literal. |
| `03-KB-D10` line 29 | A `PrintText` with a **trailing empty field**. |
| `05-SE-C08` line 81, and 129 others | `PlayVoice` with **empty lipsync and tag fields** but the tabs still written. Fields must be read by position with defaults, not matched against an exact arity. |
| `01-00-E01` | Two timecodes with a **frame field of 26** in a 24 fps script. Fold the overflow in rather than rejecting. |

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
user's `.CMAP` and looking for a run of records that reproduces them at a
24-byte stride finds the table and validates it at the same time.

The match is scored rather than all-or-nothing, and anchored on any region
rather than the first:

- A region's bounding box can legitimately differ from its sprite rect —
  regions that abut have their boxes clipped by the neighbour. The route map's
  episode tabs do this, hence e.g. 11/15 boxes matching a table that is correct.
- A screen can have untabled widgets ahead of tabled ones in ID order, so the
  run does not always start at region 1.
- Below a threshold (3 exact matches and a majority) the search reports failure
  rather than drawing sprites from a wrong offset.

### Screens whose layout is not in a table

`SAVELOAD` and `REPLAY_PLAYDATA` are refused by the search, correctly: their
slot rows are laid out by a loop at runtime — ten rows at a 33px pitch — so
those rects exist nowhere in the binary. Those two screens need their layout
reproduced in code. Every other screen recovers: title (all three variants),
menubar, all three options pages, both backlogs, the exit popup, the replay
popups, `REPLAY_HSCENE` and all 15 route maps.

A few screens do not name their base art after the stem, because they share one
chip sheet and hit map across several backgrounds: `Exit/Popup` uses
`Popup_Exit.png` or `Popup_Title.png`, and `SaveLoad/SaveLoad` uses `Save.png`
or `Load.png`.

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

`Ini.GPK` holds eight files. The ones that matter:

- `STARTSCRIPT.INI` — entry point (`00/00-00-A00`), title/system BGM, logo.
- `FILMENGINE.INI` — save paths, system SFX, font, select graphics, fade timings.
- `DX9GRAPHIC.INI` — the four resolutions and pixel formats.
- `ENDLIST.INI` — the 22 endings and their title cards.
- `FEELINGSCRIPT.INI` / `STANDERDSCRIPT.INI` — per-script affection deltas,
  keyed by script name, feeding the route logic.

`Config.DAT` and the save files use a `DFLT` + zlib container (magic `DFLT`
followed by a raw zlib stream).

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

Not yet decoded. A 512 KB x86 DLL holding 1,825 script-name strings and
exporting:

    CheckInputScript  CheckScript      CheckScriptNo    GetBackScriptFile
    GetNextScriptFile GetPackFile      GetPackMax       GetPatchMax
    GetReadScriptCount GetRouteMapPage GetScriptMax     GetStory
    GetVersionToRoute LoadInitScript   SetDigScript     SetFeeling
    SetPackName       SetRoot          SetScript        ZeroReset
    searchRoot

The routing appears to be compiled code rather than a data table — the disasm
shows long chains of `cmp`/`jne` against an index followed by a string push,
which is what a large `switch` returning names compiles to.

Plan: a Ghidra headless script that recovers the graph from the user's own copy
and emits JSON at first run.
