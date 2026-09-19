#!/usr/bin/env bash
#
# Builds the vendored SDL3 in third_party/sdl for a distribution build.
#
#   tools/build-sdl.sh linux            -> target/sdl/linux
#   tools/build-sdl.sh windows          -> target/sdl/windows   (MinGW-w64 cross)
#   tools/build-sdl.sh android-arm64    -> target/sdl/android-arm64
#   tools/build-sdl.sh android-x86_64   -> target/sdl/android-x86_64
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
#
# **Android is the exception, and not for a licensing reason.** There SDL3 is
# built SHARED, because on Android SDL is half Java: SDLActivity, SDLSurface
# and SDLAudioManager are classes in the APK whose native methods are resolved
# against libSDL3.so by name, and System.loadLibrary("SDL3") is what binds
# them. A static SDL3 inside libdaysengine.so has no library for that call to
# find. zlib still attaches no condition either way, so what changes is one
# file in the APK and nothing about what may be distributed.
set -euo pipefail

# Builds what ships, so it builds in the image that decides what that links --
# on a bare host, in the devcontainer, or already inside it, this is the line
# that works out which and re-execs there when it has to. The Android NDK is
# in that image too, pinned beside the rest.
# shellcheck source=tools/in-container.sh
. "$(dirname "${BASH_SOURCE[0]}")/in-container.sh"

target=${1:-}
case "$target" in
linux | windows | android-arm64 | android-x86_64) ;;
*)
    echo "usage: $0 {linux|windows|android-arm64|android-x86_64}" >&2
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

# The oldest Android this runs on. Android 7.0, which is where the linker
# gained the namespace behaviour SDL's Java-to-native binding wants and where
# `posix_spawn` and a complete `<locale.h>` arrived for ffmpeg. Written down in
# one place: the gradle project reads it back out of this file rather than
# spelling a second number that could disagree.
ANDROID_API=24

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

if [ "${target#android-}" != "$target" ]; then
    ndk=${ANDROID_NDK_HOME:-${ANDROID_NDK_ROOT:-}}
    if [ -z "$ndk" ] || [ ! -f "$ndk/build/cmake/android.toolchain.cmake" ]; then
        echo "set ANDROID_NDK_HOME to an Android NDK (r27 or newer)" >&2
        exit 1
    fi
    case "$target" in
    android-arm64) abi=arm64-v8a ;;
    android-x86_64) abi=x86_64 ;;
    esac
    cmake_args+=(
        -DCMAKE_TOOLCHAIN_FILE="$ndk/build/cmake/android.toolchain.cmake"
        -DANDROID_ABI="$abi"
        -DANDROID_PLATFORM="android-$ANDROID_API"
        # Shared, for the Java-to-native binding described at the top of this
        # file. The static half is switched off so nothing links the wrong one
        # by accident.
        -DSDL_SHARED=ON
        -DSDL_STATIC=OFF
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
        echo "This is unmodified SDL3. On desktop it is linked into the"
        echo "daysengine binary; on Android it is libSDL3.so in the APK, for"
        echo "the Java-to-native reason in tools/build-sdl.sh. To build against"
        echo "your own copy instead, point tools/build-sdl.sh at it, or drop the"
        echo "static-sdl feature and link the SDL3 your system provides."
    } >"$prefix/SDL-SOURCE.txt"
    cp "$src/LICENSE.txt" "$prefix/SDL-LICENSE.txt"
}

# **A configure that cannot be reused is worse than no configure.** CMake
# caches the absolute path of the compiler it found, and for a cross build the
# absolute path of the toolchain file too. Those move: a bumped NDK pin in
# tools/dist/Dockerfile, or a tree built once on a host and once in the build
# image, leaves a cache naming a compiler that is not there -- and cmake's
# answer to that is to fail rather than to look again. Nothing in the cache is
# worth that, so a cache whose paths have gone is dropped and the configure is
# done afresh.
cache=$build/CMakeCache.txt
if [ -f "$cache" ]; then
    stale=0
    while IFS= read -r path; do
        [ -n "$path" ] && [ ! -e "$path" ] && stale=1
    done < <(sed -n 's/^CMAKE_\(TOOLCHAIN_FILE\|C_COMPILER\):[A-Z]*=\(.*\)$/\2/p' "$cache")
    if [ "$stale" = 1 ]; then
        echo "== $build was configured against a toolchain that has moved; reconfiguring"
        rm -rf "$build"
    fi
fi

cmake "${cmake_args[@]}"
cmake --build "$build" --parallel "$(nproc)"
cmake --install "$build"
emit_source_note

echo
echo "SDL3 ($target) installed to $prefix"
echo "pkg-config path: $prefix/lib/pkgconfig"
