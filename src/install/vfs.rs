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
//!
//! # A pack is not one file
//!
//! `FUN_0043eee0` walks the route module's pack table and hands each name to
//! `FUN_0043ecf0`, which appends the engine INI's `[FileExtend]` (`".GPK"`)
//! and calls `FUN_004413c0`. That opens `[Directory]` + the name — `Packs\` —
//! and then layers `_GetPatchMax@0()` = 10 **patch overlays** over it, named by
//! `_SetPackName@16`'s `L"%s.%03d"` off the name that already carries the
//! extension:
//!
//! ```text
//! Packs/System.GPK        the base pack
//! Packs/System.GPK.000    overlay 0
//! ...
//! Packs/System.GPK.009    overlay 9
//! ```
//!
//! `FUN_00440940` searches them **highest first**: overlay 9 down to overlay 0,
//! and only then the base pack. So a later overlay shadows an earlier one and
//! any overlay shadows the base, which is what makes a patch a patch. The index
//! here is built in the opposite order — base, then 0 upward — so that the last
//! writer wins and the resulting map is the same.
//!
//! Neither retail install ships an overlay, so nothing in either one changes;
//! a patched or translated install is what this is for.

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
    #[error(transparent)]
    Binaries(#[from] super::binaries::Error),
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
    binaries: super::binaries::Binaries,
    archives: Vec<Archive>,
    /// The logical pack each archive is a layer of, e.g. `"System"` for both
    /// `System.GPK` and `System.GPK.003`.
    packs: Vec<String>,
    /// Lowercased `"pack/inner/path.ext"` -> handle.
    index: HashMap<String, Handle>,
}

impl Vfs {
    /// Mounts every `.GPK` under `<root>/Packs`.
    ///
    /// The decryption key is recovered from the game executable found in `root`.
    pub fn mount(root: impl AsRef<Path>) -> Result<Self, Error> {
        let root = root.as_ref().to_path_buf();
        let executable = super::binaries::find_executable(&root)?;
        let key = Key::from_executable(&executable)?;
        let binaries = super::binaries::Binaries::find(&root, &executable);
        let patches = patch_max(&binaries);

        let mut bases = pack_paths(&root)?;
        bases.sort();

        let mut archives = Vec::with_capacity(bases.len());
        let mut packs = Vec::with_capacity(bases.len());
        let mut index = HashMap::new();
        let mut overlaid = 0usize;
        for base in &bases {
            let name = pack_display_name(base);
            let pack = name.to_ascii_lowercase();
            // Base first, then overlay 0 upward, so the highest-numbered
            // overlay holding a path is the one left in the index.
            for layer in std::iter::once(base.clone()).chain(overlay_paths(base, patches)) {
                let archive = match Archive::open(&layer, &key) {
                    Ok(archive) => archive,
                    // A base pack that will not open is the install being
                    // broken; an overlay that will not open costs only itself.
                    Err(err) if layer != *base => {
                        log::warn!("{}: {err}; not layered over {name}", layer.display());
                        continue;
                    }
                    Err(err) => return Err(err.into()),
                };
                let a = archives.len();
                for (e, entry) in archive.entries().iter().enumerate() {
                    let logical = format!("{pack}/{}", entry.name.to_ascii_lowercase());
                    if index
                        .insert(
                            logical,
                            Handle {
                                archive: a,
                                entry: e,
                            },
                        )
                        .is_some()
                    {
                        overlaid += 1;
                    }
                }
                if layer != *base {
                    log::info!(
                        "{} layers {} entries over {name}",
                        layer.display(),
                        archive.entries().len()
                    );
                }
                archives.push(archive);
                packs.push(name.clone());
            }
        }

        log::info!(
            "mounted {} packs in {} files, {} entries from {} ({overlaid} replaced by an overlay)",
            bases.len(),
            archives.len(),
            index.len(),
            root.display()
        );
        Ok(Vfs {
            root,
            binaries,
            archives,
            packs,
            index,
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The game executable the archive key was recovered from.
    pub fn executable(&self) -> &Path {
        &self.binaries.executable
    }

    /// The three shipped binaries, classified while the key was being found.
    ///
    /// Mounting already has to read every `.dll` in the root to learn how many
    /// patch overlays a pack carries, so callers that want the menu or route
    /// module take that answer rather than classifying them a second time.
    pub fn binaries(&self) -> &super::binaries::Binaries {
        &self.binaries
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

    /// The pack a handle lives in, e.g. `"Movie00"`.
    ///
    /// The logical pack, not the file: a handle into `Movie00.GPK.002` is in
    /// `Movie00`. [`Vfs::layer_of`] is the file.
    pub fn pack_of(&self, handle: Handle) -> &str {
        &self.packs[handle.archive]
    }

    /// The pack file a handle was read out of — a base pack or one overlay.
    pub fn layer_of(&self, handle: Handle) -> &Path {
        self.archives[handle.archive].path()
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

    /// Every logical path with the handle it resolves to, unordered.
    ///
    /// The layered view: one entry per path the game can see, pointing at
    /// whichever layer won. For tooling and diagnostics.
    pub fn entries(&self) -> impl Iterator<Item = (&str, Handle)> {
        self.index.iter().map(|(k, &h)| (k.as_str(), h))
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

/// How many overlays a pack may carry, out of the player's own route module.
///
/// Not written down here: `_GetPatchMax@0` is a constant return, and reading
/// it is what keeps this right on a build whose answer is not ten. A route
/// module that cannot be found or read costs the overlays and nothing else,
/// which is no loss on an install that ships none.
fn patch_max(binaries: &super::binaries::Binaries) -> u32 {
    let bytes = binaries.route_bytes();
    let found = days_route::pe::Image::parse(&bytes)
        .ok()
        .and_then(|image| image.patch_max());
    match found {
        Some(max) => max,
        None => {
            log::warn!("no _GetPatchMax@0 in the route module; pack overlays will be skipped");
            0
        }
    }
}

/// The overlay files layered over `base`, lowest first, that actually exist.
///
/// The original probes all ten names and tolerates every miss — School Days
/// HQ's pack table names a `Commentary` pack the retail install does not ship,
/// and the game runs — so a name with no file behind it is simply not a layer.
fn overlay_paths(base: &Path, patches: u32) -> impl Iterator<Item = PathBuf> {
    let base = base.to_path_buf();
    (0..patches)
        .map(move |i| overlay_path(&base, i))
        .filter(|p| p.is_file())
}

/// Where overlay `i` of `base` would live, whether or not it is there.
///
/// `_SetPackName@16` formats `L"%s.%03d"` off a name that already carries the
/// `[FileExtend]`, so this is `Packs/System.GPK.000` and not
/// `Packs/System.000`.
fn overlay_path(base: &Path, i: u32) -> PathBuf {
    let mut name = base.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".{i:03}"));
    base.with_file_name(name)
}

/// A pack file's name without the `[FileExtend]`: `System`, `System.000`.
pub fn pack_display_name(path: &Path) -> String {
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    match name.to_ascii_lowercase().find(".gpk") {
        Some(at) => format!("{}{}", &name[..at], &name[at + 4..]),
        None => name.into_owned(),
    }
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

    /// `_SetPackName@16` numbers the name the base pack was opened under,
    /// which already carries the `[FileExtend]`.
    #[test]
    fn an_overlay_is_numbered_after_the_extension_not_before_it() {
        let base = Path::new("/game/Packs/System.GPK");
        assert_eq!(
            overlay_path(base, 0),
            Path::new("/game/Packs/System.GPK.000")
        );
        assert_eq!(
            overlay_path(base, 9),
            Path::new("/game/Packs/System.GPK.009")
        );
        assert_eq!(pack_display_name(&overlay_path(base, 3)), "System.003");
        assert_eq!(pack_display_name(base), "System");
    }

    /// Asset directories contain dots, so only the final component counts.
    #[test]
    fn extension_check_ignores_directory_dots() {
        assert!(!has_extension("movie00/00-00/00-00-a00/00-00-a00-000"));
        assert!(has_extension("movie00/00-00/00-00-a00/00-00-a00-000.wmv"));
        assert!(!has_extension("script/english/00/00-00-a00"));
    }
}
