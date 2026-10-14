#!/usr/bin/env bash
# What still needs doing in this container, and how to work in it. Read-only:
# it checks the current state and prints, nothing else, so it is safe to run
# any number of times.
#
# post-create.sh's own output only ever reaches the creation log (VS Code hides
# it, `devcontainer up` buries it in the build log, and a reused container never
# runs it again), so this is what actually puts it in front of you:
#   - claude.sh runs it before starting Claude,
#   - the image's .bashrc runs it in every interactive terminal (full the first
#     time in a container, only the to-do items after that),
#   - post-create.sh runs it last, for the creation log.
#
#   welcome.sh           to-do items + the command reference
#   welcome.sh --brief   to-do items only
#
# Exits 10 when there is something to do, so callers can wait for a keypress.
set -uo pipefail

cd "$(dirname "$0")/.."

brief=0
[ "${1:-}" = "--brief" ] && brief=1
todo=0

# **The player's install.** Each is mounted at its own path, so DAYS_GAME_DIR and
# DAYS_SHINYDAYS_DIR mean the same thing in here as on the host --
# docker-compose.yml sets both from the same .env it mounted them from. Packs/
# is what every title's archives live in, so it is what "there is an install
# here" means.
for title in "School Days HQ:${DAYS_GAME_DIR:-}" "Shiny Days:${DAYS_SHINYDAYS_DIR:-}"; do
    name=${title%%:*}
    dir=${title#*:}
    [ -n "$dir" ] || continue
    if [ ! -d "$dir/Packs" ]; then
        todo=1
        echo "  ! $name: no Packs/ at $dir -- check the path in .env."
    fi
done

if [ -z "${DAYS_GAME_DIR:-}" ] && [ -z "${DAYS_SHINYDAYS_DIR:-}" ]; then
    todo=1
    cat <<'MSG'
  ------------------------------------------------------------------
  No install is mounted.

  Copy .env.example to .env -- at the top of the repository, not in
  .devcontainer/ -- point DAYS_GAME_DIR at your own School Days HQ
  directory, and rebuild the container. Without it the engine builds
  and nothing can be verified: the archive key and the UI widget
  tables both come from your install.

  The same file is what .vscode/launch.json reads, so filling it in
  once is also what makes the "Play:" configurations work.
  ------------------------------------------------------------------
MSG
fi

# What every build links (post-create.sh builds them; this says when it could
# not). Without them `cargo build` fails on pkg-config.
if [ ! -f third_party/sdl/CMakeLists.txt ] || [ ! -f third_party/ffmpeg/configure ]; then
    todo=1
    echo "  ! third_party/ is empty -- run: git submodule update --init --depth 1"
elif [ ! -d target/sdl/linux/lib/pkgconfig ] || [ ! -d target/ffmpeg/linux/lib/pkgconfig ]; then
    todo=1
    echo "  ! The vendored SDL3/ffmpeg are not built -- run: ./tools/build-sdl.sh linux && ./tools/build-ffmpeg.sh linux"
fi

# The identity git commits with. Nothing is copied from the host: ~/.gitconfig
# and ~/.ssh are volumes this container owns (see docker-compose.yml), so the
# first container asks once and every rebuild after it stays quiet.
if ! git config --global --get user.email >/dev/null 2>&1; then
    todo=1
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

  Then print the public key and add it to your git server (GitHub:
  Settings > SSH and GPG keys > New SSH key, key type "Signing Key";
  Gitea/Forgejo/GitLab have the same under SSH keys). Add it again as
  an "Authentication Key" if you also push over SSH with it:

      cat /root/.ssh/id_ed25519.pub

  Pushing over HTTP(S) instead: the first push asks for your password
  or token and git keeps it in /root/.config/git/credentials (the same
  volume), so it is asked once, not on every push or rebuild.
  ------------------------------------------------------------------

MSG
elif [ -f /root/.ssh/id_ed25519.pub ] \
    && [ -z "$(git config --global --get user.signingkey 2>/dev/null)" ]; then
    # A key is in the volume but git is not signing with it yet.
    cat <<'MSG'

  A key exists in /root/.ssh but git does not sign with it. To use it:
      git config --global gpg.format ssh
      git config --global user.signingkey /root/.ssh/id_ed25519.pub
      git config --global commit.gpgsign true
  and add the output of this to your git server as a Signing Key:
      cat /root/.ssh/id_ed25519.pub

MSG
fi

if [ "$brief" = 0 ]; then
    cat <<'MSG'

Ready. `just` is not installed; these are the justfile's recipes, spelled out:
  cargo build --locked --workspace                               build
  cargo fmt --all -- --check                                     the commit gate:
  cargo clippy --locked --workspace --all-targets -- -D warnings   all three clean
  cargo test --locked --workspace
  cargo deny --locked check                                      supply-chain audit
  ./target/debug/daysengine <subcommand>                         headless verification
  ./tools/dist.sh linux                                          release archive (nested podman)
Ghidra: $HEADLESS "$GHIDRA_PROJ" SDHQ -process ... -noanalysis (CLAUDE.md)
This list again: bash .devcontainer/welcome.sh
MSG
fi

[ "$todo" = 1 ] && exit 10
exit 0
