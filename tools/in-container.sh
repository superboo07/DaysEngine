# Sourced by the three scripts that build what ships -- tools/build-sdl.sh,
# tools/build-ffmpeg.sh and tools/dist.sh. It answers one question, and acts on
# it before the script it was sourced from does anything:
#
#   "am I already in the image a release is built in?"
#
#   yes  -> return, and the script runs normally.
#   no   -> build that image if needed, re-exec this same script inside it, and
#           exit with whatever it returned.
#
# So `./tools/dist.sh linux` is one command that means the same thing in three
# places: on a bare host, in the devcontainer, and inside the build image
# itself. The tasks in .vscode/tasks.json and the recipes in the justfile call
# the scripts exactly as they always did and end up in the right container
# without knowing it.
#
# **Why a container at all, rather than building where you are.** Two reasons,
# and they are not the same reason. An archive's glibc floor is a property of
# the image it was linked in, and the devcontainer -- Mesa, the Vulkan loader, a
# debugger, podman, Claude Code -- is not a decision about that, it is an
# accident. And SDL3 and ffmpeg have build dependencies of their own, dozens of
# -dev packages between them, which live in exactly one place rather than being
# installed twice. tools/dist/Dockerfile is both: a toolchain, the MinGW
# cross-compiler, the assembler, those build dependencies, and nothing else.
#
# A developer build is NOT untouched by this, and has not been since the
# libraries stopped being the host's: `just deps` builds the same pinned SDL3 and
# ffmpeg through these same scripts, into the same prefixes, in this same image.
# What differs between a developer build and an archive is the packaging, not
# what gets linked. See docs/DEPENDENCIES.md.

# Set by tools/dist/Dockerfile. Inside the build image there is nothing to do.
if [ -n "${DAYS_IN_CONTAINER:-}" ]; then
    return 0
fi

# The script that sourced this, and the arguments it was given. A sourced file
# inherits the caller's positional parameters, so "$@" here is the caller's.
days_script=$(cd "$(dirname "${BASH_SOURCE[1]}")" && pwd)/$(basename "${BASH_SOURCE[1]}")
days_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
days_rel=${days_script#"$days_root"/}

days_die() {
    echo "in-container: $*" >&2
    exit 1
}

# **Podman or docker, whichever is installed.** Podman first: it needs no
# daemon, so on a machine with both it is the one that works without the caller
# being in a privileged group -- and inside the devcontainer it is the only one
# there at all.
days_engine=${DAYS_CONTAINER_ENGINE:-}
if [ -z "$days_engine" ]; then
    for days_candidate in podman docker; do
        if command -v "$days_candidate" >/dev/null 2>&1; then
            days_engine=$days_candidate
            break
        fi
    done
fi
if [ -z "$days_engine" ]; then
    cat >&2 <<'EOF'
error: no container runtime found.

  What a release archive links is a property of the image it is built in, not of
  the machine that started the build -- see the comment at the top of
  tools/in-container.sh. Install podman or docker, or open this repository in
  the devcontainer (.devcontainer/), which carries podman.

  To name a runtime yourself:  DAYS_CONTAINER_ENGINE=...
EOF
    exit 1
fi

# **The two flags podman needs when it is nested inside the devcontainer.** Both
# were measured in this image against rootless docker as the outer runtime, and
# the third thing that makes them work is on the other side, in
# .devcontainer/docker-compose.yml.
#
#   --network=host      podman's own bridge wants CAP_NET_ADMIN, which the
#                       devcontainer has not got.
#   --cgroups=disabled  the outer container's /sys/fs/cgroup is read-only, so
#                       creating the container's cgroup fails with "opening
#                       file `cgroup.subtree_control` for writing". Podman then
#                       requires a private PID namespace, which is why
#                       --pid=host is NOT passed: the two are mutually
#                       exclusive, and with it the run fails anyway on
#                       "mount `proc` to `proc`: Operation not permitted".
#
# That last failure is what `systempaths=unconfined` on the devcontainer fixes:
# a new PID namespace needs a fresh /proc, and the kernel refuses to mount one
# while the outer /proc carries the runtime's masked overmounts.
#
# Nothing here applies to podman on a real host, or to docker.
days_engine_flags=()
if [ "$days_engine" = podman ] && { [ -f /.dockerenv ] || [ -n "${DEVCONTAINER:-}" ]; }; then
    days_engine_flags=(--network=host --cgroups=disabled)
fi

days_image=${DAYS_BUILD_IMAGE:-daysengine-dist-builder:latest}

# The compiler is the repository's pin, read from the file that owns it rather
# than written down a second time in the Dockerfile.
days_channel=$(sed -n 's/^channel *= *"\(.*\)"/\1/p' "$days_root/rust-toolchain.toml")
[ -n "$days_channel" ] || days_die "no channel in rust-toolchain.toml"

# **The image is stale when it was not built from this Dockerfile and this pin.**
# "Does an image by that name exist" is the wrong question: a Dockerfile that
# gained a package still matches by name, and the build then fails deep inside
# for a reason that has nothing to do with the tree. The label is a hash of both
# inputs rather than an mtime, because a fresh clone gives every file the
# checkout's timestamp and would rebuild the image for nothing.
days_label=$({
    sha256sum "$days_root/tools/dist/Dockerfile"
    echo "$days_channel"
} | sha256sum | cut -d' ' -f1)

days_have_image() {
    "$days_engine" image inspect "$days_image" >/dev/null 2>&1 || return 1
    [ "$("$days_engine" image inspect -f '{{index .Config.Labels "daysengine.dockerfile"}}' \
        "$days_image" 2>/dev/null)" = "$days_label" ]
}

if ! days_have_image; then
    echo "== building $days_image with $days_engine (rust $days_channel)" >&2
    # A build takes the network flag; --cgroups is a run-time option only.
    "$days_engine" build ${days_engine_flags[0]:+"${days_engine_flags[0]}"} \
        --label "daysengine.dockerfile=$days_label" \
        --build-arg "RUST_CHANNEL=$days_channel" \
        -t "$days_image" "$days_root/tools/dist" >&2
fi

if [ ! -f "$days_root/third_party/sdl/CMakeLists.txt" ] ||
    [ ! -f "$days_root/third_party/ffmpeg/configure" ]; then
    days_die "third_party is empty. Run: git submodule update --init --depth 1"
fi

# **The tree is mounted at its own path, not at /src.** Everything lands under
# target/ in it, exactly as a build outside the container leaves it -- and that
# is the point: what these builds install into target/sdl and target/ffmpeg
# includes pkg-config and CMake files, and those record the prefix they were
# configured with as an absolute path. Mounted at /src, ffmpeg's libavcodec.pc
# says `prefix=/src/target/ffmpeg/linux` and SDL3's says the same, so the
# libraries are unusable anywhere but inside this container -- which is exactly
# what a developer build then needs them to be. Mounting at the real path costs
# nothing and makes the artifacts mean the same thing on both sides.
#
# The crates go on a volume because the container is --rm, and without one every
# build refetches the whole graph.
days_run=(--rm)
[ -t 0 ] && [ -t 1 ] && days_run+=(-it)
days_run+=(
    -v "$days_root:$days_root"
    -w "$days_root"
    -e CARGO_TERM_COLOR=always
    # tools/dist.sh names the archive after the commit, so git runs in there --
    # and git refuses a tree owned by another uid unless the mount is declared
    # safe.
    -e GIT_CONFIG_COUNT=1
    -e GIT_CONFIG_KEY_0=safe.directory
    -e GIT_CONFIG_VALUE_0="$days_root"
)

# **Who the build runs as, and why the answer is not always the same.** What
# matters is only that the archive comes out owned by the person who asked for
# it rather than by root.
#
#   rootless podman, rootless docker  container root already *is* the caller,
#                                     so the default is right and --user would
#                                     map into the subuid range and fail to
#                                     write target/ at all.
#   a root daemon                     --user is what keeps root's name off the
#                                     caller's working tree. That uid cannot
#                                     write the image's /usr/local/cargo either,
#                                     so HOME and CARGO_HOME move under target/,
#                                     which the caller does own -- and the
#                                     registry volume is skipped for the same
#                                     reason.
days_rootless=0
case $("$days_engine" info -f '{{.SecurityOptions}}' 2>/dev/null) in
*rootless*) days_rootless=1 ;;
esac
[ "$days_engine" = podman ] && [ "$(id -u)" -ne 0 ] && days_rootless=1

if [ "$days_rootless" = 0 ] && [ "$(id -u)" -ne 0 ]; then
    mkdir -p "$days_root/target/dist/.home" "$days_root/target/dist/.cargo"
    days_run+=(
        --user "$(id -u):$(id -g)"
        -e HOME="$days_root/target/dist/.home"
        -e CARGO_HOME="$days_root/target/dist/.cargo"
    )
else
    days_run+=(-v "${DAYS_CARGO_VOLUME:-daysengine-dist-cargo}:/usr/local/cargo/registry")
fi

exec "$days_engine" run "${days_engine_flags[@]}" "${days_run[@]}" \
    "$days_image" "$days_rel" "$@"
