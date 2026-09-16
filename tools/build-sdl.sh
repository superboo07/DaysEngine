#!/usr/bin/env bash
#
# Builds the vendored SDL3 in third_party/sdl for a distribution build.
#
#   tools/build-sdl.sh linux     -> target/sdl/linux
#   tools/build-sdl.sh windows   -> target/sdl/windows   (MinGW-w64 cross)
#
# Like tools/build-ffmpeg.sh, this is the release path and not the developer
# one: `cargo build` and `just check` link whatever libSDL3 the machine has.
# What this produces is the copy that goes in a release archive.
#
# SDL3 is zlib-licensed, so unlike ffmpeg there is nothing owed beyond keeping
# its copyright notice with the binary -- `emit_source_note` copies LICENSE.txt
# next to the library and records the commit it was built from, which is also
# what makes a shipped build reproducible.
#
# It is built STATIC, and that is the whole reason this differs from ffmpeg.
# zlib attaches no condition to linking, so SDL3 can go inside the executable
# and one more file leaves the archive. ffmpeg cannot follow: LGPL v2.1 permits
# static linking only against an obligation to let the player relink, and
# shipping it shared is how section 6 is satisfied without one.
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
src=$root/third_party/sdl
prefix=$root/target/sdl/$target
build=$root/target/sdl/build-$target

if [ ! -f "$src/CMakeLists.txt" ]; then
    echo "third_party/sdl is empty. Run: git submodule update --init --depth 1" >&2
    exit 1
fi

cmake_args=(
    -S "$src"
    -B "$build"
    -DCMAKE_BUILD_TYPE=Release
    -DCMAKE_INSTALL_PREFIX="$prefix"
    -DSDL_SHARED=OFF
    -DSDL_STATIC=ON
    # Static SDL3 still has to be position-independent: it is linked into an
    # executable the toolchain builds as PIE.
    -DCMAKE_POSITION_INDEPENDENT_CODE=ON
    -DSDL_TESTS=OFF
    -DSDL_EXAMPLES=OFF
    -DSDL_INSTALL_TESTS=OFF
)

if [ "$target" = windows ]; then
    cmake_args+=(
        -DCMAKE_SYSTEM_NAME=Windows
        -DCMAKE_SYSTEM_PROCESSOR=x86_64
        -DCMAKE_C_COMPILER=x86_64-w64-mingw32-gcc
        -DCMAKE_CXX_COMPILER=x86_64-w64-mingw32-g++
        -DCMAKE_RC_COMPILER=x86_64-w64-mingw32-windres
        -DCMAKE_FIND_ROOT_PATH=/usr/x86_64-w64-mingw32
        -DCMAKE_FIND_ROOT_PATH_MODE_PROGRAM=NEVER
        -DCMAKE_FIND_ROOT_PATH_MODE_LIBRARY=ONLY
        -DCMAKE_FIND_ROOT_PATH_MODE_INCLUDE=ONLY
    )
fi

emit_source_note() {
    {
        echo "SDL3, as shipped with DaysEngine"
        echo
        echo "Upstream:  https://github.com/libsdl-org/SDL"
        echo "Commit:    $(git -C "$src" rev-parse HEAD)"
        echo "Describe:  $(git -C "$src" describe --tags --always 2>/dev/null || echo unknown)"
        echo
        echo "Licence:   zlib. The full text is in SDL-LICENSE.txt, distributed"
        echo "           beside this file."
        echo
        echo "This is unmodified SDL3, statically linked into daysengine and"
        echo "days. To build against your own copy instead, point"
        echo "tools/build-sdl.sh at it, or drop the static-sdl feature and link"
        echo "the SDL3 your system provides."
    } >"$prefix/SDL-SOURCE.txt"
    cp "$src/LICENSE.txt" "$prefix/SDL-LICENSE.txt"
}

cmake "${cmake_args[@]}"
cmake --build "$build" --parallel "$(nproc)"
cmake --install "$build"
emit_source_note

echo
echo "SDL3 ($target) installed to $prefix"
echo "pkg-config path: $prefix/lib/pkgconfig"
