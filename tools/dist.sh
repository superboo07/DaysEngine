#!/usr/bin/env bash
#
# Builds a release archive.
#
#   tools/dist.sh linux     -> target/dist/daysengine-linux-x86_64.tar.gz
#   tools/dist.sh windows   -> target/dist/daysengine-windows-x86_64.zip
#
# An archive is a drag-and-drop: the player unpacks it into their own install
# beside the game executable and runs it. So it is FLAT -- no subdirectories on
# either platform -- and it contains as few files as the licences allow.
#
# There is one binary to ship, because there is only one: the inspection tools
# are subcommands of daysengine rather than a second program. Running it with no
# subcommand plays the game, which is what a player unpacking this wants.
#
# The archive carries no game data. The archive key still comes from their
# executable and the widget tables still come from their menu DLL.
#
# ---------------------------------------------------------------------------
# Why SDL3 is inside the binaries and ffmpeg is not
#
# SDL3 is zlib-licensed. zlib attaches no condition to linking, so SDL3 is
# linked statically (the `static-sdl` feature) and no libSDL3 ships at all.
#
# ffmpeg is LGPL v2.1, which permits static linking only against an obligation
# to let the player relink the executable against their own build of the
# library. Shipping it shared is how section 6 is satisfied instead, and it is
# the arrangement this project wants anyway: a player who needs a patched libav
# drops the DLL in. So the av* libraries stay separate files, and
# FFMPEG-SOURCE.txt and COPYING.LGPLv2.1 travel with them.
#
# Everything else the binaries need -- libgcc, the C++ runtime, winpthreads,
# every Rust crate including the days-* format readers -- is already static.
# Rust links its own rlibs into the executable; none of them is ever a file in
# an archive.
# ---------------------------------------------------------------------------
#
# This is the one place the archive layout is written down. The justfile and
# .vscode/tasks.json both call it rather than restating it, and `just` is not
# required to run it.
set -euo pipefail

target=${1:-}
case "$target" in
linux | windows) ;;
*)
    echo "usage: $0 {linux|windows}" >&2
    exit 2
    ;;
esac

root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"

sdl=$root/target/sdl/$target
ffmpeg=$root/target/ffmpeg/$target
for prefix in "$sdl" "$ffmpeg"; do
    if [ ! -d "$prefix/lib/pkgconfig" ]; then
        echo "missing $prefix -- run tools/build-sdl.sh $target and tools/build-ffmpeg.sh $target" >&2
        exit 1
    fi
done

# Only the vendored prefixes are on the search path. A library that failed to
# build then fails the link, rather than silently resolving against the build
# machine's own copy -- which on a cross-build produces an .exe nobody can run,
# and on a native build produces an archive that works here and nowhere else.
export PKG_CONFIG_LIBDIR="$sdl/lib/pkgconfig:$ffmpeg/lib/pkgconfig"
export FFMPEG_PKG_CONFIG_PATH="$ffmpeg/lib/pkgconfig"

out=$root/target/dist/daysengine-$target-x86_64
rm -rf "$out"
mkdir -p "$out"

if [ "$target" = windows ]; then
    export PKG_CONFIG_ALLOW_CROSS=1
    # bindgen parses ffmpeg's headers with clang, which otherwise reads glibc's.
    export BINDGEN_EXTRA_CLANG_ARGS="--target=x86_64-w64-mingw32 -I/usr/x86_64-w64-mingw32/include"
    cargo build --locked --release --features static-sdl \
        --bin daysengine --target x86_64-pc-windows-gnu
    built=$root/target/x86_64-pc-windows-gnu/release
    binaries=(daysengine.exe)
else
    cargo build --locked --release --features static-sdl --bin daysengine
    built=$root/target/release
    binaries=(daysengine)
fi

for binary in "${binaries[@]}"; do
    cp "$built/$binary" "$out/"
done

# Copy only the libraries the binary actually asks for by name, rather than
# everything the prefixes hold. The linker drops what nothing references --
# libavdevice, for one, which rusty_ffmpeg always puts on the link line and no
# code here calls -- and an archive should not carry a megabyte nothing opens.
needed() {
    if [ "$target" = windows ]; then
        x86_64-w64-mingw32-objdump -p "$1" | sed -n 's/^\s*DLL Name: //p'
    else
        readelf -d "$1" | sed -n 's/.*Shared library: \[\(.*\)\]/\1/p'
    fi
}

for binary in "${binaries[@]}"; do
    while read -r lib; do
        [ -n "$lib" ] || continue
        # Everything the system provides stays the system's.
        for dir in "$ffmpeg/bin" "$ffmpeg/lib" "$sdl/bin" "$sdl/lib"; do
            if [ -e "$dir/$lib" ]; then
                cp -P "$dir/$lib" "$out/"
                # A versioned soname is a symlink to the real file; take both.
                real=$(readlink -f "$dir/$lib")
                cp "$real" "$out/" 2>/dev/null || true
                break
            fi
        done
    done < <(needed "$out/$binary")
done

if [ "$target" = linux ]; then
    # $ORIGIN, not $ORIGIN/lib: the archive is flat, and the libraries beside
    # the binary must win over anything installed.
    patchelf --set-rpath '$ORIGIN' "${binaries[@]/#/$out/}"
fi

# The compliance artifact. FFMPEG-SOURCE.txt records ffmpeg's upstream commit
# and full configure line, which together with COPYING.LGPLv2.1 is what LGPL
# 2.1 asks of us for a dynamically linked library. SDL3 is zlib and only wants
# its notice kept, which SDL-LICENSE.txt is. Do not publish an archive without
# these four files.
cp "$sdl/SDL-SOURCE.txt" "$sdl/SDL-LICENSE.txt" "$out/"
cp "$ffmpeg/FFMPEG-SOURCE.txt" "$ffmpeg/COPYING.LGPLv2.1" "$out/"
cp "$root/LICENSE" "$out/"

cd "$root/target/dist"
case "$target" in
windows) archive=daysengine-windows-x86_64.zip && zip -qr "$archive" "$(basename "$out")" ;;
linux) archive=daysengine-linux-x86_64.tar.gz && tar -czf "$archive" "$(basename "$out")" ;;
esac

echo
echo "wrote target/dist/$archive"
ls -l "$out"
