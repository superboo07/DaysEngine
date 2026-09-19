//! Where the engine's file reads and writes actually go.
//!
//! Everywhere else in this crate, a game file is a [`Path`] under the install
//! root and `std::fs` opens it. That is true on every desktop platform and it
//! is not true on Android, where the player grants access to a *folder* rather
//! than to a path: what comes back from the folder picker is a Storage Access
//! Framework tree, and a file inside it is reached by document id through
//! `ContentResolver`. There is no name the C library can open.
//!
//! So the reads and writes go through here instead. A [`Backend`] is installed
//! once, before anything is mounted, and the free functions below mirror the
//! `std::fs` calls this crate used to make directly. On a desktop the backend
//! is [`Local`] and each one is the `std::fs` call it replaced, with the same
//! [`io::Error`] coming back; on Android it is the SAF tree (see
//! `install::saf`).
//!
//! # Why a process-wide backend rather than a handle passed around
//!
//! There is exactly one install open per process — [`Vfs::mount`] takes a root
//! and the whole run is that install — so a handle threaded through every
//! caller would be the same value at every call site. The cost of threading it
//! is not the plumbing: it is that `days-gpk`, `days-save` and the PE parsers
//! all take paths, and giving each of them a storage parameter would rewrite
//! the format readers that the recovered behaviour rests on in order to say
//! something none of them needs to know.
//!
//! The invariant that makes it safe is narrow and checked: [`install`] may be
//! called once, and it is called before the first read. [`Local`] is what a
//! process that never calls it gets, which is every desktop run and every
//! test.
//!
//! [`Vfs::mount`]: super::vfs::Vfs::mount

use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// One entry from [`read_dir`].
///
/// A [`PathBuf`] and a kind, which is all any caller here wants: the pack
/// scanner keeps the `.gpk` files, [`Binaries`](super::binaries) keeps the
/// `.dll` and `.exe` files, and neither asks anything else about them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirEntry {
    pub path: PathBuf,
    pub is_dir: bool,
}

/// What the engine's file access is implemented against.
///
/// Implementors are asked for absolute paths under the install root. The
/// Android backend is handed the same paths and resolves them against the
/// granted tree, which is why nothing above this line has to know which one is
/// installed.
pub trait Backend: Send + Sync {
    /// Opens a file for reading, seekably.
    ///
    /// A [`File`] rather than a `Box<dyn Read + Seek>` because a GPK is read a
    /// piece at a time out of a `File` held in `days_gpk::Archive`, and
    /// because SAF has a real file descriptor to give — see
    /// [`Archive::from_file`](days_gpk::Archive::from_file).
    fn open(&self, path: &Path) -> io::Result<File>;
    fn read(&self, path: &Path) -> io::Result<Vec<u8>>;
    fn write(&self, path: &Path, bytes: &[u8]) -> io::Result<()>;
    /// Replaces `to` with `from`, which must already exist.
    fn rename(&self, from: &Path, to: &Path) -> io::Result<()>;
    fn create_dir_all(&self, path: &Path) -> io::Result<()>;
    fn read_dir(&self, path: &Path) -> io::Result<Vec<DirEntry>>;
    /// Whether the path is a file that can be opened.
    fn is_file(&self, path: &Path) -> bool;
    fn is_dir(&self, path: &Path) -> bool;

    /// The one install this backend is, when it is only ever one.
    ///
    /// [`Local`] answers `None`: it is the whole filesystem, and which
    /// directory in it holds the game is a question the command line, the
    /// environment and the working directory answer. A backend that exists
    /// *because* the player picked a folder already knows, and this is how it
    /// tells `discover_game_dir` instead of that function growing a second
    /// platform branch.
    fn root(&self) -> Option<&Path> {
        None
    }
}

/// The ordinary filesystem: every call is the `std::fs` one it replaced.
pub struct Local;

impl Backend for Local {
    fn open(&self, path: &Path) -> io::Result<File> {
        File::open(path)
    }

    fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
        std::fs::read(path)
    }

    fn write(&self, path: &Path, bytes: &[u8]) -> io::Result<()> {
        std::fs::write(path, bytes)
    }

    fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        std::fs::rename(from, to)
    }

    fn create_dir_all(&self, path: &Path) -> io::Result<()> {
        std::fs::create_dir_all(path)
    }

    fn read_dir(&self, path: &Path) -> io::Result<Vec<DirEntry>> {
        let mut out = Vec::new();
        for entry in std::fs::read_dir(path)? {
            let entry = entry?;
            out.push(DirEntry {
                is_dir: entry.path().is_dir(),
                path: entry.path(),
            });
        }
        Ok(out)
    }

    fn is_file(&self, path: &Path) -> bool {
        path.is_file()
    }

    fn is_dir(&self, path: &Path) -> bool {
        path.is_dir()
    }
}

static BACKEND: OnceLock<Box<dyn Backend>> = OnceLock::new();

/// Installs the backend for this process. Call it before mounting anything.
///
/// Returns `Err` with the backend back when one is already installed, rather
/// than swapping it: a second install would leave files opened through the
/// first one referring to a tree nothing else can see.
pub fn install(backend: Box<dyn Backend>) -> Result<(), Box<dyn Backend>> {
    BACKEND.set(backend)
}

/// The installed backend, or [`Local`] when nothing installed one.
pub fn backend() -> &'static dyn Backend {
    BACKEND.get_or_init(|| Box::new(Local)).as_ref()
}

pub fn open(path: impl AsRef<Path>) -> io::Result<File> {
    backend().open(path.as_ref())
}

pub fn read(path: impl AsRef<Path>) -> io::Result<Vec<u8>> {
    backend().read(path.as_ref())
}

/// Reads a file as UTF-8, with the same `InvalidData` error `std::fs` gives.
pub fn read_to_string(path: impl AsRef<Path>) -> io::Result<String> {
    let bytes = read(path)?;
    String::from_utf8(bytes).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
}

pub fn write(path: impl AsRef<Path>, bytes: impl AsRef<[u8]>) -> io::Result<()> {
    backend().write(path.as_ref(), bytes.as_ref())
}

pub fn rename(from: impl AsRef<Path>, to: impl AsRef<Path>) -> io::Result<()> {
    backend().rename(from.as_ref(), to.as_ref())
}

pub fn create_dir_all(path: impl AsRef<Path>) -> io::Result<()> {
    backend().create_dir_all(path.as_ref())
}

/// Every entry directly in a directory, unordered — callers that care sort.
pub fn read_dir(path: impl AsRef<Path>) -> io::Result<Vec<DirEntry>> {
    backend().read_dir(path.as_ref())
}

pub fn is_file(path: impl AsRef<Path>) -> bool {
    backend().is_file(path.as_ref())
}

pub fn is_dir(path: impl AsRef<Path>) -> bool {
    backend().is_dir(path.as_ref())
}

/// The install the backend is, when it is only ever one. See [`Backend::root`].
pub fn root() -> Option<&'static Path> {
    backend().root()
}

/// A path under a backend's own root, as a `/`-separated relative path.
///
/// For a backend whose root is invented rather than real — Android's, where
/// the path never reaches a filesystem and is only ever a way to name a
/// document — this is the whole of turning what the engine spells into
/// something that can be walked. `..` and `.` are dropped rather than
/// followed: there is nothing above the granted folder, and a path that tried
/// to climb out of it would be asking for something the player did not grant.
pub fn under(root: &Path, path: &Path) -> io::Result<String> {
    let rest = path.strip_prefix(root).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{} is not inside {}", path.display(), root.display()),
        )
    })?;
    Ok(rest
        .components()
        .filter_map(|part| match part {
            std::path::Component::Normal(name) => Some(name.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/"))
}

/// Splits a relative path into its parent and its final component.
///
/// `"Packs/System.GPK"` -> `("Packs", "System.GPK")`, and `"Packs"` ->
/// `("", "Packs")`, where the empty parent is the root itself.
pub fn split(relative: &str) -> (&str, &str) {
    match relative.rsplit_once('/') {
        Some((parent, name)) => (parent, name),
        None => ("", relative),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The paths the engine actually builds, turned into what a document tree
    /// can be walked with. `Vfs::mount` joins `Packs` onto the root and
    /// `save::slot_path` joins whatever `FILMENGINE.INI` says, so both shapes
    /// have to come out the same way.
    #[test]
    fn a_path_under_the_root_becomes_its_components() {
        let root = Path::new("/saf");
        assert_eq!(
            under(root, Path::new("/saf/Packs/System.GPK")).unwrap(),
            "Packs/System.GPK"
        );
        assert_eq!(under(root, Path::new("/saf/Save")).unwrap(), "Save");
        assert_eq!(under(root, Path::new("/saf")).unwrap(), "");
    }

    /// Nothing is above the granted folder, so nothing may climb out of it.
    #[test]
    fn climbing_out_of_the_root_is_not_a_path() {
        let root = Path::new("/saf");
        assert_eq!(
            under(root, Path::new("/saf/Packs/../Save")).unwrap(),
            "Packs/Save"
        );
        assert!(under(root, Path::new("/elsewhere/Packs")).is_err());
    }

    /// The final component is the name a provider is asked to create or
    /// rename, and the rest is the directory it goes in.
    #[test]
    fn a_relative_path_splits_at_its_last_separator() {
        assert_eq!(split("Save/SaveFile000.DAT"), ("Save", "SaveFile000.DAT"));
        assert_eq!(split("Config.DAT"), ("", "Config.DAT"));
        assert_eq!(split(""), ("", ""));
    }
}
