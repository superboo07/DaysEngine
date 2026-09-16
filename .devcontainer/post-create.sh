#!/usr/bin/env bash
# Runs once, after the container is created.
set -euo pipefail

cd "$(dirname "$0")/.."

# The tree is a bind mount from the host, so git sees an owner that is not the
# user running it.
git config --global --add safe.directory "$PWD" || true

# third_party/ffmpeg and third_party/sdl are the pinned sources EVERY build
# links, a developer's as much as a release archive's (tools/build-sdl.sh,
# tools/build-ffmpeg.sh, and the note at the top of the justfile). Without them
# there is nothing to build against at all, so say so loudly -- but do not fail
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

# The identity git commits with. Nothing is copied from the host: ~/.gitconfig
# and ~/.ssh are volumes this container owns (see docker-compose.yml), so the
# first container asks once and every rebuild after it stays quiet.
if ! git config --global --get user.email >/dev/null 2>&1; then
    cat <<'MSG'

  ------------------------------------------------------------------
  git has no identity in this container yet. It is not taken from your
  host config on purpose -- set it once and the volume keeps it:

      git config --global user.name  "you"
      git config --global user.email "you@example.com"

  For signed commits, put a key in /root/.ssh (also a volume, also
  yours) and point git at it:

      ssh-keygen -t ed25519 -f /root/.ssh/id_ed25519
      git config --global gpg.format ssh
      git config --global user.signingkey /root/.ssh/id_ed25519.pub
      git config --global commit.gpgsign true
  ------------------------------------------------------------------

MSG
fi

echo
if [ -e "/game/schooldays/Packs" ] || [ -e "/game/schooldays/SCHOOLDAYS HQ.exe" ]; then
    echo "School Days install mounted at /game/schooldays."
else
    cat <<'MSG'
  ------------------------------------------------------------------
  /game/schooldays holds no install.

  Copy .devcontainer/.env.example to .devcontainer/.env, point
  DAYS_GAME_DIR at your own School Days HQ directory, and rebuild the
  container. Without it the engine builds and nothing can be verified:
  the archive key and the UI widget tables both come from your install.
  ------------------------------------------------------------------
MSG
fi

echo
echo "Ready. Builds run in a container either way -- see tools/container.sh."
echo "  just check          fmt, clippy -D warnings, tests"
echo "  just dist-linux     release archive, built in a clean image"
