#!/usr/bin/env bash
#
# Builds the vendored ffmpeg in third_party/ffmpeg for a distribution build.
#
#   tools/build-ffmpeg.sh linux            -> target/ffmpeg/linux
#   tools/build-ffmpeg.sh windows          -> target/ffmpeg/windows  (MinGW-w64)
#   tools/build-ffmpeg.sh android-arm64    -> target/ffmpeg/android-arm64
#   tools/build-ffmpeg.sh android-x86_64   -> target/ffmpeg/android-x86_64
#
# This IS the developer build path as well as the release one. `just deps` calls
# it, .cargo/config.toml points cargo at what it produces, and `cargo build`
# links that -- because Debian 13's ffmpeg 7.1 predates the dynamic swscale API
# media::image uses and segfaults on it. docs/DEPENDENCIES.md has the reasoning
# and the evidence. The same libraries then go into a release archive, for the
# people who have no distribution to inherit from.
#
# ---------------------------------------------------------------------------
# Licensing
#
# The result is LGPL v2.1, shared, and nothing else:
#
#   --disable-gpl        no GPL-only code is compiled in at all, so the result
#                        cannot be relicensed upward by an accident of configure
#   --disable-nonfree    likewise for the non-redistributable parts
#   --disable-version3   keeps every component at LGPL v2.1 rather than v3
#   --enable-shared      DaysEngine links the libraries dynamically, which is
#     --disable-static   the condition LGPL 2.1 section 6 lets us distribute
#                        under without offering our own object files
#   --disable-autodetect configure links nothing it happened to find on the
#                        build machine, so the shipped libraries have no
#                        external dependencies and no licence we did not choose
#
# A release archive must carry, next to the libraries: this source tree's
# upstream URL and commit, ffmpeg's own COPYING.LGPLv2.1, and the configure
# line. `emit_source_note` below writes all three into FFMPEG-SOURCE.txt, and
# `just dist-linux` / `just dist-windows` copy it into the archive.
# ---------------------------------------------------------------------------
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
src=$root/third_party/ffmpeg
prefix=$root/target/ffmpeg/$target
build=$root/target/ffmpeg/build-$target

if [ ! -f "$src/configure" ]; then
    echo "third_party/ffmpeg is empty. Run: git submodule update --init --depth 1" >&2
    exit 1
fi

# ---------------------------------------------------------------------------
# What the engine actually decodes, and nothing else.
#
# The games ship exactly three media formats, and src/media says which code
# path each one takes:
#
#   .WMV  ASF container, WMV3 (VC-1 Main) video, no audio stream at all
#         (src/media/video.rs, and the note at the top of src/media/mod.rs)
#   .OGG  Ogg Vorbis, every sound in the game (src/media/audio.rs)
#   .PNG  decoded by the `png` crate, NOT by ffmpeg -- but then handed to
#         libswscale to be scaled, so that a still and a movie frame reach the
#         window through the same filter (src/media/image.rs)
#
# So: two demuxers, three decoders, no encoders, no muxers, no devices, no
# network. There is no ffmpeg png decoder here because nothing asks for one;
# swscale is what the stills need and swscale is not a codec.
#
# Filters are the deliberate exception and stay complete. The chain between the
# decoder and the scaler is a filtergraph string out of the player's own
# DaysEngine.ini (src/media/filter.rs), and its doc comment promises that
# "everything libavfilter knows is available". Trimming the filter list would
# turn that into a lie and break configs that already work against a system
# ffmpeg.
# ---------------------------------------------------------------------------
components=(
    --disable-encoders
    --disable-muxers
    --disable-devices
    --disable-bsfs
    --disable-decoders
    --enable-decoder=wmv3,vc1,vorbis
    --disable-demuxers
    --enable-demuxer=asf,ogg
    --disable-protocols
    --enable-protocol=file
    --disable-network
)

configure=(
    --prefix="$prefix"
    --disable-gpl
    --disable-nonfree
    --disable-version3
    --enable-shared
    --disable-static
    --disable-autodetect
    --disable-programs
    --disable-doc
    --disable-debug
    --enable-pic
    "${components[@]}"
)

if [ "$target" = windows ]; then
    configure+=(
        --arch=x86_64
        --target-os=mingw32
        --cross-prefix=x86_64-w64-mingw32-
    )
    # Threading, the DLL suffix and the import-library layout all come from
    # --target-os=mingw32: configure turns on w32threads by itself, puts the
    # DLLs in bin/ and the .dll.a import libraries in lib/.
fi

if [ "${target#android-}" != "$target" ]; then
    ndk=${ANDROID_NDK_HOME:-${ANDROID_NDK_ROOT:-}}
    toolchain=$ndk/toolchains/llvm/prebuilt/linux-x86_64
    if [ -z "$ndk" ] || [ ! -x "$toolchain/bin/clang" ]; then
        echo "set ANDROID_NDK_HOME to an Android NDK (r27 or newer)" >&2
        exit 1
    fi
    # Kept in step with tools/build-sdl.sh, which is where the number is
    # written down and explained.
    api=$(sed -n 's/^ANDROID_API=\([0-9]*\)$/\1/p' "$root/tools/build-sdl.sh")
    [ -n "$api" ] || {
        echo "cannot read ANDROID_API out of tools/build-sdl.sh" >&2
        exit 1
    }
    case "$target" in
    android-arm64)
        arch=aarch64
        triple=aarch64-linux-android
        ;;
    android-x86_64)
        arch=x86_64
        triple=x86_64-linux-android
        ;;
    esac
    configure+=(
        --enable-cross-compile
        --target-os=android
        --arch="$arch"
        --sysroot="$toolchain/sysroot"
        --cc="$toolchain/bin/$triple$api-clang"
        --cxx="$toolchain/bin/$triple$api-clang++"
        --ar="$toolchain/bin/llvm-ar"
        --nm="$toolchain/bin/llvm-nm"
        --ranlib="$toolchain/bin/llvm-ranlib"
        --strip="$toolchain/bin/llvm-strip"
    )
    # --target-os=android is what makes these loadable from an APK at all.
    # ffmpeg's configure gives that case `SHFLAGS='-shared -Wl,-soname,
    # $(SLIBNAME)'` with no install links, so the result is `libavcodec.so`
    # with `libavcodec.so` as its soname rather than the usual
    # `libavcodec.so.63`. Android's packager only extracts files named
    # `lib*.so` from an APK and its linker only looks them up by that name, so
    # a versioned soname is a library the app cannot load.
fi

emit_source_note() {
    local commit
    commit=$(git -C "$src" rev-parse HEAD)
    {
        echo "FFmpeg, as shipped with DaysEngine"
        echo
        echo "Upstream:  https://github.com/FFmpeg/FFmpeg"
        echo "Commit:    $commit"
        echo "Describe:  $(git -C "$src" describe --tags --always 2>/dev/null || echo unknown)"
        echo
        echo "Licence:   LGPL v2.1 or later. The full text is in COPYING.LGPLv2.1,"
        echo "           distributed beside this file."
        echo
        echo "Built with:"
        printf '  %s\n' "${configure[@]}"
        echo
        echo "These libraries are unmodified FFmpeg. DaysEngine links them"
        echo "dynamically and ships them unchanged; replacing them with your own"
        echo "build of the same soname is supported and is the point of shipping"
        echo "them shared rather than static."
    } >"$prefix/FFMPEG-SOURCE.txt"
    cp "$src/COPYING.LGPLv2.1" "$prefix/COPYING.LGPLv2.1"
}

mkdir -p "$build" "$prefix"
# ffmpeg's configure insists on being run from the build directory for an
# out-of-tree build, and refuses to run in-tree once one exists.
(cd "$build" && "$src/configure" "${configure[@]}")
make -C "$build" -j"$(nproc)"
make -C "$build" install
emit_source_note

echo
echo "ffmpeg ($target) installed to $prefix"
echo "pkg-config path: $prefix/lib/pkgconfig"
