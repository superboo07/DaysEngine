# Status

What is recovered and what works, per title. **This file is the system of
record for that tracking.** It lives here rather than in `README.md`, which is
the user's own document and is not maintained by agent sessions. The recovered
behaviour itself, with its provenance, belongs in `docs/FORMATS.md` and in the
doc comment beside the code.

Early. School Days HQ is the title the engine was brought up on and the one the
rows below describe; Shiny Days is being brought up on the same code.

## Platforms

| | State |
|---|---|
| Linux x86_64 | **Works.** The platform the engine is developed and verified on. Every build links the pinned SDL3 and ffmpeg from `third_party/`, static and shared respectively |
| Windows x86_64 | **Builds.** Cross-compiled from Linux for `x86_64-pc-windows-gnu` against vendored SDL3 and ffmpeg; `just dist-windows` packages the `.exe` files with their DLLs. **Not yet run against a real install on Windows** — the build is verified, the game is not |
| macOS | Not worked on, and not planned |
| Android | **Builds.** `arm64-v8a` and `x86_64`, as an APK; the engine is `libdaysengine.so` and SDL calls its `SDL_main`. The player picks their install in Android's own folder picker, and it is read through the Storage Access Framework. Touch is a mouse, and Back is Cancel. **Not yet run against a real install on a device** — the build is verified, the game is not. `docs/ANDROID.md` |

Two things in the engine are platform-specific, and both are about the
machine rather than the game. `install::clock` asks SDL for the local UTC
offset a save slot's timestamp line needs. And **Android has no directory an
app may open by name** — what the player grants is a Storage Access Framework
tree, reached by document id through `ContentResolver` — so every read and
write of a file in the install goes through `install::storage`, which is
`std::fs` on a desktop and `install::saf` there. Nothing above that seam knows
which. Everything else is portable Rust over SDL3 and libav.

`docs/WINDOWS.md` and `docs/ANDROID.md` have the cross-builds. Both, and the
Linux archive, are built in one pinned image; `tools/in-container.sh` says
why.

**The file formats transfer for free.** Both titles are FILMEngine, so the
archives, the scripts, the glyph store, the hit maps, the atlases and the save
container are the same readers with no branch in them at all:

| | School Days HQ | Shiny Days |
|---|---|---|
| `.GPK` packs | 29 packs, 69,936 entries | 27 packs, 89,273 entries |
| `.ORS` scripts | 1,857, all parsed | 2,587, all parsed |
| `FONTDATA.DAT` | 22,420 glyphs | the same file, byte for byte |
| Routes | 55 routes, 1,857 scenes | 103 routes, 2,301 scenes |
| Replay scenes | 41 over 4 pages | 36 over 3 pages |
| Save files | 22, all round-trip | 78, all round-trip |
| Control bar | 25 widgets | 16 widgets |

**Every screen is a fresh recovery.** The two modules lay their menus out
differently and Shiny Days adds screens School Days HQ has none of, so each one
is recovered from the player's own module: the Option screen's tabs, the Replay
screen's split, the control bar, the dress-select screen (menu mode 9, which
School Days HQ has no case for at all, reached from the title's `START`), and
Shiny Days' fifteenth `.ORS` command. `daysengine menu --check-all` drives every screen of either install
headlessly; both pass with no warnings.

The largest recovered difference is that **Shiny Days ships 288 scenes twice**.
The second recording of each is the same lines and the same voices over
different art, and which one plays depends on the uniform the player picked on
the dress-select screen — one character of the script's name is replaced, and
the name that results is what plays, what the recorded choices are filed under,
and what a save slot carries. `docs/FORMATS.md` has the recovery.

What works today:

| Area | State |
|---|---|
| `.GPK` archive reading | **Done** — all 69,936 entries across 29 packs decode and verify, and all 89,273 across Shiny Days' 27. A pack is not one file: the engine layers up to `_GetPatchMax@0()` patch overlays — `System.GPK.000` through `System.GPK.009` — over each one, highest first, and so does this. Neither retail install ships one; a patched or translated install is what they are for. Split volumes (`System.GPK1`..`F`) are recovered in `docs/FORMATS.md` and not implemented — no install ships one to verify a reader against |
| Archive key recovery | **Done** — read from the user's own executable, not embedded |
| `.ORS` script format | **Decoded** — 14 commands, and the fifteenth Shiny Days adds, documented in `docs/FORMATS.md` |
| `.CMAP` UI hit maps | **Decoded** |
| `_CHIP` sprite atlases | **Decoded** — the widget table is recovered from the user's own menu module by content, not by a hardcoded address |
| `FONTDATA.DAT` glyph store | **Decoded** — all 22,420 glyphs render |
| Video / audio decode | **Done** — matches ffmpeg's own output |
| Playback / windowing | **Plays a scene** — video, audio, timeline, dialogue |
| UI rendering — title, menubar, options, replay grid, backlog, route maps | **Composites** — the game's own art, at all four resolutions; `daysengine ui` renders any screen headlessly |
| UI rendering — save/load | **Works** — the game's own LOAD/SAVE screen, its ten rows, ten page buttons and route-map button, from the player's art; picking a row loads or writes that slot. Its chip table cannot be reached from the hit map: ten of its rows are one sprite behind two hit regions each, and the ten comment panels are three rows tall, so only the twelve buttons in between reproduce a record. The table is anchored on those twelve and the other twenty read at the indices the shipped code reads them at — extrapolating them at the table's stride was enough on School Days HQ and lands on the play-data list's table on Shiny Days, whose records reproduce all thirty-two hit boxes to the pixel and draw every row a third of its width. That one row sprite is what both halves light, so pointing at a comment highlights the whole row. The ten records that are three rows tall are not hover art at all: they size the panel of the expanded-comment tooltip, which works — pointing at a row opens its whole comment over the list, wrapped the way the DLL wraps it, opening upwards on the last two rows so it cannot run off the bottom. The comment column and the tooltip are both gated on `FILMENGINE.INI [TextInput]`, which is what host `+0xd8` answers. Naming a save works: the original opens a Win32 dialog from the executable's own resources, and this draws that template — caption, prompt, button captions and every rectangle out of the player's exe — with SDL text input behind it so an IME works. Every row shows its timestamp, chapter and comment: the DLL rasterises all thirty columns into one off-screen surface and gives each column a sprite that cuts it, and both the cut and the placement are recovered — including the fact that the second band of ten records is not a duplicate of the first but where the comment goes. **The two modules lay a row out differently and each set is recovered from its own module**: School Days HQ puts the timestamp first and the chapter after it in a 2048x1024 surface, Shiny Days puts the chapter in a narrow cell at the left with the timestamp beside it, squeezed 548 into 189, in a 1024x1024 one, and expands a comment into a buffer of its own. Which set a screen uses is decided by the menu module's export table. **Shiny Days' list slides**: it is a strip of six page-panels the module stacks from the player's own `SaveLoadList.png`, and a page button slides it one page at a time — twenty frames of linear travel each, so page 1 to page 10 is nine of them. What confines the strip to the list is the screen's own base art: nothing scissors it, and `Load.png` is opaque everywhere but the window the list sits in, so the module draws the strip, the row highlight and the rows and *then* the base art over them. The row highlight and the tooltip are the two things the slide takes away; a page button clicked mid-slide redirects the strip rather than being refused. The list **drags** too, on the same held flag the Option screen's sliders latch: a press anywhere takes hold of it, the strip tracks the pointer one for one, and letting go snaps it to the nearest row — a tenth of a page — or clamps it to the ends. So the list can rest between two pages, and then its ten records span the join and name the ten slots from that row on, which is what the module's `+0x5d0` is for. A drag's settle has a page decision of its own, `FUN_1001f130`, which asks which bank the scroll ended up over rather than which way a step went |
| UI rendering — replay play-data list | **Works** — the player's own save slots, three columns a row, the expanded comment on hover; picking one plays that save back by the answers it recorded |
| UI input handling / screen state machine | **Works** — title, settings and replay are live: pointer, keyboard and controller, each screen's own widget-to-action table out of the DLL, both popups, and the mode graph out of `SystemInit`. The title's `START` reaches the dress-select screen on a module that has one, which is how Shiny Days asks for a uniform before a new game and the only arm of that dispatch the two titles spell differently |
| Settings | **Works** — `Config.DAT` is read and written back, volumes reach the mixer, and the Option screen's three tabs drive it |
| Replay | **Works** — the 41 scenes, their unlock flags, their scripts and the branch tables eleven of them walk are recovered from the user's own `SysMenuSDHQ.dll`; picking one plays it through, following the player's choices. Shiny Days lays the screen out differently and its 36 scenes come out of `SysMenuSD.dll` the same way |
| In-game control bar | **Works** — a drop-down over the top 75 pixels, translucent over the frame, ramping in over 300ms and out over 1000ms exactly as the original does; all 25 widgets, their enabled rules, their resting and hover art and their captions, out of the DLL's own dispatch, reachable by pointer or by a controller selection that walks the live ones; pause and the auto flag act, and the three seek buttons are the game's own — `Rewind to beginning of current part` restarts the part and, pressed again inside the first 72 frames, goes **back** a part through `_GetBackScriptFile@12` with that part's feeling deltas taken back off the counters and no read mark; `Skip to end of current part` and `Skip to next choice` both stop one second before a choice the part still raises, and differ only in that the second chases one across the parts that follow; the five rate buttons fast-forward, scaling the script clock and retiming the audio with it — resampled at 1x, 2x and 4x and muted above, which is `FUN_004433d0`'s own rule. The buttons that move to the next script hand over to the branch graph, and the rewind asks it backwards — `daysengine route --play <script> --steps N --rewind N` walks a route out and back. All four menu buttons open the screen they ask for — save, load, backlog and Option. The right-hand box is the replay-mode indicator's transparency slider: its ten cells set how solid the `REPLAYMODE` sign on the picture is drawn, and the whole box lights up only while playback is following a save's recorded answers. `daysengine bar` prints the table |
| Choice boxes (`[SetSELECT]`) | **Works** — raised and decided on the script clock, so an ignored choice still times out; the shipped hit maps where they exist and the game's own screen split where they do not, pointer, keyboard and controller — the four navigation slots the original's own `+0x148` carries and `FUN_0044de50` reads — and a random pick while skipping, as the original does. `daysengine select` prints the map and metrics |
| Subtitles | **Works, the game's own way** — broken by `FUN_0043f600` (62 columns, word-wrapped at spaces, English only, `\n` as a hard break, ruby marks recognised), spaced by the recovered pitch and kerning table rather than by measuring the glyph, and placed by `FUN_0044bf30`: centred on each line's own width, anchored to the bottom, at the per-resolution scale, with `[LeftArrangement]` switching to a left-aligned block. The speaker name is not drawn, because the original never hands it to the text layer — it goes to the backlog instead — and the whole block is behind the `TextView` setting. There is no text box and no name box to draw: `FUN_0044bf30` draws the lines, the ruby and the choice blocks and nothing else, over the picture |
| Backlog | **Works** — the control bar's third menu button raises it over playback, and Close puts the player back where the script was paused. Both screens, `BackLog_Horizon` and `BackLog_Vertical`, draw from the player's own art at all four resolutions, and the lines are drawn into them: wrapped by `FUN_10002a90`'s own rule (48 columns in English, 27 in Japanese with the shipped kinsoku sets, 13 down a vertical column), stacked by `FUN_10003600` with the entry the player is on centred and its neighbours measured off their own heights, and rasterised into the `0x800` x `0x400` buffer one sprite stretches over the whole screen. `[BackLogType]` picks the flow and `[AgateUsing]`/`UseAgate` gates the ruby, which no shipped line uses. `setSystemInit`'s case 3 is **recovered**: it selects `DAT_1004ffc8`, whose static-init thunk `FUN_10038360` calls `FUN_10001cb0`, which installs `MENU::BackLogView::vftable` — so the bar's third menu button is this screen. The engine keeps the lines the way the original does — one record per `[PrintText]`, running for the whole session, trimmed back to where the script started when a restart moves the timeline, and never emptied otherwise, because nothing in the menu module asks the host to empty it. `daysengine backlog <script>` draws the screen over one script's lines and `daysengine menu --from-bar 3 --lines-from <script>` drives the screen itself |
| Route / branch graph | **Recovered** — the 55 routes, their 1,857-entry script tables and all 55 transition state machines come out of the user's own `RouteProcSDHQ.dll`, the tables by content and the machines by decoding the handlers, with no address embedded; Shiny Days' 103 routes and 2,301 scenes come out of `RouteProcSD.dll` by the same decoder. Scripts chain: a choice moves the player through the graph and credits what it earns. `daysengine route --edges` prints every edge and checks the graph against the tables. Shiny Days' module decides its own endings too, through three exports School Days HQ's does not have: whether a script's `[EndRoll]` plays at all, which of a pair of recordings it is — the three positions that answer are the three end rolls the packs ship as an `A`/`B` pair — and whether the `Ex01` pack's episode title card goes over the start of the credits, which one position asks for and the pack holds exactly one card for. Decoded from the player's own module the same way; `daysengine route <script>` reports what each answers |
| Affection gauge | **Works** — the five counters, both tables, the relative test that 25 routes branch on and the 13 absolute thresholds, and the gauge on the control bar drawn from the game's own art: the strip that slides green towards Kotonoha and orange towards Sekai, and the two full-length bars either side takes once it is more than 83 points ahead. A delta raises it and it runs the original's ramp — `SeUp` or `SeDown` for the direction the counter moved, then a slide to the new lead over 1.5s, a 2s hold and down again — opaque throughout, over a bar that has otherwise faded away, and carrying on across the cut into the next scene the way the original does. `daysengine route` shows a save's counters and which way the test falls; `daysengine bar --feeling 001,002` draws the gauge at any pair, and `--gauge --feeling-was` runs the ramp between two pairs |
| Save data | **Works, both ways** — `Save/SaveFileNNN.DAT` is a log of where the player is, every story point they reached with the state they reached it in, and the choice they made at every script. Read and written, along with `GlobalFlag.DAT` and the line the save screen shows. Every one of the 22 files in the School Days HQ test install and all 78 of Shiny Days' read and write back **byte for byte identical**, so a save this engine writes is a save the original game reads. The one thing the two titles spell differently is tag 1's version field, an `f32` against a wide string, and that is the route module's doing rather than the save's. `daysengine save --slot N` decodes one; `daysengine save --roundtrip` is that check |
