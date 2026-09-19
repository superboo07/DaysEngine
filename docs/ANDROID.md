# Building for Android

The app is `target/dist/daysengine-android-<commit>.apk`. It bundles no game
data, exactly as the desktop builds do not: the first time it is opened it asks
the player which folder their own install is in, through Android's own folder
picker, and everything after that is the game's own art.

```bash
just android-deps    # SDL3 + ffmpeg for both ABIs. Slow; once per checkout.
just android         # a debug-signed APK, ready for `adb install`
just dist-android    # an unsigned release APK in target/dist/
```

`tools/build-android.sh` is where the build actually lives, so the `just`
recipes are one line each and the script can be run directly.

## What you need on the build machine

Podman or docker, and nothing else. The NDK, the Android SDK's build tools,
gradle and the JDK are all pinned in `tools/dist/Dockerfile` and every step of
the build runs inside that image — the same image a Linux or Windows archive is
built in, and for the same reason. `tools/in-container.sh` builds it on first
use and re-execs into it.

The image is large — it goes from about 3.5 GB to 6.8 GB, most of it the NDK.
That is paid once per image rather than per build, and what it buys is an APK
that does not depend on which machine assembled it: `aapt2`, `d8`, `zipalign`
and `apksigner` all decide what ends up in an APK, and a developer's own
versions of those are exactly the accident this image exists to remove.

**Two settings are what make that true rather than nearly true**, and both are
there because the first containerised build broke them. `buildToolsVersion` is
pinned in `android/app/build.gradle`: left to itself, AGP picks a version and
*downloads it in the middle of the build*, which it did — and since the build
container is `--rm`, it would have done so on every build forever.
`android.builder.sdkDownload=false` in `android/gradle.properties` then makes a
component the image does not have a hard error rather than a quiet fetch.

Gradle's own caches go in `target/android/gradle-home` for the same `--rm`
reason, which is why the first build spends a few minutes fetching the Android
Gradle Plugin and later ones do not.

## The two ABIs

`arm64-v8a` and `x86_64`: every phone and tablet of the last several years, and
the emulator. 32-bit ARM is not built — Play has required 64-bit since 2019 and
nothing in this engine needs a 32-bit process, since `days-gpk` and `days-ui`
*parse* the game's 32-bit PE files rather than running them.

The list is `abis` in `tools/build-android.sh`, four fields per line, and
adding one is the whole change.

## The engine is a library here

An Android app has no `main`. `SDLActivity` loads the shared objects
`getLibraries()` names, then looks up `SDL_main` in the last of them and calls
it on a thread of its own. So the build produces `libdaysengine.so`, and
`SDL_main` is `src/android.rs`.

That is why the loop lives in `daysengine::game::run` rather than in
`src/main.rs`, which is now four lines calling it. **One loop for every
platform** — the same rule that made the inspection subcommands part of
`daysengine` instead of a second binary. `src/android.rs` does the three things
this platform needs before that loop can start, and then calls it.

`cargo rustc --crate-type cdylib` rather than a `crate-type` in `Cargo.toml`,
because that key is not per-target: putting `"cdylib"` there would link a
second copy of the whole engine into every desktop `cargo build` for nothing.

## The player's install is a folder, not a path

This is the one deep difference from every other platform, and it reaches all
the way down into the pack reader.

Android has no directory an app may open by name. What the player grants in the
system folder picker is a **tree**, and a file inside it is reached by document
id through `ContentResolver`. There is no path, and `std::fs` cannot be pointed
at one.

So every read and write of a file in the install goes through
`install::storage`, a backend installed once per process before anything is
mounted. On a desktop it is `Local` and each call is the `std::fs` call it
replaced; on Android it is `install::saf`, whose other half is
`android/app/src/main/java/org/daysengine/Saf.java`.

Paths still work, because they are made up. The root is `/saf`, a name no
Android filesystem has, and `install::saf` strips it and walks what is left one
component at a time, turning each into the document id its parent's listing
gave. So `install::vfs` goes on building `/saf/Packs/System.GPK` and the INI
files go on saying `Save/GlobalFlag.DAT`.

**A pack is still read a piece at a time.** `openFileDescriptor` on a local
document gives a real, seekable descriptor, and the Java side detaches it so
the native `File` owns it — so once a pack is open the engine reads it with
ordinary seeks, with no JNI in the decode path and nothing pulling a
twenty-gigabyte install through Binder. Java is called for opening, listing,
creating, renaming and deleting, and for nothing else.

### No permissions

The APK asks for `VIBRATE` and nothing else. The tree grant *is* the authority
to read the folder, and it is persisted, so the picker appears once. Asking for
storage permission on top of it would be asking for every file on the device in
order to read the one folder the player already chose.

### `DaysEngine.ini` goes in the folder the player chose

Beside their `Packs`, as `install::engine::set_directory` is told at boot.

"Beside the running binary", which is where every other platform keeps it, is
`/system/bin` here — a system directory no app may write. The granted folder is
the closest thing this platform has to the same idea: it is writable, it is
somewhere a file manager can open, and it stays with the install it belongs to.
That last part is what makes the setting editable at all. An app's private
directory would also have worked and is what this did first, and it is
`/data/data/org.daysengine/files` — which no file manager will show and no
player can reach without `adb run-as` or root, so the one file here a player is
*meant* to edit would have been the one file they could not.

It is read and written through `install::storage` like everything else in the
install, which is what makes a path inside the tree openable at all.

## Touch

Nothing in the engine knows what a finger is, and nothing needs to. SDL reports
a touch as a mouse as well, so a tap arrives as a motion to where the finger
landed followed by a left button press at the same point, in that order, in one
pump of the event queue. Every screen here highlights what the pointer is over
and acts on the click, so the widget under the finger is selected by the motion
before the press is read, and the menus work under a finger unchanged.

`src/android.rs` sets both halves of that hint explicitly. The reverse
conversion is turned **off**: with both on, a real mouse — on a Chromebook, or
over USB — would raise a synthetic touch beside its own click and every press
would arrive twice.

The hardware Back button reaches the engine rather than finishing the activity
(`SDL_ANDROID_TRAP_BACK_BUTTON` in the manifest). SDL reports it as the
`AC BACK` key, which `install::binding` binds to `Cancel` beside Escape — so it
backs out of a menu, and does **not** quit during playback the way Escape does.
A gesture that easy to make by accident should not be the one that closes the
game.

## What ships in the APK

Per ABI: `libdaysengine.so`, `libSDL3.so`, and the six libav libraries the
engine actually names. `libavdevice` is built and never travels — `rusty_ffmpeg`
always puts it on the link line, nothing in `src/media` calls it, and the linker
drops it.

**SDL3 is a separate file here, and that is not a licensing change.** On
Android SDL is half Java: `SDLActivity`, `SDLSurface` and `SDLAudioManager` are
classes in the APK whose native methods bind against `libSDL3.so` by name, and
`System.loadLibrary("SDL3")` is what binds them. A static SDL3 inside
`libdaysengine.so` has no library for that call to find. zlib attaches no
condition either way.

ffmpeg stays shared for the reason it is shared everywhere: LGPL v2.1 permits
static linking only against an obligation to let the player relink.
`FFMPEG-SOURCE.txt` and `COPYING.LGPLv2.1` are written beside the libraries by
`tools/build-ffmpeg.sh`; **an APK you publish must carry them.**

The libraries have unversioned sonames — `libavcodec.so`, not
`libavcodec.so.63` — because Android's packager only extracts files named
`lib*.so` and its linker only looks them up by that name. ffmpeg's own
`--target-os=android` does this; nothing here renames anything.

## The Java half

`android/` is a small gradle project. Three classes:

| | |
|---|---|
| `PickerActivity` | The launcher, and the only screen in the app that is not the game's own art. Four words and a button that opens Android's folder picker. It checks the chosen folder has a `Packs` in it — the same test the desktop makes — so a wrong folder is answered here, where it can be asked again |
| `DaysEngineActivity` | `SDLActivity` with the library list and no arguments. Also brings the grant back when Android has restored it straight from the task stack, which resets every static in the app |
| `Saf` | The other side of `install::saf` |

SDL's own Java classes are compiled from `third_party/sdl`, not copied in:
`SDLActivity` is the other side of `libSDL3.so`'s native methods and the two
have to come from the same SDL.

There is no gradle wrapper. A wrapper is a binary committed to the tree that
decides what builds the app, which is the one thing the build image exists to
take out of a repository's hands; gradle is pinned in the image instead.

Everything gradle produces goes under `target/` — `jniLibs`, the assets, the
build directory, its caches, and the project cache it would otherwise keep in
a `.gradle` beside the build file (`--project-cache-dir`). Nothing generated
lands in `android/`.

## Known gaps

- **Not run against a real install on a device.** The build is verified end to
  end — both ABIs cross-compile, and the APK carries `libdaysengine.so`,
  `libSDL3.so` and the six libav libraries for each, with `SDL_main` and
  `JNI_OnLoad` exported and nothing linked but those, `liblog` and Android's
  own libc. Nobody has played it. In particular **the Storage Access Framework
  path has never executed**: it is written against Android's documented
  behaviour and it compiles, and that is not the same as working. This is the
  same state `docs/WINDOWS.md` records for Windows, and it is the first thing
  to check on a device.
- **The release APK is unsigned.** There is no signing key in this repository
  and there should not be. `apksigner sign --ks <your.keystore>` is the step
  after `just dist-android`.
- **The Option screen's windowed/full-screen toggle does nothing here.** An
  Android window is the display. The widget is still drawn, because it is the
  game's own screen and the tables come from the player's module.
