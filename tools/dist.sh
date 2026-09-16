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

# Builds what ships, so it builds in the image that decides what that links --
# on a bare host, in the devcontainer, or already inside it, this is the line
# that works out which and re-execs there when it has to.
# shellcheck source=tools/in-container.sh
. "$(dirname "${BASH_SOURCE[0]}")/in-container.sh"

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

# The build this archive is, named so two of them cannot be confused. `-dirty`
# means the working tree had changes git did not have, so the SOURCE.zip beside
# the binary is the only record of what actually went in.
version=$(git -C "$root" rev-parse --short HEAD)
if [ -n "$(git -C "$root" status --porcelain)" ]; then
    version="$version-dirty"
fi

out=$root/target/dist/daysengine-$target-x86_64-$version
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
    # $ORIGIN, not $ORIGIN/lib: the archive is flat, and the libraries beside the
    # binary must win over anything installed. Single quotes keep $ORIGIN for the
    # dynamic loader to expand at run time rather than the shell at build time.
    #
    # Set at link time rather than written into the finished ELF afterwards: the
    # linker is already deciding what this binary says about where its libraries
    # are, and a second tool rewriting that decision is one more thing to install
    # and one more place the answer lives.
    RUSTFLAGS='-C link-arg=-Wl,-rpath,$ORIGIN' \
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

# The compliance artifact. FFMPEG-SOURCE.txt records ffmpeg's upstream commit
# and full configure line, which together with COPYING.LGPLv2.1 is what LGPL
# 2.1 asks of us for a dynamically linked library. SDL3 is zlib and only wants
# its notice kept, which SDL-LICENSE.txt is. Do not publish an archive without
# these four files.
cp "$sdl/SDL-SOURCE.txt" "$sdl/SDL-LICENSE.txt" "$out/"
cp "$ffmpeg/FFMPEG-SOURCE.txt" "$ffmpeg/COPYING.LGPLv2.1" "$out/"
cp "$root/LICENSE" "$out/"

# The engine's own source, as it was when this binary was built.
#
# The commit in the archive's name says which source that is, but only while the
# commit is reachable -- and says nothing at all for a `-dirty` build, which is
# exactly the build whose source is hardest to reconstruct later. So the source
# travels with the binary rather than being pointed at.
#
# Tracked files plus untracked ones git is not ignoring, taken from the working
# tree rather than from HEAD: a dirty build then ships what actually built it.
# `target/` is excluded because .gitignore excludes it.
#
# The third_party submodules are gitlinks, so their contents are not in here.
# That is deliberate and not a compliance gap: FFMPEG-SOURCE.txt beside this
# file names ffmpeg's exact upstream commit, which is what LGPL 2.1 asks for,
# and .gitmodules in the zip names the repository it came from.
emit_source_zip() {
    local list
    list=$(mktemp)
    git -C "$root" ls-files --cached --others --exclude-standard |
        grep -vx -e third_party/ffmpeg -e third_party/sdl >"$list"

    {
        echo "DaysEngine source, as built"
        echo
        echo "Commit:  $version"
        echo "Built:   $(date -u '+%Y-%m-%d %H:%M:%S UTC') for $target"
        echo
        if [ "${version%-dirty}" != "$version" ]; then
            echo "This build was made from a WORKING TREE, not from a commit. What"
            echo "is in this zip is what went into the binary; the commit named"
            echo "above is only where those changes started."
            echo
        fi
        echo "The vendored SDL3 and ffmpeg sources are not in here -- they are"
        echo "git submodules, and FFMPEG-SOURCE.txt and SDL-SOURCE.txt beside"
        echo "this zip name the exact upstream commit of each. Restore them with"
        echo "'git submodule update --init --depth 1'."
        echo
        echo "Build it with 'cargo build --locked --release'; see docs/WINDOWS.md"
        echo "for a release archive."
    } >"$root/target/dist/SOURCE.txt"

    (cd "$root" && zip -q -X "$out/SOURCE.zip" -@ <"$list")
    (cd "$root/target/dist" && zip -qj "$out/SOURCE.zip" SOURCE.txt)
    rm -f "$list" "$root/target/dist/SOURCE.txt"
}
emit_source_zip

cd "$root/target/dist"
case "$target" in
windows) archive=daysengine-windows-x86_64-$version.zip && zip -qry "$archive" "$(basename "$out")" ;;
linux) archive=daysengine-linux-x86_64-$version.tar.gz && tar -czf "$archive" "$(basename "$out")" ;;
esac

echo
echo "wrote target/dist/$archive"
ls -l "$out"
