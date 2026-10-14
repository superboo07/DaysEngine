#!/usr/bin/env bash
# Runs once, after the container is created. Idempotent and never destructive:
# nothing that already exists is replaced.
set -euo pipefail

cd "$(dirname "$0")/.."

# The tree is a bind mount from the host, so git sees an owner that is not the
# user running it.
git config --global --add safe.directory "$PWD" || true

bash .devcontainer/enable-plugins.sh

# third_party/ffmpeg and third_party/sdl are the pinned sources EVERY build
# links, a developer's as much as a release archive's (tools/build-sdl.sh,
# tools/build-ffmpeg.sh, and the note at the top of the justfile). Without them
# there is nothing to build against at all, so say so -- but do not fail
# the container, which is still worth having for everything that is not a build.
git submodule update --init --depth 1 || {
    echo "submodule fetch failed; nothing will build until it succeeds." >&2
}

# The libraries every build links, built once, in the image that owns SDL3's and
# ffmpeg's build dependencies (tools/in-container.sh re-execs there by itself).
# Slow the first time and a no-op afterwards -- the prefixes are under target/,
# which is in the bind mount, so a container rebuild does not repeat it.
#
# Not fatal. A container that came up without these is still worth having for
# everything that is not a build, and the failure `cargo build` then gives names
# the missing prefix outright.
if [ ! -d target/sdl/linux/lib/pkgconfig ] || [ ! -d target/ffmpeg/linux/lib/pkgconfig ]; then
    echo "Building the vendored SDL3 and ffmpeg. This is the slow part, and it happens once."
    ./tools/build-sdl.sh linux && ./tools/build-ffmpeg.sh linux || {
        echo "vendored library build failed; run 'just deps' once it is fixed." >&2
    }
fi

# Install the compiler rust-toolchain.toml pins, and the two components the
# commit gate needs. The image ships rustup with no toolchain so the pin stays
# in the one file that owns it; this is the first run that acts on it, and it
# lands on a volume, so a rebuild does not repeat the download.
rustup show active-toolchain || rustup toolchain install
rustup component add rustfmt clippy
# The Windows archive is cross-compiled from Linux (docs/WINDOWS.md). The
# toolchain for it costs a few megabytes and saves a confusing first failure.
rustup target add x86_64-pc-windows-gnu

# Warm the registry cache so the first build is a build and not a download.
cargo fetch --locked || true

# Everything to tell you -- a missing install, git identity, signing key, the
# commands -- is in welcome.sh, which checks the state as it is *now* and runs
# again in every terminal and before claude.sh starts Claude. Output from this
# script only reaches the creation log, which nobody sees twice.
bash .devcontainer/welcome.sh || true
