# All recipes pass --locked: a build must never silently rewrite Cargo.lock.
# If a recipe fails with "the lock file needs to be updated", that is the point.
# Update it deliberately with `just add` / `just update`, then review the diff.

# ---------------------------------------------------------------------------
# What a build links
#
# The pinned SDL3 and ffmpeg in third_party/, built by tools/build-sdl.sh and
# tools/build-ffmpeg.sh -- **not** the distribution's packages, and the same
# libraries for a developer build as for a release archive.
#
# That reverses an earlier decision, and why is worth keeping: inheriting a
# distribution's CVE patches on two large C media parsers is a real benefit, but
# it was chosen while this project was developed on a host whose ffmpeg happened
# to be current. In general it is not. Debian 13 ships ffmpeg 7.1, and
# `media::image` calls `sws_scale_frame` on an allocated-but-uninitialised
# context -- the dynamic swscale API, which ffmpeg's own header documents as
# usable "without setting up any frame properties or calling sws_init_context()"
# and which first ships in **n8.0** (`git tag --contains 2a091d4f2e`). On 7.1
# that call reaches `av_frame_ref` through `sws_frame_start` with frames the
# context was never configured for, and segfaults. A build that depends on the
# host being new enough is a build that breaks on whichever host is not.
#
# **Both builds happen in the same image**, tools/dist/, which is the only place
# that carries SDL3's and ffmpeg's own build dependencies -- and which is what
# decides an archive's glibc floor. tools/in-container.sh is that argument in
# full; build-sdl.sh and build-ffmpeg.sh re-exec there on their own. So there is
# one prefix, one set of libraries, and nothing for a developer build to leave
# behind that a release would then ship.
#
# Where cargo is *told* all this is .cargo/config.toml, not here, because
# `cargo build` is what gets typed far more often than `just`.
# ---------------------------------------------------------------------------
sdl := justfile_directory() / "target/sdl/linux"
ffmpeg := justfile_directory() / "target/ffmpeg/linux"

default: check

# Build the vendored SDL3 and ffmpeg. Slow, and once per checkout: every recipe
# below needs them, and this does nothing when they are already there. Delete
# the prefixes to force a rebuild.
deps:
    #!/usr/bin/env bash
    set -euo pipefail
    [ -d "{{ sdl }}/lib/pkgconfig" ] || ./tools/build-sdl.sh linux
    [ -d "{{ ffmpeg }}/lib/pkgconfig" ] || ./tools/build-ffmpeg.sh linux

build: deps
    cargo build --locked --workspace

release: deps
    cargo build --locked --release --workspace

check: deps
    cargo fmt --all -- --check
    cargo clippy --locked --workspace --all-targets -- -D warnings
    cargo test --locked --workspace

# Supply-chain gate: advisories, licenses, banned crates, source registries.
audit:
    cargo deny --locked check

# Deliberate dependency change. Review the Cargo.lock diff before committing.
update:
    cargo update --locked --dry-run || true
    @echo "Review the above, then run: cargo update -p <crate> --precise <version>"

# Prove a fresh clone reproduces: no lockfile drift after a full build.
verify-lock: build
    git diff --exit-code Cargo.lock

# ---------------------------------------------------------------------------
# Distribution builds
#
# A release archive links exactly what a developer build links -- see the note
# at the top of this file -- so what is left below is only the packaging, and
# the Windows cross-build, which needs a prefix of its own because it is a
# different target rather than a different promise.
#
# SDL3 is zlib and is linked in statically, so no libSDL3 ships. ffmpeg is LGPL
# v2.1 and stays shared, because static linking it would oblige us to let the
# player relink -- see the licensing note at the top of tools/build-ffmpeg.sh.
# The archive carries its licence text, its upstream commit and its configure
# line beside the libraries, and is otherwise flat: a drag-and-drop.
# ---------------------------------------------------------------------------

# The Windows half of `deps`, cross-compiled with MinGW-w64.
windows-deps:
    ./tools/build-sdl.sh windows
    ./tools/build-ffmpeg.sh windows

# Cross-compiled Windows binaries, without packaging them.
#
# PKG_CONFIG_ALLOW_CROSS lets pkg-config answer for a target that is not the
# host; without it the pkg-config crate refuses on principle. BINDGEN_EXTRA_
# CLANG_ARGS points bindgen at the MinGW headers, because it parses ffmpeg's
# headers with clang and clang otherwise reads glibc's.
windows-release:
    #!/usr/bin/env bash
    set -euo pipefail
    root=$(pwd)
    test -d "$root/target/ffmpeg/windows/lib/pkgconfig" || { echo "run 'just windows-deps' first"; exit 1; }
    export PKG_CONFIG_ALLOW_CROSS=1
    export PKG_CONFIG_LIBDIR="$root/target/sdl/windows/lib/pkgconfig:$root/target/ffmpeg/windows/lib/pkgconfig"
    export FFMPEG_PKG_CONFIG_PATH="$root/target/ffmpeg/windows/lib/pkgconfig"
    export BINDGEN_EXTRA_CLANG_ARGS="--target=x86_64-w64-mingw32 -I/usr/x86_64-w64-mingw32/include"
    cargo build --locked --release --workspace --target x86_64-pc-windows-gnu

# Release archives. tools/dist.sh is where the layout actually lives, so that
# .vscode/tasks.json can call the same thing and `just` stays optional.
dist-linux: deps
    ./tools/dist.sh linux

dist-windows: windows-deps
    ./tools/dist.sh windows

# ---------------------------------------------------------------------------
# Android
#
# The engine is a shared object here rather than a program -- an Android app
# has no main, SDLActivity loads libdaysengine.so and calls its SDL_main -- and
# the app around it is android/, a gradle project whose only screen asks which
# folder the player's install is in. Everything else on screen is the game's
# own art, as everywhere else.
#
# Built in the same image as every other release: the NDK, the Android SDK's
# build tools and gradle are pinned in tools/dist/Dockerfile beside the MinGW
# cross-compiler. See docs/ANDROID.md.
# ---------------------------------------------------------------------------

# SDL3 and ffmpeg for both packaged ABIs. Slow, and once per checkout.
android-deps:
    ./tools/build-sdl.sh android-arm64
    ./tools/build-sdl.sh android-x86_64
    ./tools/build-ffmpeg.sh android-arm64
    ./tools/build-ffmpeg.sh android-x86_64

# A debug-signed APK, ready for `adb install`.
android:
    ./tools/build-android.sh

# An unsigned release APK in target/dist/, named after the commit.
dist-android:
    ./tools/build-android.sh --release
