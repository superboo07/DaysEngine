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

    width   u32 LE
    height  u32 LE
    pixels  [u8; width * height]   region ID, 0 = no region

One byte per pixel naming the widget under it. Shipped per screen at four
resolutions, by filename suffix:

| Suffix | Size | `DX9GRAPHIC.INI` key |
|---|---|---|
| *(none)* | 800x600 | `DisplayWidthSize` / `DisplayHeightSize` |
| `_WIDE` | 800x450 | windowed widescreen |
| `_WIDE_NOTE` | 1024x576 | `FullNoteWidth` / `FullNoteHeight` |
| `_WIDE_FULL` | 1280x720 | `FullWideWidth` / `FullWideHeight` |

Each screen is `NAME.PNG` (base art), `NAME_CHIP.PNG` (widget state sprites) and
`NAME*.CMAP`. Region IDs are dense from 1.

**Open question:** the packing of `_CHIP` sheets. For `TITLE` the five regions
are 116x38 and the sheet is 584x78 — five sprites across, two state rows. For
`MENUBAR` the region widths sum to 920 against a 799-wide sheet, so the layout
wraps or is grouped by widget class. Needs pixel inspection.

---

## `FONTDATA.DAT` / `FONTDATA_ENG.DAT` — font

39 MB. Opens with what appears to be 65,536 `u32` entries (256 KB — one slot per
Unicode BMP code point) indexing into glyph data that follows. Values seen are
`0x0004xxxx`, consistent with an offset biased by the 0x40000-byte header.
Not yet decoded past the header.

---

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
