# Working on DaysEngine

DaysEngine is a from-scratch, cross-platform reimplementation of Overflow's
FILMEngine, the engine behind *School Days HQ*. The user drops our binaries
into their own original install and it plays like the real game.

Two things are non-negotiable:

1. **No bundled game data.** Everything is derived at runtime from the user's
   install — the archive key from their `SCHOOLDAYS HQ.exe`, the UI widget
   tables from their `SysMenuSDHQ.dll`. Never hardcode a recovered address,
   never embed extracted data.
2. **The game's own UI**, composited from their art — not a lookalike we
   rebuilt.

---

## Reverse engineering: the rules

Almost everything this engine does is a behaviour recovered from two shipped
binaries. The single largest source of wasted work on this project has been
**answering a question by assumption when the answer was sitting in the
binary**. These rules exist because each one has already been paid for.

### Decompile first. Always.

Do not infer a format, a layout, or a decision rule from the data when the
code that produces it is right there. A self-consistent theory that fits the
bytes is not evidence: the font glyph encoding was once inferred into an answer
that consumed every stream exactly, to the byte, and still rendered noise.

Read the parser. Then check the data against it.

### Follow the call all the way to the value

Finding the *question* is not finding the *answer*. When a caller asks
something through an interface, keep going until you reach the code that
produces the value — the actual store, the actual constant, the actual INI key.

The failure that motivated this rule: the menu DLL asks its host "is this save
all-clear?" through vtable slot `+0xe8`. That was written down as "the
`AllClear` save flag" and built on for two sessions. It is not. Slot `+0xe8`
returns a plain member that the live constructor sets to zero and nothing else
ever writes, so the answer is *always false* and the screen it selects is
unreachable in the retail build. One decompile of the slot would have caught
it. Nobody did that decompile, because the question had already been given a
plausible name.

A name is not a finding. A decompile is a finding.

### State the anchor, and verify it independently

Every offset table, vtable, or structure mapping rests on one anchor — "this
function is slot N", "this object starts here". Write the anchor down and
confirm it a second way before building on it. In this codebase the host vtable
was confirmed by three independent facts: the pointer run in `.rdata` begins
exactly there, it is preceded by an RTTI pointer, and the one slot already
known from the other side lands at the expected offset.

**A C++ object can have more than one vtable.** The host interface here is a
secondary base subobject installed at `[object+0x2c]`, so every offset the DLL
uses is `0x2c` below the executable's own. A scan for writes at the DLL's
offsets found nothing and nearly produced the conclusion "this member is never
written" — which was wrong, and would have been wrong in a way that looked like
a discovery. Before concluding a member is never written, prove you are
scanning the right offset.

### A null result is a claim, and needs the same standard

"Nothing writes this", "no such string exists", "this branch is unreachable" —
these are findings that need evidence, not the absence of evidence. Say which
method you used and what its blind spots are. Confirm with a second, different
method. `FUN_00421b30` really does have zero callers; that was checked with
Ghidra's reference index *and* a raw byte scan of `.text`.

### "The original ships this bug" is the claim that needs the most evidence

When a reading predicts something visibly broken — a line drawn off its own
sprite, a screen that cannot be reached, a value nobody uses — the explanation
is nearly always that the disassembly has not been read far enough. It is the
most comfortable conclusion available and the least verified, so treat a
prediction of brokenness as a signal to keep going, not as a result.

Three questions close most of them: what are the buffer's real dimensions, who
writes the value **last**, and what runs every frame. Then check the prediction
against a screenshot of the retail game before writing anything down. A claim
of this kind is not recovered until something outside the decompiler agrees
with it.

Both of the Shiny Days Replay list's "shipped bugs" were mine, not Overflow's.
The expanded comment's pen goes back to `0x400` after every twenty characters
while the sprite cuts from x 0, 986 wide — so lines two and three are invisible.
They are not: `FUN_100296a0` builds that buffer `(0x400, 0x100, 0x208888)`, and
the blitter takes a pitch rather than a width, so `0x400` is the first pixel of
the next scanline. And the comment centring "applies under `[UseEnglish]`" —
except the retail screen shows comments hard left in an English install, because
`FUN_100267c0` wipes all six panels' centres every time it fills one and
`FUN_1002c1f0` re-reads that array every frame.

This does not forbid the finding. A shipped bug that really is one stays, the
way the save/load tooltip's two uninitialised floats stay — that one is
established by reading the branches that never write them. What is forbidden is
reaching for it as the explanation.

### Never fabricate a value

If something is not recovered, the code and the docs say **"not recovered"**.
Do not pick a plausible constant, a likely index, or a sensible default and let
it pass as recovered. There are several honest "not recovered" notes in
`src/ui/menu.rs` and `docs/FORMATS.md`; match that style. A gap that is labelled
is a task. A gap that is filled with a guess is a bug that looks like a
feature.

### When a claim turns out to be wrong, delete it

Documentation here is inherited as fact. The next session reads a doc comment
the way it reads the binary, and by the time a claim is found to be wrong the
word it introduced has already spread into names, tests and `docs/FORMATS.md`.
So a wrong claim is **replaced and removed**, never annotated. Do not write
"previously thought to be X", do not keep the old name with a correction beside
it, and do not let a fixed paragraph sit next to the one it fixes. Change the
claim, rename everything it leaked into, delete whatever only existed to support
it, and then grep for the wrong word and confirm none of it survived.

The failure that motivated this: the Option screen's third tab was written up as
"gamepad configuration", inferred from the DirectInput-shaped calls around it.
It is SOMCON, a toy on a COM port — the DLL imports no input API at all, its own
art says `Port number`, and one look at the import table would have said so. By
then the wrong word was in a module doc, a struct, three field and variant
names, nine test names and two sections of `docs/FORMATS.md`. Fixing only the
sentence that was literally false would have left the whole codebase reading as
though it drove a gamepad.

This does not apply to limitations. "Not recovered" notes, shipped bugs written
down as such, and the reasons a screen is unimplemented are all findings, and
they stay. What gets deleted is the claim that is false.

### Verify against the real install before claiming it works

The headless tools exist for this: `days menu`, `days ui`, `days save`,
`days render`. Run them against the user's install and look at the output.
"The tests pass" is not the same as "the screen is right" — the widget tables
were correct while the screen it chose was wrong, and only a screenshot of the
real game caught it.

**Verify with the headless tools, never by launching the game.** Do not run
`daysengine` unless the user asks for it in so many words. It opens a window on
their desktop and takes over their machine, and playing the game is the part
they want to do themselves. Build it — `cargo build --release` — and tell them
it is ready; the run is theirs.

### Record the provenance next to the behaviour

Every recovered rule carries the function that established it, in the doc
comment where the rule lives (`FUN_0041fee0`, `FUN_00420310`, and so on) and in
`docs/FORMATS.md`. That is what makes the next session able to re-check a claim
instead of inheriting it.

---

## Ghidra

Recovering anything new means decompiling the shipped binaries, so you need a
Ghidra project holding them. **There isn't one in this repository and there
cannot be**: `SCHOOLDAYS HQ.exe` and `SysMenuSDHQ.dll` are the user's game
files, and this project ships no game data. Everyone who does RE work here
builds their own project from their own install.

Nothing in the engine depends on that project existing — it is a research
tool, not part of the build. `cargo build`, the tests and the `days` inspection
commands all work without it.

### Creating one

Requires a Ghidra install and a copy of the game. Nothing below assumes where
either lives — set these three to whatever is true for your machine.

```bash
export HEADLESS=/opt/ghidra/support/analyzeHeadless   # wherever Ghidra is
export GHIDRA_PROJ=~/ghidra_projects/SDHQ             # any empty directory
export GAME="/path/to/School Days HQ"
mkdir -p "$GHIDRA_PROJ"
```

Import and analyze each binary once. This takes a while — the executable is
the slow one.

```bash
"$HEADLESS" "$GHIDRA_PROJ" SDHQ -import "$GAME/SCHOOLDAYS HQ.exe"
"$HEADLESS" "$GHIDRA_PROJ" SDHQ -import "$GAME/SysMenuSDHQ.dll"
```

### Running a script against it

Once imported, **do not re-analyze**; pass `-noanalysis`.

```bash
"$HEADLESS" "$GHIDRA_PROJ" SDHQ -process "SCHOOLDAYS HQ.exe" -noanalysis \
  -scriptPath "$GHIDRA_PROJ" -postScript your_script.py
```

Use `-process "SysMenuSDHQ.dll"` for the menu modules.

Addresses quoted in this repository's docs and comments — `FUN_0041fee0`,
`FUN_0042baf0`, the host vtable at `0x004d2894` — are Ghidra's default names
for the retail build at its default image base, so they should match in a
freshly imported project. If they don't, you have a different build, and every
recovered claim needs re-checking against it rather than trusting the numbers
written down here.

Scripts and their `*_output.txt` are kept beside the Ghidra project, not in
this repository (see the no-stray-files rule below). Write new ones in the same
style as whatever is already there.

### Hazards, all of which have cost a cycle

- Scripts run under **Jython 2.7: ASCII only.** One em dash in a docstring is
  a `SyntaxError`.
- `mem.getBytes()` returns **zeros for `.data`** in `SysMenuSDHQ.dll`, and
  vtable scans of the exe image come back empty. Code xrefs and the decompiler
  are fine; raw memory reads are not. Map RVA to file offset and read the file
  yourself (DLL image base `0x10000000`, `.data` va `0x10043000` raw
  `0x40e00`).
- Objects in `.data` built by C++ static initializers **have no vtable in the
  file**. Name such classes through the CRT static-init thunks and neighbouring
  string literals.
- Ghidra 12.1.2 has **no `DefinedDataIterator.definedStrings`**. Use
  `program.getListing().getDefinedData(True)` and check `data.hasStringValue()`.
- A whole-program decompile sweep of the exe takes **over 20 minutes**. Scope
  sweeps to an address range.
- `pkill -f analyzeHeadless` **matches its own command line** and will kill
  your shell. Don't.

A plain Python scan over the raw file is often the faster and more reliable
second opinion, especially for reading tables out of `.data` — and having two
methods that agree is the standard the rules above ask for.

---

## The code

```text
src/install/      the player's install: vfs, ini, config, save
src/media/        audio + video decode through system ffmpeg
src/playback/     stage, mixer, lipsync, text, compose
src/ui/           menu, screen, options, replay, ending
src/main.rs       `daysengine` — the game (SDL3)
src/bin/days.rs   `days` — offline inspection tools

crates/days-gpk     GPK archives
crates/days-script  .ORS timelines
crates/days-font    FONTDATA.DAT
crates/days-save    Save/GlobalFlag.DAT flag store
crates/days-ui      CMAP hit maps + _CHIP atlas recovery
```

**This is one ordinary crate, not a pile of them.** New engine functionality is
a module in one of those four `src/` groups — find the group it belongs to
rather than dropping another file at the top level. Only add a crate for a
standalone reader of a format the game ships, and say why in the commit.

**One engine, not one per title.** A second game is not a second copy of the
code. When a title needs behaviour a type or a function nearly has, **extend
that type or function** so it serves both — a new parameter, a new arm, a
recovered value read from the player's own module instead of written down. Do
not add a parallel `Foo`/`FooSD`, a second enum whose variants restate the
first's, or a second copy of arithmetic that already exists somewhere. Before
writing anything, grep for what already does the job and call it; if two places
end up doing the same thing, factor the shared half out rather than leaving both.

Two things are not duplication and are expected: a **per-module screen
recovery** gets its own module under `src/ui/` when a title lays a screen out
differently, and a **recovered rule** that genuinely differs between titles gets
its own branch with its own provenance. What must not be duplicated is the
logic underneath them.

Design decisions already made and not up for re-litigation:

- Rust + SDL3 + **system** ffmpeg, linked not vendored, so we inherit the
  distro's CVE patches on two large C media parsers.
- The runtime is a **scheduler, not an interpreter**. `.ORS` files are
  timelines; every statement owns a `[start, end)` window in frames at 24 fps.
- **A missing asset logs and continues**, never fatal. A screen that cannot be
  drawn returns `Action::Unavailable` and leaves the player where they were.
- **Headless rendering is the verification path.** The SDL path draws through
  the GPU and cannot be inspected from a test.
- **Menu modes stay the game's own integers** (`Mode(2)` is the title) because
  they cross the engine/menu boundary in the original.
- `src/media` is the only module allowed `unsafe`, and only because Rust
  requires the keyword on FFI calls into system libav.
- **All pack and CMAP lookups are case-insensitive.** INIs say
  `System/Title/TitleBase.png`; packs store `TITLE/TITLEBASE.PNG`.

---

## Dependencies

Read `docs/DEPENDENCIES.md` before adding anything.

- Every direct dependency pinned **exactly** (`=X.Y.Z`), never a caret range.
- `Cargo.lock` is committed; the toolchain is pinned in `rust-toolchain.toml`.
- **Every build uses `--locked`.** A "lockfile needs updating" failure is the
  gate working, not an obstacle to route around.
- `cargo deny --locked check` must stay clean.
- Prefer writing ~150 lines over pulling a general-purpose crate for one small
  need.

---

## House rules

- `cargo fmt`, `cargo clippy --all-targets -- -D warnings`, and the test suite
  all clean before a commit. `just` is not on PATH; run the cargo commands from
  the `justfile` directly.
- **No stray files in the repo.** Scratch output goes to a temp directory.
- Backticks in `git commit -m` get shell-expanded and silently mangle the
  message. Use `git commit -F <file>`.
- **Never create a branch.** Commit to whatever branch is checked out. This
  project works on `master` directly and there is no remote to open a pull
  request against, so a branch is pure friction — it hides finished work behind
  a merge nobody asked for. Branch only when explicitly told to.
- Indented code blocks in `//!` module docs become doctests and fail to
  compile. Use ```` ```text ```` fences.
- **Never go through the user's personal files.** If something is needed that
  is not in the repo or the game install, ask for it.
