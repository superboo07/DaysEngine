//! Finding the three shipped binaries an install is built out of, without
//! knowing what they are called.
//!
//! The engine needs bytes from three files that live beside the packs: the
//! executable (for the archive key and the comment dialog template), the menu
//! module (for every widget table) and the route module (for the branch
//! graph). Their retail names differ between titles on the same engine —
//! `SCHOOLDAYS HQ.exe` / `SysMenuSDHQ.dll` / `RouteProcSDHQ.dll` for School
//! Days HQ, `SHINYDAYS.exe` / `SysMenuSD.dll` / `RouteProcSD.dll` for Shiny
//! Days — and localised and repackaged installs rename them again.
//!
//! So none of them is found by name. Each is found by what it **is**:
//!
//! * the executable carries a `CODE` / `CIPHERCODE` resource, which is what
//!   [`days_gpk::Key::from_executable`] already looked for,
//! * the menu module exports [`MENU_EXPORTS`],
//! * the route module exports [`ROUTE_EXPORTS`].
//!
//! Those export names are the module's published interface to the executable,
//! so they are the part least free to drift: the two titles' menu modules
//! differ by two exports out of forty and agree on every name below.
//!
//! # Provenance
//!
//! The export sets were read out of both titles' import and export
//! directories. School Days HQ's `SCHOOLDAYS HQ.exe` imports `SysMenuSDHQ.dll`
//! and `RouteProcSDHQ.dll`; Shiny Days' `SHINYDAYS.exe` imports `SysMenuSD.dll`
//! and `RouteProcSD.dll`. `SysMenuSD.dll` adds `_GetBGMVolume@0` and
//! `_GetSEVolume@0` to what `SysMenuSDHQ.dll` exports and drops nothing;
//! `RouteProcSD.dll` adds `_ChangeSubtitle@4`, `_CheckEndRollSelect@8`,
//! `_CheckEndRollView@4` and `_CheckUniformBlock@4` and drops nothing.

use super::storage;
use days_gpk::Key;
use days_route::pe::Image;
use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("no game executable with a CIPHERCODE resource in {0}")]
    NoExecutable(PathBuf),
}

/// Exports that identify the menu module.
///
/// `_SystemInit@8` is the screen state machine the whole menu layer is, and
/// `_SystemMenuInit@4` is how the executable hands it the host object. No
/// other module in either install exports either name.
pub const MENU_EXPORTS: [&str; 2] = ["_SystemInit@8", "_SystemMenuInit@4"];

/// Exports that identify the route module.
///
/// `_GetNextScriptFile@12` is the branch decision itself and `_CheckScript@8`
/// the flag test behind it.
pub const ROUTE_EXPORTS: [&str; 2] = ["_GetNextScriptFile@12", "_CheckScript@8"];

/// The binaries found in one install root.
///
/// Only the executable is required: a missing menu or route module costs the
/// menus or the branch graph, and the engine says so and plays on. That is the
/// same tolerance the rest of the install layer has for a missing asset.
#[derive(Debug, Clone)]
pub struct Binaries {
    /// The game executable, found by its `CIPHERCODE` resource.
    pub executable: PathBuf,
    /// The menu module, which holds every screen's widget table.
    pub menu: Option<PathBuf>,
    /// The route module, which holds the branch graph.
    pub route: Option<PathBuf>,
}

impl Binaries {
    /// Classifies every `.exe` and `.dll` in `root`, finding the executable too.
    ///
    /// The executable is the one carrying a `CODE` / `CIPHERCODE` resource,
    /// which is the same test [`days_gpk::Key::from_executable`] applies: an
    /// install has exactly one, because it is the only file the archive key
    /// could have come from.
    pub fn discover(root: &Path) -> Result<Self, Error> {
        Ok(Self::find(root, &find_executable(root)?))
    }

    /// Classifies every `.exe` and `.dll` in `root`.
    ///
    /// `executable` is passed in rather than searched for again, because the
    /// [`Vfs`](super::vfs::Vfs) has already had to find it to get the archive
    /// key and finding it twice means reading it twice.
    pub fn find(root: &Path, executable: &Path) -> Self {
        let mut menu = None;
        let mut route = None;
        for path in dlls(root) {
            let Ok(bytes) = storage::read(&path) else {
                continue;
            };
            let Ok(image) = Image::parse(&bytes) else {
                continue;
            };
            let has = |names: &[&str]| names.iter().all(|n| image.exports.contains_key(*n));
            if menu.is_none() && has(&MENU_EXPORTS) {
                menu = Some(path);
            } else if route.is_none() && has(&ROUTE_EXPORTS) {
                route = Some(path);
            }
        }
        if menu.is_none() {
            log::warn!(
                "no module in {} exports {}; the menus need it and will be skipped",
                root.display(),
                MENU_EXPORTS.join(" and "),
            );
        }
        if route.is_none() {
            log::warn!(
                "no module in {} exports {}; the branch graph is unavailable, \
                 so scripts will not chain",
                root.display(),
                ROUTE_EXPORTS.join(" and "),
            );
        }
        Binaries {
            executable: executable.to_path_buf(),
            menu,
            route,
        }
    }

    /// The menu module's bytes, or an empty vector when there is none.
    ///
    /// Empty rather than an error because that is what every caller wants: the
    /// widget tables are only needed for menus, and a screen that cannot find
    /// its table already reports itself unavailable.
    pub fn menu_bytes(&self) -> Vec<u8> {
        read_or_warn(self.menu.as_deref(), "the widget tables live in it")
    }

    /// The route module's bytes, or an empty vector when there is none.
    pub fn route_bytes(&self) -> Vec<u8> {
        read_or_warn(
            self.route.as_deref(),
            "the branch graph's script tables live in it",
        )
    }
}

fn read_or_warn(path: Option<&Path>, why: &str) -> Vec<u8> {
    let Some(path) = path else {
        return Vec::new();
    };
    match storage::read(path) {
        Ok(bytes) => bytes,
        Err(err) => {
            log::warn!("reading {} — {why}: {err}", path.display());
            Vec::new()
        }
    }
}

/// Every `.dll` directly in `root`, in a stable order.
///
/// Sorted so that two installs holding the same files classify the same way
/// whatever order the filesystem hands them back in.
fn dlls(root: &Path) -> Vec<PathBuf> {
    let Ok(read) = storage::read_dir(root) else {
        return Vec::new();
    };
    let mut out: Vec<PathBuf> = read
        .into_iter()
        .map(|e| e.path)
        .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("dll")))
        .collect();
    out.sort();
    out
}

/// Finds the game executable by looking for one carrying the archive key.
///
/// Matching on a retail filename would break on every localised and
/// repackaged install, and on the other titles on this engine, so the test is
/// the resource rather than the name.
pub fn find_executable(root: &Path) -> Result<PathBuf, Error> {
    let Ok(read) = storage::read_dir(root) else {
        return Err(Error::NoExecutable(root.to_path_buf()));
    };
    let mut exes: Vec<PathBuf> = read
        .into_iter()
        .map(|e| e.path)
        .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("exe")))
        .collect();
    exes.sort();
    exes.into_iter()
        .find(|p| storage::read(p).is_ok_and(|bytes| Key::from_image(&bytes).is_ok()))
        .ok_or_else(|| Error::NoExecutable(root.to_path_buf()))
}
