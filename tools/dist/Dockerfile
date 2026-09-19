# The image a release archive is built in.
#
# **This exists because the devcontainer is not what should be shipped from.**
# That image carries Mesa, the Vulkan loader, a debugger, podman and Claude
# Code, and it links the system SDL3 and ffmpeg Debian packages; an archive
# built there is linked against whatever a developer happened to have installed.
# What goes in an archive is built here instead: a base with a toolchain, the
# MinGW cross-compiler, and nothing else. The SDL3 and ffmpeg it links are the
# pinned sources in third_party/, built by tools/build-sdl.sh and
# tools/build-ffmpeg.sh inside this container.
#
# The base also sets the glibc floor of a Linux archive -- a binary built here
# needs a glibc at least as new as trixie's. Moving that floor is a deliberate
# change to this line.
#
# Built and run by tools/in-container.sh, which the three scripts that build
# what ships source at their top; it is also what supplies RUST_CHANNEL.
FROM debian:trixie-slim

# **apt drops to uid 65534 to download, and cannot when this image is built by
# podman inside the devcontainer**: the nested user namespace has
# /proc/self/setgroups set to deny, so apt's setgroups(2) is EINVAL and every
# method process dies with "Failed to setgroups". Telling apt to stay root is
# the fix, and it costs nothing here -- this is a throwaway build image whose
# only job is to compile this repository, not a system anyone logs into.
RUN printf 'APT::Sandbox::User "root";\n' > /etc/apt/apt.conf.d/99-no-sandbox

RUN apt-get update && export DEBIAN_FRONTEND=noninteractive \
	&& apt-get install -y --no-install-recommends \
		build-essential \
		pkg-config \
		# bindgen's parser: rusty_ffmpeg generates its bindings from whichever
		# ffmpeg headers it is pointed at, and on the cross-build it has to be
		# told which sysroot to read (BINDGEN_EXTRA_CLANG_ARGS in tools/dist.sh).
		clang \
		libclang-dev \
		# SDL3's build, and ffmpeg's assembler. Without nasm, configure refuses
		# to build the hand-written x86 assembly and WMV3 decode falls back to
		# C -- which a 24 fps movie at 800x452 cannot afford.
		cmake \
		ninja-build \
		nasm \
		# tools/dist.sh reads the binary's own DT_NEEDED (readelf) or import
		# table (objdump) to decide which libraries travel with it.
		binutils \
		binutils-mingw-w64 \
		mingw-w64 \
		git \
		ca-certificates \
		curl \
		python3 \
		file \
		xz-utils \
		zip \
	&& apt-get clean && rm -rf /var/lib/apt/lists/*

# **What the shipped SDL3 can talk to is decided here.** These are headers, not
# libraries to run against: SDL3 dlopen()s Wayland, X11, ALSA and PulseAudio at
# runtime, so the player needs none of these packages -- but SDL builds a backend
# only where it found the headers, and its cmake refuses to configure at all
# when it finds neither X11 nor Wayland. A build in an image without them
# produces an SDL3 that cannot open a window on any machine.
#
# Mesa's -dev metapackages rather than the libgl1-mesa-dev spelling: that one is
# transitional in trixie and libegl-dev/libgl-dev/libgles-dev are what it points
# at. None of this touches the Windows cross-build, which gets its window from
# win32 and its sound from wasapi.
RUN apt-get update && export DEBIAN_FRONTEND=noninteractive \
	&& apt-get install -y --no-install-recommends \
		libx11-dev libxext-dev libxrandr-dev libxi-dev libxcursor-dev \
		libxfixes-dev libxss-dev libxtst-dev \
		libxkbcommon-dev libxkbcommon-x11-dev \
		libwayland-dev libwayland-bin wayland-protocols \
		libdrm-dev libgbm-dev libegl-dev libgl-dev libgles-dev \
		libasound2-dev libpulse-dev libudev-dev libdbus-1-dev \
		libdecor-0-dev \
	&& apt-get clean && rm -rf /var/lib/apt/lists/*

ENV RUSTUP_HOME=/usr/local/rustup CARGO_HOME=/usr/local/cargo
ENV PATH=/usr/local/cargo/bin:$PATH

# The toolchain pin comes from the repository's rust-toolchain.toml, read by
# tools/in-container.sh and passed in here: one pin, in the file that owns it.
# It is installed into the image rather than left to rustup's first run, because
# this container is --rm and would otherwise download a compiler every build.
#
# **rustfmt and clippy are installed here, and that is not a convenience.**
# rust-toolchain.toml asks for both, so rustup installs them on first use --
# which in a --rm container means downloading them on *every build*, over the
# network, before anything compiles. Measured: "info: downloading 2 components"
# at the head of every containerised cargo invocation. --profile minimal plus
# an explicit component list is the same set, decided here instead.
ARG RUST_CHANNEL
RUN test -n "$RUST_CHANNEL" \
	&& curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
		| sh -s -- -y --default-toolchain "$RUST_CHANNEL" --profile minimal --no-modify-path \
		-c rustfmt -c clippy \
	&& rustup target add x86_64-pc-windows-gnu \
		aarch64-linux-android x86_64-linux-android \
	&& chmod -R a+rwX "$RUSTUP_HOME" "$CARGO_HOME"

# ---------------------------------------------------------------------------
# Android
#
# The same argument as everything above: what ships is built here, not on
# whichever machine ran the command. The NDK already pins an API level and
# carries its own headers and stub libraries, so it fixes an APK's floor the
# way the base image fixes a Linux archive's glibc floor -- but the *rest* of
# an Android build does not come with the NDK. aapt2, d8, zipalign, apksigner,
# the platform jar and gradle itself all decide what is in the APK and how it
# is signed, and a machine's own versions of those are exactly the accident
# this image exists to remove.
#
# It is the largest thing in here by a wide margin: the NDK alone is around
# three gigabytes, and the SDK and gradle add a further gigabyte. That is the
# price of an APK that is the same on any machine, and it is paid once per
# image rather than per build.
#
# Every version below is pinned, the same rule every dependency in this
# repository follows.
# platform-tools is adb, which cannot reach a device from in here. It is
# installed because the Android Gradle Plugin looks for it and warns on every
# build when it is missing, and a warning nobody can act on is a warning people
# learn to read past.
ARG ANDROID_CMDLINE_TOOLS=11076708
ARG ANDROID_PLATFORM=35
ARG ANDROID_BUILD_TOOLS=35.0.0
ARG ANDROID_NDK=27.2.12479018
ARG GRADLE_VERSION=8.11.1

ENV ANDROID_HOME=/opt/android-sdk
ENV ANDROID_SDK_ROOT=/opt/android-sdk
ENV ANDROID_NDK_HOME=/opt/android-sdk/ndk/27.2.12479018
ENV GRADLE_HOME=/opt/gradle

# The JDK gradle and the Android build tools run on. Android Gradle Plugin 8.7
# wants 17 or newer, and 21 is the oldest trixie carries -- it ships 21 and 25
# and nothing below. Headless because nothing in here draws.
RUN apt-get update && export DEBIAN_FRONTEND=noninteractive \
	&& apt-get install -y --no-install-recommends \
		openjdk-21-jdk-headless \
		unzip \
	&& apt-get clean && rm -rf /var/lib/apt/lists/*

RUN set -eux; \
	mkdir -p "$ANDROID_HOME/cmdline-tools"; \
	curl -fsSL -o /tmp/cmdline-tools.zip \
		"https://dl.google.com/android/repository/commandlinetools-linux-${ANDROID_CMDLINE_TOOLS}_latest.zip"; \
	unzip -q /tmp/cmdline-tools.zip -d "$ANDROID_HOME/cmdline-tools"; \
	mv "$ANDROID_HOME/cmdline-tools/cmdline-tools" "$ANDROID_HOME/cmdline-tools/latest"; \
	rm /tmp/cmdline-tools.zip; \
	yes | "$ANDROID_HOME/cmdline-tools/latest/bin/sdkmanager" --licenses >/dev/null; \
	"$ANDROID_HOME/cmdline-tools/latest/bin/sdkmanager" \
		"platforms;android-${ANDROID_PLATFORM}" \
		"build-tools;${ANDROID_BUILD_TOOLS}" \
		"ndk;${ANDROID_NDK}" \
		platform-tools >/dev/null; \
	test -x "$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64/bin/clang"; \
	chmod -R a+rX "$ANDROID_HOME"

# Gradle itself, rather than a wrapper jar committed to this repository. A
# wrapper is a binary in the tree that decides what builds the app, which is
# the one thing this image is for.
RUN set -eux; \
	curl -fsSL -o /tmp/gradle.zip \
		"https://services.gradle.org/distributions/gradle-${GRADLE_VERSION}-bin.zip"; \
	unzip -q /tmp/gradle.zip -d /opt; \
	mv "/opt/gradle-${GRADLE_VERSION}" "$GRADLE_HOME"; \
	rm /tmp/gradle.zip
ENV PATH=$GRADLE_HOME/bin:$ANDROID_HOME/cmdline-tools/latest/bin:$PATH
# ---------------------------------------------------------------------------

# A default only. tools/in-container.sh mounts the tree at its own absolute path
# and passes -w to match, because the pkg-config and CMake files that SDL3 and
# ffmpeg install record their prefix absolutely -- a tree mounted somewhere else
# produces libraries that only resolve in here.
WORKDIR /

# What tools/in-container.sh looks for: a build already inside this image runs
# rather than starting another container.
ENV DAYS_IN_CONTAINER=1
