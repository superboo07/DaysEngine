#!/usr/bin/env bash
#
# Builds the engine for Android and, unless told otherwise, packages it as an
# APK.
#
#   tools/build-android.sh              debug APK, every configured ABI
#   tools/build-android.sh --release    release APK (unsigned)
#   tools/build-android.sh --libs-only  just the .so files, no gradle
#
# What comes out is target/dist/daysengine-android-<commit>.apk, and what goes
# into it is no game data at all: the player picks their own install the first
# time the app is opened. See docs/ANDROID.md.
#
# ---------------------------------------------------------------------------
# Why the engine is a library here and not a program
#
# An Android app has no main. SDLActivity loads the shared objects listed by
# getLibraries(), then looks up SDL_main in the last of them and calls it on a
# thread of its own -- so what this builds is libdaysengine.so, a cdylib, whose
# SDL_main is src/android.rs.
#
# `cargo rustc --crate-type cdylib` rather than a crate-type in Cargo.toml,
# because that key is not per-target: adding "cdylib" there would link a second
# copy of the whole engine on every desktop `cargo build` for nothing.
#
# --no-default-features drops `static-sdl`. On Android SDL is half Java --
# SDLActivity and SDLSurface are classes in the APK whose native methods bind
# against libSDL3.so by name -- so it is the one platform where SDL3 ships as
# its own file. tools/build-sdl.sh has the longer version.
# ---------------------------------------------------------------------------
set -euo pipefail

# Builds what ships, so it builds in the image that decides what that is -- the
# NDK, the Android SDK's build tools and gradle are all pinned in
# tools/dist/Dockerfile, for the same reason the MinGW cross-compiler and the
# glibc floor are. On a bare host, in the devcontainer, or already inside it,
# this line works out which and re-execs there when it has to.
# shellcheck source=tools/in-container.sh
. "$(dirname "${BASH_SOURCE[0]}")/in-container.sh"

root=$(cd "$(dirname "$0")/.." && pwd)
mode=debug
libs_only=0
for arg in "$@"; do
    case "$arg" in
    --release) mode=release ;;
    --libs-only) libs_only=1 ;;
    *)
        echo "usage: $0 [--release] [--libs-only]" >&2
        exit 2
        ;;
    esac
done

ndk=${ANDROID_NDK_HOME:-${ANDROID_NDK_ROOT:-}}
toolchain=$ndk/toolchains/llvm/prebuilt/linux-x86_64
if [ -z "$ndk" ] || [ ! -x "$toolchain/bin/clang" ]; then
    echo "no Android NDK at ANDROID_NDK_HOME=${ndk:-<unset>}" >&2
    exit 1
fi

# Gradle's caches live under target/ rather than in $HOME, because the build
# container is --rm and would otherwise refetch the Android Gradle Plugin on
# every build. Same reasoning as the two library prefixes beside it.
export GRADLE_USER_HOME=$root/target/android/gradle-home
api=$(sed -n 's/^ANDROID_API=\([0-9]*\)$/\1/p' "$root/tools/build-sdl.sh")

# The ABIs in the APK, as
# "<cargo target>:<android abi>:<ndk triple>:<library prefix>". Adding one here
# is the whole change: the loop below builds it, the copy below that places it,
# and gradle packages whatever ends up in jniLibs.
abis=(
    "aarch64-linux-android:arm64-v8a:aarch64-linux-android:android-arm64"
    "x86_64-linux-android:x86_64:x86_64-linux-android:android-x86_64"
)

jni_libs=$root/target/android/jniLibs

for entry in "${abis[@]}"; do
    IFS=: read -r rust abi triple flavour <<<"$entry"
    sdl_prefix=$root/target/sdl/$flavour
    ff_prefix=$root/target/ffmpeg/$flavour
    [ -d "$sdl_prefix/lib/pkgconfig" ] || "$root/tools/build-sdl.sh" "$flavour"
    [ -d "$ff_prefix/lib/pkgconfig" ] || "$root/tools/build-ffmpeg.sh" "$flavour"

    echo "== $abi"
    # pkg-config answers for a target that is not the host only when told it
    # may, and LIBDIR *replaces* its search path, so a prefix that has not been
    # built fails the link with a clear error instead of resolving against the
    # machine's own libraries. FFMPEG_PKG_CONFIG_PATH is rusty_ffmpeg's own.
    export PKG_CONFIG_ALLOW_CROSS=1
    export PKG_CONFIG_LIBDIR="$sdl_prefix/lib/pkgconfig:$ff_prefix/lib/pkgconfig"
    export FFMPEG_PKG_CONFIG_PATH="$ff_prefix/lib/pkgconfig"
    # rusty_ffmpeg generates its bindings by parsing ffmpeg's headers with
    # clang, which otherwise reads glibc's.
    export BINDGEN_EXTRA_CLANG_ARGS="--target=$triple$api --sysroot=$toolchain/sysroot"
    # Named here rather than in .cargo/config.toml because the path is
    # wherever this machine's NDK is, and that file is checked in.
    upper=$(echo "$rust" | tr 'a-z-' 'A-Z_')
    export "CARGO_TARGET_${upper}_LINKER=$toolchain/bin/$triple$api-clang"

    profile=()
    if [ "$mode" = release ]; then
        profile=(--release)
    fi
    (cd "$root" && cargo rustc --locked --lib --no-default-features \
        --crate-type cdylib --target "$rust" "${profile[@]}")

    out=$jni_libs/$abi
    mkdir -p "$out"
    cp "$root/target/$rust/$mode/libdaysengine.so" "$out/"
    cp "$sdl_prefix/lib/libSDL3.so" "$out/"
    # Only the libraries the engine actually names. libavdevice is built --
    # rusty_ffmpeg always puts it on the link line -- and nothing in src/media
    # calls it, so the linker drops it and it never travels.
    for lib in avutil swresample swscale avcodec avformat avfilter; do
        cp "$ff_prefix/lib/lib$lib.so" "$out/"
    done
    "$toolchain/bin/llvm-strip" --strip-unneeded "$out"/*.so
done

# The build this APK is, named so two of them cannot be confused. `-dirty`
# means the working tree had changes git did not, so the source shipped inside
# is the only record of what actually went in.
version=$(git -C "$root" rev-parse --short HEAD 2>/dev/null || echo unknown)
if [ -n "$(git -C "$root" status --porcelain 2>/dev/null)" ]; then
    version="$version-dirty"
fi

# ---------------------------------------------------------------------------
# What an APK owes, and where it carries it
#
# The same four files a desktop archive ships, plus the engine's own source.
# FFMPEG-SOURCE.txt records ffmpeg's upstream commit and full configure line,
# which together with COPYING.LGPLv2.1 is what LGPL 2.1 asks of us for a
# dynamically linked library; SDL3 is zlib and only wants its notice kept.
#
# They go in the APK's assets rather than beside it, because there is no
# "beside it" on a phone -- an APK is one file, and a compliance artifact the
# player would have to be handed separately is one that does not travel. Do not
# publish an APK without these.
# ---------------------------------------------------------------------------
assets=$root/target/android/assets
rm -rf "$assets"
mkdir -p "$assets"
first=${abis[0]}
IFS=: read -r _ _ _ flavour <<<"$first"
cp "$root/target/sdl/$flavour/SDL-SOURCE.txt" "$root/target/sdl/$flavour/SDL-LICENSE.txt" "$assets/"
cp "$root/target/ffmpeg/$flavour/FFMPEG-SOURCE.txt" \
    "$root/target/ffmpeg/$flavour/COPYING.LGPLv2.1" "$assets/"
cp "$root/LICENSE" "$assets/"
# shellcheck source=tools/source-zip.sh
. "$root/tools/source-zip.sh"
days_source_zip "$root" "$version" "android" "$assets/SOURCE.zip"

if [ "$libs_only" = 1 ]; then
    echo
    echo "shared objects in $jni_libs"
    exit 0
fi

# ---------------------------------------------------------------------------
# The debug key, and why it is kept
#
# A debug APK has to be signed with the *same* key every time or `adb install`
# on top of the last one fails with "signatures do not match", and the only
# way out is uninstalling -- which throws away the folder grant with it.
#
# Android's own debug keystore lives in $HOME/.android, and $HOME inside the
# --rm build container is thrown away when the container exits, so the plugin
# generated a fresh one on every build and every APK was signed by a different
# key. Measured: two consecutive builds of the same commit signed by
# 73c433bb... and 120e8390.... So the keystore lives under target/ with every
# other build output, and android/app/build.gradle names it outright rather
# than leaving the plugin to find one.
#
# **These credentials are not a secret and are not meant to be.** They are
# exactly what every Android SDK generates for a debug key, they are in every
# copy of this file, and a debug key is not what a published build is signed
# with -- `--release` produces an unsigned APK precisely so that signing it is
# a deliberate act with a key this repository never sees.
#
# By default it is per-checkout, under target/ with every other build output:
# a fresh clone builds a new one, and installing over an APK from a different
# checkout needs one uninstall. DAYS_ANDROID_KEYSTORE moves it somewhere that
# outlives target/ -- the devcontainer sets it to a volume it owns, so
# `cargo clean` and a container rebuild both leave the key alone. See
# .devcontainer/docker-compose.yml and tools/in-container.sh, which carries
# the path into the build container.
# ---------------------------------------------------------------------------
keystore=${DAYS_ANDROID_KEYSTORE:-$root/target/android/debug.keystore}
mkdir -p "$(dirname "$keystore")"
if [ ! -f "$keystore" ]; then
    echo "== generating a debug key in $(dirname "$keystore")"
    keytool -genkeypair -noprompt \
        -keystore "$keystore" -storepass android -keypass android \
        -alias androiddebugkey -keyalg RSA -keysize 2048 -validity 10950 \
        -dname 'C=US, O=Android, CN=Android Debug'
fi

task=assemble$([ "$mode" = release ] && echo Release || echo Debug)
# --project-cache-dir is not optional. Gradle keeps per-project state in a
# `.gradle` directory beside the build file unless told otherwise, and that is
# generated output in the source tree -- which this repository does not have.
# Everything gradle writes belongs under target/ with every other build output.
gradle -p "$root/android" --no-daemon \
    --project-cache-dir "$root/target/android/gradle-cache" \
    -Pdaysengine.keystore="$keystore" "$task"

# **Found rather than spelled out.** A debug build leaves `app-debug.apk` and
# an unsigned release leaves `app-release-unsigned.apk` -- the name carries
# the signing state, so guessing it works for one mode and not the other.
# There is exactly one APK in that directory; take it.
built=$(find "$root/target/android/gradle/app/outputs/apk/$mode" \
    -maxdepth 1 -name '*.apk' -print -quit 2>/dev/null)
if [ -z "$built" ]; then
    echo "gradle produced no APK in target/android/gradle/app/outputs/apk/$mode" >&2
    exit 1
fi
mkdir -p "$root/target/dist"
apk=$root/target/dist/daysengine-android-$version.apk
cp "$built" "$apk"

echo
echo "$apk"
echo
if [ "$mode" = release ]; then
    echo "Unsigned. Sign it before installing:"
    echo "  apksigner sign --ks <your.keystore> $apk"
else
    echo "Signed with the debug key. Install it with:"
    echo "  adb install -r $apk"
fi
