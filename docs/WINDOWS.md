# Building for Windows

Windows binaries are **cross-compiled from Linux**. There is no Windows machine
in this project's loop and no CI to borrow one from, so the target is
`x86_64-pc-windows-gnu` — the one a GCC cross-compiler can link. The MSVC target
would need `link.exe` and a Windows SDK, neither of which exists here.

The result is `daysengine.exe` and the DLLs it needs, in
`target/dist/daysengine-windows-x86_64/`.

## What you need on the build machine

```bash
# Arch
sudo pacman -S mingw-w64-gcc cmake nasm clang
# Debian / Ubuntu
sudo apt install mingw-w64 cmake nasm clang
rustup target add x86_64-pc-windows-gnu
```

`nasm` is ffmpeg's assembler. Without it configure refuses to build the
hand-written x86 assembly, and WMV3 decode falls back to C — which a 24 fps
movie at 800x452 cannot afford.

`clang` is bindgen's parser. `rusty_ffmpeg` generates its bindings from
whichever ffmpeg headers it is pointed at, which is why this project tracks the
host's libav instead of lagging behind releases, and it is also why clang has to
be told which sysroot to read: see `BINDGEN_EXTRA_CLANG_ARGS` in the
`windows-release` recipe.

## The submodules

```bash
git submodule update --init --depth 1
```

`third_party/ffmpeg` and `third_party/sdl` are pinned upstream sources. They are
**not** the developer build path: `cargo build` and `just check` link the system
SDL3 and ffmpeg, which is what `docs/DEPENDENCIES.md` asks for. They exist
because a release archive cannot rely on the player having either — and on
Windows there is no distribution shipping libav security updates to inherit
from in the first place.

Both are built shared, never static, so a player can drop in their own build of
either without rebuilding the engine. For ffmpeg that is also what makes the
LGPL workable; see below.

## Building

```bash
just windows-deps      # SDL3 + ffmpeg for MinGW-w64. Slow; once per checkout.
just windows-release   # daysengine.exe, unpackaged
just dist-windows      # the above, packaged with its DLLs and licences
```

`windows-release` sets `PKG_CONFIG_LIBDIR` to **only** the two vendored
prefixes. That is deliberate: a Windows library that failed to build will fail
the link rather than silently resolving against the host's Linux one, which is
the failure mode that produces an `.exe` nobody can run.

## What ships, and why

An archive is a drag-and-drop, so it is flat and as short as the licences allow:

```text
daysengine.exe
avcodec-63.dll  avfilter-12.dll  avformat-63.dll
avutil-61.dll   swresample-7.dll swscale-10.dll
FFMPEG-SOURCE.txt  COPYING.LGPLv2.1
SDL-SOURCE.txt     SDL-LICENSE.txt
LICENSE
```

**One binary, and there is only one to ship.** The inspection tools are
subcommands of `daysengine` rather than a second program: `daysengine menu`,
`daysengine save --roundtrip`, `daysengine assets`. Running it with no
subcommand plays the game.

The player drops these into their own install beside `SCHOOLDAYS HQ.exe`. As
everywhere else in this project, **no game data is bundled**: the archive key
still comes from their executable and the widget tables still come from their
`SysMenuSDHQ.dll`.

There is no `SDL3.dll`, and there are no `.rlib` files. **SDL3 is zlib-licensed,
so it is linked into the executable** — the `static-sdl` feature, which
`tools/dist.sh` turns on and a developer build leaves off. Rust's own crates,
the `days-*` format readers included, are `.rlib`s that the linker puts inside
the binary; none of them is ever a file in an archive. libgcc and winpthreads
are static too, which is why the `.exe` imports nothing but the av* DLLs and
Windows' own system libraries.

`libavdevice` is built but not shipped: `rusty_ffmpeg` always puts it on the
link line and nothing in `src/media` calls it, so the linker drops it.
`tools/dist.sh` copies only the libraries the binary actually names in its
import table, so it never travels.

### ffmpeg is the one thing that stays a separate file

LGPL v2.1 permits static linking only against an obligation to let the player
relink the executable against their own build of the library. Shipping it shared
satisfies section 6 without one — and it is the arrangement this project wants
anyway, because a player who needs a patched libav can drop the DLL in.

### The ffmpeg in an archive is LGPL v2.1 and nothing else

`tools/build-ffmpeg.sh` configures `--disable-gpl --disable-nonfree
--disable-version3`, so no GPL-only or v3-only component can be pulled in by an
accident of configure, and `--disable-autodetect` so it links nothing that
merely happened to be installed on the build machine. It is built
`--enable-shared --disable-static`, which is the arrangement LGPL 2.1 section 6
permits without our having to offer object files of the engine itself.

`FFMPEG-SOURCE.txt` in the archive records the upstream URL, the exact commit,
and the full configure line; `COPYING.LGPLv2.1` is ffmpeg's own licence text.
Those two files are the compliance artifact — keep them in any archive you
publish.

### What is compiled in

Only what `src/media` actually decodes, which the games make unusually narrow:

| Format | Path |
| --- | --- |
| `.WMV` | ASF demuxer, WMV3 (VC-1 Main) decoder. The movies carry **no audio stream** — see the note atop `src/media/mod.rs` |
| `.OGG` | Ogg demuxer, Vorbis decoder. Every sound in the game is one of these |
| `.PNG` | decoded by the `png` crate, **not** by ffmpeg — then handed to libswscale, so a still and a movie frame reach the window through the same filter (`src/media/image.rs`) |

So: three decoders, two demuxers, no encoders, no muxers, no devices, no
network. Verified against `config_components.h` after a build.

**Filters are the deliberate exception and stay complete** — all 439 of them.
The chain between the decoder and the scaler is a filtergraph string out of the
player's own `DaysEngine.ini`, and `src/media/filter.rs` promises that
"everything libavfilter knows is available". Trimming the filter list would
break configs that already work against a system ffmpeg.

## Linux distribution builds

`just dist-linux` produces the same flat layout with `.so` files instead of
DLLs, and an `$ORIGIN` rpath so the archive's own libraries win over anything
installed. It needs `patchelf`.

A developer build still uses the system libraries and a shared SDL3. Vendoring
and static SDL3 are for archives.

## Known gaps

- **32-bit Windows is not built.** The retail games are 32-bit, but nothing in
  this engine reads their code at runtime — `days-gpk` recovers the archive key
  and `days-ui` the widget tables by *parsing* the PE files, which a 64-bit
  process does as happily as a 32-bit one.
- **Only `x86_64-pc-windows-gnu` is wired up.** An MSVC build would want vcpkg
  for both libraries and a different set of recipes; nobody has needed one.
