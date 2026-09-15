# All recipes pass --locked: a build must never silently rewrite Cargo.lock.
# If a recipe fails with "the lock file needs to be updated", that is the point.
# Update it deliberately with `just add` / `just update`, then review the diff.

default: check

build:
    cargo build --locked --workspace

release:
    cargo build --locked --release --workspace

check:
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
# A developer build links the system SDL3 and ffmpeg, which is what
# docs/DEPENDENCIES.md asks for: a distribution ships its own CVE patches on two
# large C media parsers and we want to inherit them rather than freeze a copy.
#
# A *release archive* cannot rely on that. The player who unpacks it may have no
# libSDL3 at all, and on Windows there is no distribution to inherit from in the
# first place. So the archives carry their own, built from the pinned submodules
# in third_party/ by tools/build-sdl.sh and tools/build-ffmpeg.sh.
#
# SDL3 is zlib and is linked in statically, so no libSDL3 ships. ffmpeg is LGPL
# v2.1 and stays shared, because static linking it would oblige us to let the
# player relink -- see the licensing note at the top of tools/build-ffmpeg.sh.
# The archive carries its licence text, its upstream commit and its configure
# line beside the libraries, and is otherwise flat: a drag-and-drop.
# ---------------------------------------------------------------------------

# Build the vendored SDL3 and ffmpeg for the host. Slow; once per checkout.
deps-linux:
    ./tools/build-sdl.sh linux
    ./tools/build-ffmpeg.sh linux

# The same, cross-compiled for Windows with MinGW-w64.
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
dist-linux: deps-linux
    ./tools/dist.sh linux

dist-windows: windows-deps
    ./tools/dist.sh windows
