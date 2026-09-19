//! The player's install, when it is an Android Storage Access Framework tree.
//!
//! Android has no directory an app may simply open by name. What the player
//! grants through the system folder picker is a **tree**, and a file inside it
//! is reached by document id through `ContentResolver` — there is no path, and
//! `std::fs` cannot be pointed at one. So this is the [`Backend`] the engine
//! runs on there, and everything above [`storage`](super::storage) goes on
//! spelling paths.
//!
//! # Paths are made up, and that is the whole trick
//!
//! The root is [`ROOT`], a name no Android filesystem has. Every path the
//! engine builds hangs off it — `/saf/Packs/System.GPK`,
//! `/saf/Save/GlobalFlag.DAT` — and this strips the prefix and walks what is
//! left one component at a time, turning each into the document id its parent
//! listing gave. A listing is asked for once and kept, because mounting reads
//! the root twice over (every `.exe` for the archive key, every `.dll` for the
//! menu and route modules) and a `ContentResolver` query is a Binder round
//! trip rather than a `readdir`.
//!
//! Lookups are case-insensitive, for the reason every other lookup in this
//! engine is: an install that came off a Windows machine spells `Packs` and
//! `PACKS` interchangeably and the `.INI` files do not agree with the
//! directory.
//!
//! # A pack is still read a piece at a time
//!
//! `ContentResolver.openFileDescriptor` on a local document gives a **real,
//! seekable file descriptor**, so once a pack is open the engine reads it with
//! `pread` exactly as it does everywhere else — no JNI in the decode path, and
//! no need to pull a twenty-gigabyte install through Binder. Java is called
//! for opening, listing, creating, renaming and deleting, and for nothing
//! else. That matters because those calls happen while the install is being
//! mounted, on the thread SDL calls `SDL_main` on, which is already a Java
//! thread; a decode thread that only reads an already-open pack never touches
//! the JVM at all.
//!
//! # Provenance
//!
//! Nothing here is recovered from the game. This is Android's own API and the
//! behaviour is Android's: the naming rule `createFile` relies on is
//! `FileUtils.buildUniqueFile`, which keeps a display name whose extension
//! maps to no known MIME type — every name the engine writes (`.DAT`, `.tmp`,
//! `.new`, and `DaysEngine.ini`) is one of those, so a file is created under
//! the name it asked for.

#![allow(unsafe_code)]

use super::storage::{self, Backend, DirEntry};
use std::collections::HashMap;
use std::fs::File;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// The made-up root every path in a SAF install hangs off.
///
/// It only has to be something no real path collides with and something
/// legible in a log line, because nothing ever opens it.
pub const ROOT: &str = "/saf";

/// What Android calls a directory.
const MIME_DIR: &str = "vnd.android.document/directory";

/// What a created file is called, and why it keeps the name it was given: see
/// the provenance note at the top of this module.
const MIME_FILE: &str = "application/octet-stream";

/// One entry in a directory listing.
#[derive(Debug, Clone)]
struct Node {
    document: String,
    is_dir: bool,
}

/// A directory whose listing has been read.
///
/// Lowercased name -> the entry, alongside the name as the provider spells it.
/// Both halves are wanted: lookups here are case-insensitive for the reason
/// every lookup in this engine is, and [`Backend::read_dir`] has to hand back
/// the name the provider actually gave.
#[derive(Debug)]
struct Listing {
    children: HashMap<String, (String, Node)>,
}

/// The install, as a granted tree.
pub struct Saf {
    root: PathBuf,
    root_document: String,
    /// Relative directory path, lowercased and `/`-separated, to its listing.
    /// The root is the empty string.
    listings: Mutex<HashMap<String, Listing>>,
}

impl Saf {
    /// Wraps the tree whose root document is `root_document`.
    pub fn new(root_document: String) -> Saf {
        Saf {
            root: PathBuf::from(ROOT),
            root_document,
            listings: Mutex::new(HashMap::new()),
        }
    }

    /// The path every install path is built from.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// A path under [`ROOT`] as a `/`-separated relative path.
    fn relative(&self, path: &Path) -> io::Result<String> {
        storage::under(&self.root, path)
    }

    /// The document id of a directory, listing whatever is needed on the way.
    fn directory(&self, relative: &str) -> io::Result<String> {
        if relative.is_empty() {
            return Ok(self.root_document.clone());
        }
        let node = self.node(relative)?;
        if !node.is_dir {
            return Err(io::Error::new(
                io::ErrorKind::NotADirectory,
                format!("{ROOT}/{relative} is not a directory"),
            ));
        }
        Ok(node.document)
    }

    /// The entry a relative path names.
    fn node(&self, relative: &str) -> io::Result<Node> {
        let (parent, name) = storage::split(relative);
        self.fill(parent)?;
        let listings = self.listings.lock().expect("saf listings poisoned");
        listings
            .get(parent)
            .and_then(|dir| dir.children.get(&name.to_ascii_lowercase()))
            .map(|(_, node)| node.clone())
            .ok_or_else(|| missing(relative))
    }

    /// Reads a directory's listing if it has not been read yet.
    fn fill(&self, relative: &str) -> io::Result<()> {
        if self
            .listings
            .lock()
            .expect("saf listings poisoned")
            .contains_key(relative)
        {
            return Ok(());
        }
        let document = self.directory(relative)?;
        let mut children = HashMap::new();
        for row in bridge::list(&document)? {
            children.insert(
                row.name.to_ascii_lowercase(),
                (
                    row.name,
                    Node {
                        document: row.document,
                        is_dir: row.is_dir,
                    },
                ),
            );
        }
        self.listings
            .lock()
            .expect("saf listings poisoned")
            .insert(relative.to_string(), Listing { children });
        Ok(())
    }

    /// Drops a directory's listing, so the next lookup asks the provider again.
    fn forget(&self, relative: &str) {
        self.listings
            .lock()
            .expect("saf listings poisoned")
            .remove(relative);
    }

    /// The document a path names, creating the file if it is not there.
    fn for_writing(&self, path: &Path) -> io::Result<String> {
        let relative = self.relative(path)?;
        if let Ok(node) = self.node(&relative) {
            if node.is_dir {
                return Err(io::Error::new(
                    io::ErrorKind::IsADirectory,
                    format!("{} is a directory", path.display()),
                ));
            }
            return Ok(node.document);
        }
        let (parent, name) = storage::split(&relative);
        let parent_document = self.directory(parent)?;
        let document = bridge::create(&parent_document, MIME_FILE, name)?;
        self.forget(parent);
        Ok(document)
    }
}

impl Backend for Saf {
    fn open(&self, path: &Path) -> io::Result<File> {
        let node = self.node(&self.relative(path)?)?;
        bridge::open(&node.document, "r")
    }

    fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
        let mut file = self.open(path)?;
        let mut out = Vec::new();
        file.read_to_end(&mut out)?;
        Ok(out)
    }

    fn write(&self, path: &Path, bytes: &[u8]) -> io::Result<()> {
        let document = self.for_writing(path)?;
        // "rwt" rather than "w": a provider is only obliged to truncate for a
        // mode that says so, and a short save written over a long one would
        // otherwise keep the old tail.
        let mut file = bridge::open(&document, "rwt")?;
        file.write_all(bytes)?;
        file.flush()
    }

    fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        let from_relative = self.relative(from)?;
        let to_relative = self.relative(to)?;
        let node = self.node(&from_relative)?;
        // `renameDocument` refuses a name that is taken, and every caller here
        // is replacing a file it means to replace — a save slot, the settings
        // — so what is in the way goes first.
        if let Ok(existing) = self.node(&to_relative) {
            bridge::delete(&existing.document)?;
        }
        let (_, name) = storage::split(&to_relative);
        bridge::rename(&node.document, name)?;
        self.forget(storage::split(&from_relative).0);
        self.forget(storage::split(&to_relative).0);
        Ok(())
    }

    fn create_dir_all(&self, path: &Path) -> io::Result<()> {
        let relative = self.relative(path)?;
        let mut here = String::new();
        for part in relative.split('/').filter(|p| !p.is_empty()) {
            let below = if here.is_empty() {
                part.to_string()
            } else {
                format!("{here}/{part}")
            };
            match self.node(&below) {
                Ok(node) if node.is_dir => {}
                Ok(_) => {
                    return Err(io::Error::new(
                        io::ErrorKind::AlreadyExists,
                        format!("{ROOT}/{below} is a file"),
                    ))
                }
                Err(_) => {
                    let parent = self.directory(&here)?;
                    bridge::create(&parent, MIME_DIR, part)?;
                    self.forget(&here);
                }
            }
            here = below;
        }
        Ok(())
    }

    fn read_dir(&self, path: &Path) -> io::Result<Vec<DirEntry>> {
        let relative = self.relative(path)?;
        self.fill(&relative)?;
        let listings = self.listings.lock().expect("saf listings poisoned");
        let listing = listings.get(&relative).ok_or_else(|| missing(&relative))?;
        Ok(listing
            .children
            .values()
            .map(|(name, node)| DirEntry {
                path: path.join(name),
                is_dir: node.is_dir,
            })
            .collect())
    }

    fn is_file(&self, path: &Path) -> bool {
        self.relative(path)
            .and_then(|r| self.node(&r))
            .is_ok_and(|node| !node.is_dir)
    }

    fn is_dir(&self, path: &Path) -> bool {
        self.relative(path)
            .and_then(|r| self.node(&r))
            .is_ok_and(|node| node.is_dir)
    }

    fn root(&self) -> Option<&Path> {
        Some(&self.root)
    }
}

/// The error a caller checking for `NotFound` has to get.
///
/// Several of them do — a save slot that has never been written, a settings
/// file on a fresh install, the flag store before the first chapter is
/// finished — and each treats `NotFound` as "not there yet" and anything else
/// as a fault worth logging.
fn missing(relative: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::NotFound,
        format!("{ROOT}/{relative} is not in the granted folder"),
    )
}

/// The calls into the app's `Saf` class, and the JNI needed to make them.
///
/// Every `unsafe` here is the FFI Rust requires the keyword on: taking the
/// `JavaVM` the loader hands `JNI_OnLoad`, and turning a descriptor the
/// provider has already detached into the [`File`] that owns it. There is no
/// safe binding to call instead.
mod bridge {
    use jni::objects::{GlobalRef, JClass, JObjectArray, JString, JValue};
    use jni::{JNIEnv, JavaVM};
    use std::fs::File;
    use std::io;
    use std::os::fd::FromRawFd;
    use std::sync::OnceLock;

    /// The app class every call below is a static method of.
    const CLASS: &str = "org/daysengine/Saf";

    static VM: OnceLock<JavaVM> = OnceLock::new();
    /// A **global** reference, taken while the app's class loader is on the
    /// stack.
    ///
    /// `FindClass` resolves against the class loader of the frame that calls
    /// it, and a thread the engine attached itself has none — it gets the
    /// system loader, which knows nothing about the APK. Looking the class up
    /// in `JNI_OnLoad`, which the loader calls from `System.loadLibrary`, is
    /// the one moment the app's loader is there to be asked.
    static SAF: OnceLock<GlobalRef> = OnceLock::new();

    /// Called by the runtime when `libdaysengine.so` is loaded.
    ///
    /// # Safety
    ///
    /// `vm` is the `JavaVM` the Android runtime passes, which is valid for the
    /// life of the process.
    #[no_mangle]
    pub unsafe extern "system" fn JNI_OnLoad(
        vm: *mut jni::sys::JavaVM,
        _reserved: *mut std::ffi::c_void,
    ) -> jni::sys::jint {
        let Ok(vm) = (unsafe { JavaVM::from_raw(vm) }) else {
            return 0;
        };
        if let Ok(mut env) = vm.get_env() {
            if let Ok(class) = env.find_class(CLASS) {
                if let Ok(global) = env.new_global_ref(class) {
                    let _ = SAF.set(global);
                }
            }
            // A missing class is not fatal here. `JNI_OnLoad` has to return a
            // usable JNI version or the library will not load at all, and the
            // first call through this module reports the real problem with a
            // name attached.
            let _ = env.exception_clear();
        }
        let _ = VM.set(vm);
        jni::sys::JNI_VERSION_1_6
    }

    /// Whether the bridge came up. Used to decide there is a tree at all.
    pub fn present() -> bool {
        VM.get().is_some() && SAF.get().is_some()
    }

    fn fault(what: &str) -> io::Error {
        io::Error::other(format!("Android storage bridge: {what}"))
    }

    /// Runs `body` with an attached environment and the `Saf` class.
    ///
    /// The thread is attached if it is not already — the engine's own threads
    /// are not Java threads — and detached again when the guard drops, which
    /// is what keeps a decode thread from being left registered with the VM.
    fn with<T>(body: impl FnOnce(&mut JNIEnv, &JClass) -> io::Result<T>) -> io::Result<T> {
        let vm = VM
            .get()
            .ok_or_else(|| fault("no JavaVM; not on Android?"))?;
        let saf = SAF.get().ok_or_else(|| fault("no org.daysengine.Saf"))?;
        let mut env = vm
            .attach_current_thread()
            .map_err(|err| fault(&format!("cannot attach this thread: {err}")))?;
        // A borrowed view of the global reference. `JObject` and `JClass` do
        // not own what they point at, so this neither copies the reference nor
        // releases it.
        let class = unsafe { JClass::from_raw(saf.as_obj().as_raw()) };
        let out = body(&mut env, &class);
        if env.exception_check().unwrap_or(false) {
            let _ = env.exception_describe();
            let _ = env.exception_clear();
            return out.and(Err(fault("the call threw")));
        }
        out
    }

    fn string(env: &mut JNIEnv, value: &JString) -> io::Result<String> {
        env.get_string(value)
            .map(|s| s.into())
            .map_err(|err| fault(&format!("reading a returned string: {err}")))
    }

    /// One row of a directory listing.
    pub struct Row {
        pub name: String,
        pub document: String,
        pub is_dir: bool,
    }

    /// Every child of a directory document.
    ///
    /// The Java side returns three strings per child rather than an object
    /// array of its own type, because three `GetObjectArrayElement` calls and
    /// no class lookup is the whole of the marshalling.
    pub fn list(document: &str) -> io::Result<Vec<Row>> {
        with(|env, class| {
            let argument = env
                .new_string(document)
                .map_err(|err| fault(&format!("{err}")))?;
            let returned = env
                .call_static_method(
                    class,
                    "list",
                    "(Ljava/lang/String;)[Ljava/lang/String;",
                    &[JValue::Object(&argument)],
                )
                .and_then(|value| value.l())
                .map_err(|err| fault(&format!("list: {err}")))?;
            if returned.is_null() {
                return Err(fault("list returned nothing"));
            }
            let array: JObjectArray = returned.into();
            let len = env
                .get_array_length(&array)
                .map_err(|err| fault(&format!("{err}")))?;
            let mut rows = Vec::with_capacity((len / 3) as usize);
            for i in (0..len).step_by(3) {
                let mut at = |offset: i32| -> io::Result<String> {
                    let item = env
                        .get_object_array_element(&array, i + offset)
                        .map_err(|err| fault(&format!("{err}")))?;
                    string(env, &JString::from(item))
                };
                let name = at(0)?;
                let document = at(1)?;
                let kind = at(2)?;
                rows.push(Row {
                    name,
                    document,
                    is_dir: kind == "d",
                });
            }
            Ok(rows)
        })
    }

    /// Opens a document and takes ownership of the descriptor.
    ///
    /// The Java side calls `detachFd`, so the descriptor outlives the
    /// `ParcelFileDescriptor` and closing the [`File`] is what closes it.
    pub fn open(document: &str, mode: &str) -> io::Result<File> {
        let fd = with(|env, class| {
            let document = env
                .new_string(document)
                .map_err(|err| fault(&format!("{err}")))?;
            let mode = env
                .new_string(mode)
                .map_err(|err| fault(&format!("{err}")))?;
            env.call_static_method(
                class,
                "open",
                "(Ljava/lang/String;Ljava/lang/String;)I",
                &[JValue::Object(&document), JValue::Object(&mode)],
            )
            .and_then(|value| value.i())
            .map_err(|err| fault(&format!("open: {err}")))
        })?;
        if fd < 0 {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "the provider would not open that document",
            ));
        }
        Ok(unsafe { File::from_raw_fd(fd) })
    }

    /// Creates a child document and returns its id.
    pub fn create(parent: &str, mime: &str, name: &str) -> io::Result<String> {
        with(|env, class| {
            let parent = env
                .new_string(parent)
                .map_err(|err| fault(&format!("{err}")))?;
            let mime = env
                .new_string(mime)
                .map_err(|err| fault(&format!("{err}")))?;
            let name = env
                .new_string(name)
                .map_err(|err| fault(&format!("{err}")))?;
            let returned = env
                .call_static_method(
                    class,
                    "create",
                    "(Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;)Ljava/lang/String;",
                    &[
                        JValue::Object(&parent),
                        JValue::Object(&mime),
                        JValue::Object(&name),
                    ],
                )
                .and_then(|value| value.l())
                .map_err(|err| fault(&format!("create: {err}")))?;
            if returned.is_null() {
                return Err(io::Error::other("the provider would not create that file"));
            }
            string(env, &JString::from(returned))
        })
    }

    pub fn rename(document: &str, name: &str) -> io::Result<()> {
        one_call(document, Some(name), "rename", true)
    }

    pub fn delete(document: &str) -> io::Result<()> {
        one_call(document, None, "delete", false)
    }

    /// `rename(String, String)` and `delete(String)`, which differ only in
    /// whether there is a second argument.
    fn one_call(document: &str, name: Option<&str>, method: &str, named: bool) -> io::Result<()> {
        let ok = with(|env, class| {
            let document = env
                .new_string(document)
                .map_err(|err| fault(&format!("{err}")))?;
            let call = if named {
                let name = env
                    .new_string(name.unwrap_or_default())
                    .map_err(|err| fault(&format!("{err}")))?;
                env.call_static_method(
                    class,
                    method,
                    "(Ljava/lang/String;Ljava/lang/String;)Z",
                    &[JValue::Object(&document), JValue::Object(&name)],
                )
            } else {
                env.call_static_method(
                    class,
                    method,
                    "(Ljava/lang/String;)Z",
                    &[JValue::Object(&document)],
                )
            };
            call.and_then(|value| value.z())
                .map_err(|err| fault(&format!("{method}: {err}")))
        })?;
        if ok {
            Ok(())
        } else {
            Err(io::Error::other(format!("the provider refused {method}")))
        }
    }

    /// One `String` the `Saf` class answers with no arguments.
    fn no_argument(method: &str) -> io::Result<String> {
        with(|env, class| {
            let returned = env
                .call_static_method(class, method, "()Ljava/lang/String;", &[])
                .and_then(|value| value.l())
                .map_err(|err| fault(&format!("{method}: {err}")))?;
            if returned.is_null() {
                return Err(fault(&format!("{method} returned nothing")));
            }
            string(env, &JString::from(returned))
        })
    }

    /// The document id of the folder the player granted.
    pub fn root_document() -> io::Result<String> {
        no_argument("rootDocument")
    }
}

pub use bridge::{present, root_document};
