//! The branch graph's script-name tables, out of the player's own
//! `RouteProcSDHQ.dll`.
//!
//! # What the route system is
//!
//! `RouteProcSDHQ.dll` is a third shipped binary beside the executable and the
//! menu DLL, and it owns every branching decision in the game. The executable
//! does not decide anything either: its timeline-move state
//! (`FUN_00425bf0` case 7) calls `_GetNextScriptFile@12(engine + 0x2c, buf,
//! 0x104)` and plays whatever name comes back.
//!
//! Progress is two integers in the save's variable store, `ROUTE` and `SCENE`:
//!
//! ```text
//! ROUTE   0 .. 0x36      which of the 55 chapters/branches the player is on
//! SCENE   an index into that route's script-name table
//! ```
//!
//! Three of the DLL's exports are 55-way switches on `ROUTE`, one case per
//! route:
//!
//! ```text
//! GetNextScriptFile   scene state machine: (SCENE, choice) -> next SCENE
//! SetFeeling          credits the feeling deltas for the scene just chosen
//! searchRoot          finds the (ROUTE, SCENE) a script name sits at
//! ```
//!
//! The first two are recovered here by decoding them; the third is reproduced
//! by [`Routes::find_from`] searching the tables in the same order.
//!
//! and a fourth, `_SetScript@16`, is a plain `table[ROUTE][SCENE]` lookup.
//!
//! # What this crate recovers
//!
//! The **name tables** are data: 55 arrays of pointers to wide strings in
//! `.rdata`, one array per route, indexed by `SCENE`. Those are what
//! [`Routes::recover`] reads, and they are enough to say what every route
//! contains, to map a script back to its `(ROUTE, SCENE)` the way `searchRoot`
//! does, and to resolve a scene the way `_SetScript@16` does.
//!
//! The **transition logic** is not data at all: each route's
//! `GetNextScriptFile` case is a compiled `switch (SCENE)` whose arms call an
//! emitter with a literal next-scene number, so the edges exist only as x86.
//! [`Machine`] recovers them by decoding those 55 functions out of the same
//! DLL — see [`graph`] for how, and for the checks it was held to.
//!
//! # Finding the tables without hardcoding an address
//!
//! We do not embed the table addresses — they are the user's game data, and a
//! hardcoded RVA silently breaks on a different build. The tables are found by
//! **content**: a run of consecutive 4-byte values, each of which resolves
//! through the PE section table to a NUL-terminated UTF-16LE string shaped
//! like a script path (`NN/NN-XX-Ynn`, as in `00/00-00-A00`). A maximal such
//! run is one route's table, and the runs in address order are routes 0 upward.
//!
//! That is self-validating, and it was checked against the code three ways
//! before being trusted:
//!
//! - The 55 `searchRoot` handlers each reference exactly one `.data` address.
//!   Those 55 addresses are **exactly** the 55 run starts this scan finds — no
//!   extras, no misses.
//! - `_SetScript@16`'s own 55-way switch indexes the same 55 addresses.
//! - Route 0's handler (`FUN_1000de70`) bounds its loop at `< 0x15`, and the
//!   run at that address is 21 entries long.
//!
//! The recovered names also account for the script pack exactly: 1,857 names
//! against 1,857 `.ORS` files, agreeing on 1,855 of them. The four that differ
//! are shipped facts rather than recovery errors, and [`Routes::recover`]
//! keeps them — see [`Routes`].
//!
//! Everything above was transcribed from the decompiled DLL rather than
//! inferred from the bytes.

#![forbid(unsafe_code)]

pub mod graph;
mod pe;
mod walk;
mod x86;

pub use graph::{Act, Cmp, Context, Crediting, Machine, Next, Step, Term, Transition};

use pe::{u16le, u32le, Section};

/// What went wrong recovering the route system.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("not a PE image: {0}")]
    NotPe(&'static str),
    #[error("no route tables found in the DLL")]
    NoTables,
    #[error("the DLL exports no {0}")]
    NoExport(&'static str),
    #[error("GetNextScriptFile does not dispatch the way the retail build does")]
    NoDispatch,
}

/// The image base and section table of a 32-bit PE.
fn sections(dll: &[u8]) -> Result<(u32, Vec<Section>), Error> {
    let img = pe::Image::parse(dll)?;
    Ok((img.base, img.sections))
}

/// Where a virtual address lands in the file, if it is backed by file bytes.
fn to_file(secs: &[Section], va: u32) -> Option<usize> {
    secs.iter().find_map(|s| {
        let off = va.checked_sub(s.va)?;
        (off < s.vsize && off < s.rsize).then_some((s.raw + off) as usize)
    })
}

/// The longest a script path is allowed to run before we stop believing it is
/// one. The real names are twelve characters; this only has to exclude prose.
const MAX_NAME: usize = 32;

/// A NUL-terminated UTF-16LE string at a file offset, if it is printable ASCII.
///
/// The tables point only at script paths, which are ASCII. Refusing anything
/// else is what keeps the neighbouring arrays of narrow strings — the same
/// names again, in 8-bit — from being mistaken for table entries.
fn wide_ascii(dll: &[u8], at: usize) -> Option<String> {
    let mut out = String::new();
    let mut o = at;
    loop {
        let u = u16le(dll, o)?;
        if u == 0 {
            return Some(out);
        }
        if !(0x20..0x7f).contains(&u) || out.len() == MAX_NAME {
            return None;
        }
        out.push(u as u8 as char);
        o += 2;
    }
}

/// Whether a string is shaped like one of the game's script paths.
///
/// `00/00-00-A00`: a two-character chapter directory, then the same chapter,
/// a two-character route tag and a scene tag. Checked by shape rather than
/// against the pack, because two of the names in the tables do not ship.
fn is_script_path(s: &str) -> bool {
    let b = s.as_bytes();
    let alnum_upper = |c: u8| c.is_ascii_digit() || c.is_ascii_uppercase();
    b.len() >= 12
        && alnum_upper(b[0])
        && alnum_upper(b[1])
        && b[2] == b'/'
        && alnum_upper(b[3])
        && alnum_upper(b[4])
        && b[5] == b'-'
        && alnum_upper(b[6])
        && alnum_upper(b[7])
        && b[8] == b'-'
        && b[9..].iter().all(|&c| alnum_upper(c))
}

/// Every route's table of script names, indexed by `ROUTE` then `SCENE`.
///
/// Recovered from the player's own `RouteProcSDHQ.dll`; nothing here ships
/// with the engine.
///
/// Two entries in the retail tables name scripts that the retail packs do not
/// contain (`03/03-B2-A00` and `03/03-KB-E00` — both sequences start at `A01`
/// and `E01`), and two shipped scripts are named by no table at all
/// (`01/01-00-OP2` and `05/05-9O-B00`). Those are shipped facts. The dangling
/// two are kept in the table rather than dropped, so that scene numbering
/// stays the game's own; reaching one is a missing asset, which the engine
/// logs and continues past.
#[derive(Debug, Clone)]
pub struct Routes {
    routes: Vec<Vec<String>>,
}

impl Routes {
    /// Recovers every route's table from the bytes of `RouteProcSDHQ.dll`.
    ///
    /// See the module documentation for how the tables are identified and how
    /// that was checked against the code.
    pub fn recover(dll: &[u8]) -> Result<Routes, Error> {
        let (_, secs) = sections(dll)?;

        // Only initialised, file-backed data can hold a pointer table, so the
        // scan is over the sections rather than the whole file: a run must not
        // be allowed to straddle a section boundary.
        let mut routes: Vec<Vec<String>> = Vec::new();
        for s in &secs {
            let end = s.rsize.min(s.vsize);
            let mut run: Vec<String> = Vec::new();
            let mut off = 0u32;
            while off + 4 <= end {
                let name = u32le(dll, (s.raw + off) as usize)
                    .and_then(|va| to_file(&secs, va))
                    .and_then(|at| wide_ascii(dll, at))
                    .filter(|n| is_script_path(n));
                match name {
                    Some(n) => run.push(n),
                    None => {
                        if !run.is_empty() {
                            routes.push(std::mem::take(&mut run));
                        }
                    }
                }
                off += 4;
            }
            if !run.is_empty() {
                routes.push(run);
            }
        }

        if routes.is_empty() {
            return Err(Error::NoTables);
        }
        log::debug!(
            "recovered {} route tables, {} scripts",
            routes.len(),
            routes.iter().map(Vec::len).sum::<usize>()
        );
        Ok(Routes { routes })
    }

    /// How many routes there are. The retail DLL has 55, numbered `0..=0x36`.
    pub fn len(&self) -> usize {
        self.routes.len()
    }

    /// Whether no route was recovered. Never true for a store from
    /// [`Routes::recover`], which refuses an empty result.
    pub fn is_empty(&self) -> bool {
        self.routes.is_empty()
    }

    /// One route's scripts, in `SCENE` order.
    pub fn route(&self, route: usize) -> Option<&[String]> {
        self.routes.get(route).map(Vec::as_slice)
    }

    /// The script at a `(ROUTE, SCENE)`, which is what `_SetScript@16` does.
    pub fn script(&self, route: usize, scene: usize) -> Option<&str> {
        self.routes.get(route)?.get(scene).map(String::as_str)
    }

    /// Where a script name sits, as `searchRoot` answers it.
    ///
    /// `searchRoot` takes a route to start from and walks forward, wrapping at
    /// the last route back to route 0, until a route's table contains the
    /// name. Searching from `from` reproduces that order, so a name that
    /// appears in more than one table resolves to the same one the game picks.
    ///
    /// The shipped loop has no termination condition for a name that is in no
    /// table: the wrap keeps the route number in range, so the `default` arm
    /// that would bail out is unreachable and the call spins forever. We
    /// return `None` after one full pass instead of reproducing the hang.
    pub fn find_from(&self, script: &str, from: usize) -> Option<(usize, usize)> {
        if self.routes.is_empty() {
            return None;
        }
        for i in 0..self.routes.len() {
            let route = (from + i) % self.routes.len();
            if let Some(scene) = self.routes[route].iter().position(|s| s == script) {
                return Some((route, scene));
            }
        }
        None
    }

    /// Where a script name sits, searching from route 0.
    pub fn find(&self, script: &str) -> Option<(usize, usize)> {
        self.find_from(script, 0)
    }

    /// Every `(route, scene, script)` in table order.
    pub fn iter(&self) -> impl Iterator<Item = (usize, usize, &str)> {
        self.routes.iter().enumerate().flat_map(|(r, scenes)| {
            scenes
                .iter()
                .enumerate()
                .map(move |(s, name)| (r, s, name.as_str()))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a PE32 image with one section whose bytes are `data`, mapped at
    /// `va`. Enough of a header for [`sections`] and no more.
    fn image(va: u32, data: &[u8]) -> Vec<u8> {
        const PE: usize = 0x80;
        const OPT: usize = 0xe0;
        const RAW: u32 = 0x400;
        let mut v = vec![0u8; RAW as usize];
        v[0..2].copy_from_slice(b"MZ");
        v[0x3c..0x40].copy_from_slice(&(PE as u32).to_le_bytes());
        v[PE..PE + 4].copy_from_slice(b"PE\0\0");
        v[PE + 6..PE + 8].copy_from_slice(&1u16.to_le_bytes()); // one section
        v[PE + 20..PE + 22].copy_from_slice(&(OPT as u16).to_le_bytes());
        v[PE + 24..PE + 26].copy_from_slice(&0x10bu16.to_le_bytes()); // PE32
        v[PE + 24 + 28..PE + 24 + 32].copy_from_slice(&0u32.to_le_bytes()); // image base 0
        let h = PE + 24 + OPT;
        v[h + 8..h + 12].copy_from_slice(&(data.len() as u32).to_le_bytes()); // vsize
        v[h + 12..h + 16].copy_from_slice(&va.to_le_bytes()); // rva
        v[h + 16..h + 20].copy_from_slice(&(data.len() as u32).to_le_bytes()); // rsize
        v[h + 20..h + 24].copy_from_slice(&RAW.to_le_bytes()); // raw
        v.extend_from_slice(data);
        v
    }

    fn wide(s: &str) -> Vec<u8> {
        let mut v: Vec<u8> = s.encode_utf16().flat_map(u16::to_le_bytes).collect();
        v.extend_from_slice(&[0, 0]);
        v
    }

    /// Lays out `tables` as the DLL does: the strings first, then one pointer
    /// array per table, separated by a value that is not a pointer to a name.
    fn dll(tables: &[&[&str]]) -> Vec<u8> {
        let va = 0x1000u32;
        let mut blob = Vec::new();
        let mut at = Vec::new();
        for t in tables {
            let mut row = Vec::new();
            for name in *t {
                row.push(va + blob.len() as u32);
                blob.extend_from_slice(&wide(name));
            }
            at.push(row);
        }
        // Pointer arrays follow the strings, each separated by a gap so that
        // neighbouring tables do not merge into one run. The compiler aligns
        // them, and the scan steps four bytes at a time, so the fixture has to
        // align them too.
        while blob.len() % 4 != 0 {
            blob.push(0);
        }
        for row in &at {
            for p in row {
                blob.extend_from_slice(&p.to_le_bytes());
            }
            blob.extend_from_slice(&0u32.to_le_bytes());
        }
        image(va, &blob)
    }

    #[test]
    fn each_run_of_script_name_pointers_is_one_route() {
        let r = Routes::recover(&dll(&[
            &["00/00-00-A00", "00/00-00-A01"],
            &["01/01-1K-A00", "01/01-1K-B00", "01/01-1K-C00"],
        ]))
        .unwrap();
        assert_eq!(r.len(), 2);
        assert_eq!(r.route(0).unwrap().len(), 2);
        assert_eq!(r.route(1).unwrap().len(), 3);
    }

    #[test]
    fn a_scene_resolves_the_way_setscript_does() {
        let r = Routes::recover(&dll(&[&["00/00-00-A00", "00/00-00-A01"]])).unwrap();
        assert_eq!(r.script(0, 1), Some("00/00-00-A01"));
        assert_eq!(r.script(0, 2), None);
        assert_eq!(r.script(9, 0), None);
    }

    #[test]
    fn a_name_resolves_to_its_route_and_scene() {
        let r = Routes::recover(&dll(&[
            &["00/00-00-A00", "00/00-00-A01"],
            &["01/01-1K-A00", "01/01-1K-B00"],
        ]))
        .unwrap();
        assert_eq!(r.find("01/01-1K-B00"), Some((1, 1)));
        assert_eq!(r.find("05/05-KK-Z99"), None);
    }

    /// `searchRoot` starts at the route it is given and wraps, so the same
    /// name in two tables resolves to whichever comes first from there.
    #[test]
    fn the_search_starts_where_it_is_told_and_wraps() {
        let shared = "03/03-KA-A00";
        let r = Routes::recover(&dll(&[
            &["00/00-00-A00", shared],
            &["01/01-1K-A00", "01/01-1K-B00"],
            &[shared, "02/02-2K-B00"],
        ]))
        .unwrap();
        assert_eq!(r.find_from(shared, 0), Some((0, 1)));
        assert_eq!(r.find_from(shared, 1), Some((2, 0)));
        // From the last route the search wraps back round to route 0.
        assert_eq!(r.find_from(shared, 2), Some((2, 0)));
        assert_eq!(r.find_from("01/01-1K-B00", 2), Some((1, 1)));
    }

    /// A name in no table returns rather than spinning, which is where we
    /// deliberately differ from the shipped loop.
    #[test]
    fn an_unknown_name_terminates_instead_of_wrapping_forever() {
        let r = Routes::recover(&dll(&[&["00/00-00-A00"]])).unwrap();
        assert_eq!(r.find_from("99/99-ZZ-Z99", 0), None);
    }

    /// The narrow-string copies of the same names sit right beside the tables
    /// in the real DLL. Read as UTF-16 they decode to non-ASCII, which is what
    /// keeps them out of the runs.
    #[test]
    fn narrow_string_pointers_do_not_join_a_run() {
        let va = 0x1000u32;
        let mut blob = Vec::new();
        let wide_at = va;
        blob.extend_from_slice(&wide("00/00-00-A00"));
        let narrow_at = va + blob.len() as u32;
        blob.extend_from_slice(b"00-00-A00\0");
        blob.extend_from_slice(&wide_at.to_le_bytes());
        blob.extend_from_slice(&narrow_at.to_le_bytes());
        let r = Routes::recover(&image(va, &blob)).unwrap();
        assert_eq!(r.len(), 1);
        assert_eq!(r.route(0).unwrap(), &["00/00-00-A00"]);
    }

    #[test]
    fn a_dll_with_no_tables_is_an_error_rather_than_an_empty_set() {
        let blob = wide("not a script name at all");
        assert!(matches!(
            Routes::recover(&image(0x1000, &blob)),
            Err(Error::NoTables)
        ));
    }

    #[test]
    fn something_that_is_not_a_pe_is_refused() {
        assert!(matches!(
            Routes::recover(b"this is not a DLL"),
            Err(Error::NotPe(_))
        ));
    }

    /// A virtual address past what the file holds has no bytes behind it and
    /// must not be resolved into the next section.
    #[test]
    fn an_address_beyond_the_file_backed_part_does_not_resolve() {
        let secs = [Section {
            va: 0x1000,
            vsize: 0x2000,
            raw: 0x400,
            rsize: 0x100,
        }];
        assert_eq!(to_file(&secs, 0x1000), Some(0x400));
        assert_eq!(to_file(&secs, 0x10ff), Some(0x4ff));
        assert_eq!(to_file(&secs, 0x1100), None);
        assert_eq!(to_file(&secs, 0x0fff), None);
    }

    #[test]
    fn the_shape_test_accepts_the_games_names_and_little_else() {
        assert!(is_script_path("00/00-00-A00"));
        assert!(is_script_path("05/05-KB-OP1"));
        assert!(is_script_path("03/03-B2-O10"));
        assert!(!is_script_path("00-00-A00"));
        assert!(!is_script_path("PV/Trial_PV"));
        assert!(!is_script_path("Notice_SDHQ"));
        assert!(!is_script_path("00/00-00-a00"));
        assert!(!is_script_path(""));
    }
}
