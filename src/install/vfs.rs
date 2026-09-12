//! A read-only virtual filesystem over an installed game's `.GPK` packs.
//!
//! Scripts and `.INI` files refer to assets by logical path, where the **first
//! component is the pack name** and the rest is the path inside it:
//!
//! ```text
//! Movie00/00-00/00-00-A00/00-00-A00-000   ->  Movie00.GPK : 00-00/00-00-A00/00-00-A00-000.WMV
//! BGM/SD_BGM/sdbgm07                      ->  BGM.GPK     : SD_BGM/SDBGM07.OGG
//! System/Title/TitleBase.png              ->  System.GPK  : TITLE/TITLEBASE.PNG
//! ```
//!
//! Two wrinkles the format forces on us:
//!
//! * **Extensions are usually omitted.** A script says `.../00-00-A00-000` and
//!   the engine is expected to know it means `.WMV` because it appeared in a
//!   `[PlayMovie]`. Rather than thread that context through, [`Vfs::resolve`]
//!   tries the known media extensions in turn, and callers that do know can use
//!   [`Vfs::resolve_as`] to pin it.
//! * **Case is inconsistent.** Pack indices are mostly uppercase while the
//!   `.INI` files use mixed case. Every lookup here is case-insensitive.

use days_gpk::{Archive, Entry, Key};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Extensions tried, in order, when a logical path carries none.
///
/// Ordered by how often each appears in the packs so the common case hits first.
const IMPLICIT_EXTENSIONS: &[&str] = &["png", "ogg", "wmv", "ors", "cmap", "dat", "ini"];

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("no Packs directory in {0}")]
    NoPacksDir(PathBuf),
    #[error("no .GPK archives in {0}")]
    NoPacks(PathBuf),
    #[error("no game executable with a CIPHERCODE resource in {0}")]
    NoExecutable(PathBuf),
    #[error("asset not found: {0}")]
    NotFound(String),
    #[error(transparent)]
    Gpk(#[from] days_gpk::Error),
    #[error("i/o error on {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// Points at one entry in one pack.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Handle {
    archive: usize,
    entry: usize,
}

/// A background music track: an optional one-shot intro followed by a part that
/// repeats until the track changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bgm {
    pub intro: Option<Handle>,
    pub looped: Handle,
}

/// All of a game installation's packs, mounted as one namespace.
pub struct Vfs {
    root: PathBuf,
    executable: PathBuf,
    archives: Vec<Archive>,
    /// Lowercased `"pack/inner/path.ext"` -> handle.
    index: HashMap<String, Handle>,
}

impl Vfs {
    /// Mounts every `.GPK` under `<root>/Packs`.
    ///
    /// The decryption key is recovered from the game executable found in `root`.
    pub fn mount(root: impl AsRef<Path>) -> Result<Self, Error> {
        let root = root.as_ref().to_path_buf();
        let executable = find_executable(&root)?;
        let key = Key::from_executable(&executable)?;

        let mut paths = pack_paths(&root)?;
        paths.sort();

        let mut archives = Vec::with_capacity(paths.len());
        let mut index = HashMap::new();
        for path in &paths {
            let pack = path
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .to_ascii_lowercase();
            let archive = Archive::open(path, &key)?;
            let a = archives.len();
            for (e, entry) in archive.entries().iter().enumerate() {
                let logical = format!("{pack}/{}", entry.name.to_ascii_lowercase());
                // Packs do not overlap in practice; if one ever does, first wins
                // and we say so rather than silently shadowing.
                if let Some(prev) = index.insert(
                    logical.clone(),
                    Handle {
                        archive: a,
                        entry: e,
                    },
                ) {
                    log::warn!("{logical} appears in more than one pack; shadowing {prev:?}");
                }
            }
            archives.push(archive);
        }

        log::info!(
            "mounted {} packs, {} entries from {}",
            archives.len(),
            index.len(),
            root.display()
        );
        Ok(Vfs {
            root,
            executable,
            archives,
            index,
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The game executable the archive key was recovered from.
    pub fn executable(&self) -> &Path {
        &self.executable
    }

    pub fn len(&self) -> usize {
        self.index.len()
    }

    pub fn is_empty(&self) -> bool {
        self.index.is_empty()
    }

    /// Resolves a logical path, trying the known extensions if it carries none.
    pub fn resolve(&self, logical: &str) -> Option<Handle> {
        let key = normalise(logical);
        if let Some(&h) = self.index.get(&key) {
            return Some(h);
        }
        // Only guess when there is no extension at all. A path that already ends
        // in ".png" and is missing should stay missing rather than silently
        // resolving to a same-named .ogg.
        if has_extension(&key) {
            return None;
        }
        IMPLICIT_EXTENSIONS
            .iter()
            .find_map(|ext| self.index.get(&format!("{key}.{ext}")).copied())
    }

    /// Resolves a logical path with a known extension, appending it if absent.
    pub fn resolve_as(&self, logical: &str, ext: &str) -> Option<Handle> {
        let key = normalise(logical);
        if let Some(&h) = self.index.get(&key) {
            return Some(h);
        }
        self.index
            .get(&format!("{key}.{}", ext.to_ascii_lowercase()))
            .copied()
    }

    pub fn exists(&self, logical: &str) -> bool {
        self.resolve(logical).is_some()
    }

    /// Reads and decodes the entry behind a handle.
    pub fn read(&self, handle: Handle) -> Result<Vec<u8>, Error> {
        let archive = &self.archives[handle.archive];
        Ok(archive.read(&archive.entries()[handle.entry])?)
    }

    /// Resolve and read in one step.
    pub fn read_path(&self, logical: &str) -> Result<Vec<u8>, Error> {
        let h = self
            .resolve(logical)
            .ok_or_else(|| Error::NotFound(logical.to_string()))?;
        self.read(h)
    }

    /// Resolve with a known extension and read in one step.
    pub fn read_path_as(&self, logical: &str, ext: &str) -> Result<Vec<u8>, Error> {
        let h = self
            .resolve_as(logical, ext)
            .ok_or_else(|| Error::NotFound(format!("{logical}.{ext}")))?;
        self.read(h)
    }

    pub fn entry(&self, handle: Handle) -> &Entry {
        &self.archives[handle.archive].entries()[handle.entry]
    }

    /// The pack a handle lives in, e.g. `"movie00"`.
    pub fn pack_of(&self, handle: Handle) -> &str {
        self.archives[handle.archive]
            .path()
            .file_stem()
            .map(|s| s.to_str().unwrap_or_default())
            .unwrap_or_default()
    }

    /// Resolves a `[PlayBgm]` path into its intro and loop halves.
    ///
    /// Background music is not a single file. A script asks for
    /// `BGM/SD_BGM/sdbgm07` and the pack holds `SDBGM07_INT.OGG` and
    /// `SDBGM07_LOOP.OGG`: the intro plays once, then the loop repeats until
    /// the track is replaced. Nine tracks (`sdbgm14`, `18`, `20`, `28`, `29`,
    /// `31`, `32`, `33`) ship loop-only and start straight into the loop.
    ///
    /// Vocal tracks (`BGM/Vocal/SDV01`) are plain single files and are played
    /// through `[PlaySe]`, not here, so they resolve as a lone intro.
    pub fn resolve_bgm(&self, logical: &str) -> Option<Bgm> {
        let intro = self.resolve_as(&format!("{logical}_INT"), "ogg");
        let looped = self.resolve_as(&format!("{logical}_LOOP"), "ogg");
        match (intro, looped) {
            (intro, Some(looped)) => Some(Bgm { intro, looped }),
            // No loop half: either a vocal track or a plain one-shot.
            (None, None) => self.resolve_as(logical, "ogg").map(|h| Bgm {
                intro: Some(h),
                looped: h,
            }),
            (Some(intro), None) => Some(Bgm {
                intro: None,
                looped: intro,
            }),
        }
    }

    /// Every logical path, unordered. For tooling and diagnostics.
    pub fn paths(&self) -> impl Iterator<Item = &str> {
        self.index.keys().map(String::as_str)
    }
}

fn normalise(logical: &str) -> String {
    logical
        .replace('\\', "/")
        .trim_start_matches('/')
        .to_ascii_lowercase()
}

/// True if the path's last component has an extension.
///
/// Checked against the final component only: asset paths contain `.` in
/// directory names (`00-00-A00`), and treating those as extensions would
/// disable the implicit-extension search for most of the game.
fn has_extension(path: &str) -> bool {
    path.rsplit('/')
        .next()
        .is_some_and(|last| last.contains('.'))
}

fn pack_paths(root: &Path) -> Result<Vec<PathBuf>, Error> {
    let dir = root.join("Packs");
    let read = std::fs::read_dir(&dir).map_err(|source| {
        if source.kind() == std::io::ErrorKind::NotFound {
            Error::NoPacksDir(root.to_path_buf())
        } else {
            Error::Io {
                path: dir.clone(),
                source,
            }
        }
    })?;
    let mut out = Vec::new();
    for entry in read {
        let path = entry
            .map_err(|source| Error::Io {
                path: dir.clone(),
                source,
            })?
            .path();
        if path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("gpk"))
        {
            out.push(path);
        }
    }
    if out.is_empty() {
        return Err(Error::NoPacks(dir));
    }
    Ok(out)
}

/// Finds the game executable by looking for one carrying the archive key.
///
/// Matching on the retail filename alone breaks on localised and repackaged
/// installs, which rename it.
fn find_executable(root: &Path) -> Result<PathBuf, Error> {
    let preferred = root.join("SCHOOLDAYS HQ.exe");
    if preferred.is_file() && Key::from_executable(&preferred).is_ok() {
        return Ok(preferred);
    }
    let read = std::fs::read_dir(root).map_err(|source| Error::Io {
        path: root.to_path_buf(),
        source,
    })?;
    for entry in read.flatten() {
        let path = entry.path();
        if path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("exe"))
            && Key::from_executable(&path).is_ok()
        {
            return Ok(path);
        }
    }
    Err(Error::NoExecutable(root.to_path_buf()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalises_separators_and_case() {
        assert_eq!(
            normalise("System\\Title\\TitleBase.png"),
            "system/title/titlebase.png"
        );
        assert_eq!(normalise("/BGM/SD_BGM/sdbgm07"), "bgm/sd_bgm/sdbgm07");
    }

    /// Asset directories contain dots, so only the final component counts.
    #[test]
    fn extension_check_ignores_directory_dots() {
        assert!(!has_extension("movie00/00-00/00-00-a00/00-00-a00-000"));
        assert!(has_extension("movie00/00-00/00-00-a00/00-00-a00-000.wmv"));
        assert!(!has_extension("script/english/00/00-00-a00"));
    }
}
