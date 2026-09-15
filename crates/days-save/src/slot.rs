//! `Save/SaveFileNNN.DAT` — one save slot.
//!
//! # What a slot is
//!
//! Not a snapshot of the engine: a **log**, which is what its magic says. The
//! file records where the player is, every story point they have reached with
//! the state they reached it in, and the choice they made at every script.
//! Loading replays that into the engine rather than restoring a memory image,
//! which is why a slot survives across builds as long as the script version
//! matches.
//!
//! ```text
//! "SLog"                      4 bytes, compared on read
//! records, until a 0 tag:
//!     varint tag
//!     tag 1   wstring script     the file to reopen
//!             version            checked against _GetVersionToRoute@4;
//!                                an f32 in School Days HQ, a wstring in
//!                                Shiny Days -- see [`Version`]
//!             FlgH    store      the save's variable store at that moment
//!     tag 3   wstring script     a story point reached
//!             wstring story      the SP*** flag its marker set
//!             varint  order      how many story points preceded it
//!             FlgH    store      the variable store as it was there
//!     tag 4   wstring script     a script the player answered a choice at
//!             varint  choice     the index they chose, -1 for none
//!     tag 0   end of the file
//! ```
//!
//! Every other tag is refused, with the game's own words for it: "Undefined
//! backlog entry."
//!
//! # Where each record comes from
//!
//! - **tag 1** is written once. Its `FlgH` store is the position — `ROUTE` and
//!   `SCENE` are two of its names — and the script is only the file to reopen:
//!   the engine hands it to `FUN_0042a760`, which opens it and touches
//!   neither name. In Shiny Days that is the name **after** the uniform swap,
//!   so it can be a `Z` twin (`02/Z2-22-B04`) that appears in no route table;
//!   see `Progress::uniform_block` in the engine.
//! - **tag 3** is written by the story marker, host slot `+0x00`
//!   (`FUN_00428480`) — the same call that sets `SP%03d` in both flag stores.
//!   Its `order` is the size the map had when the point was first recorded, so
//!   it is the order the player reached them in and not the order they are
//!   stored in. Jumping back to a story point erases every entry from it
//!   onward (`FUN_004331a0`), which is why `order` has gaps in a save that has
//!   been rewound.
//! - **tag 4** is the recorded choice. `FUN_00431740` stores it as each choice
//!   box settles, and reads it back instead of asking the player while the
//!   engine is replaying.
//!
//! Both maps come out in `std::map` order: tag 3 by its **story flag**, tag 4
//! by its script. The single tag-1 record comes first.
//!
//! # One store, not two
//!
//! A slot carries one `FlgH` map per record, and that is the whole of the
//! save's state. Host slots `+0x08`/`+0x0c` (ints) and `+0x10`/`+0x14`
//! (booleans) all reach the same member, `host + 0x14`; only `+0x18`/`+0x1c`
//! are a different store, the global one in `GlobalFlag.DAT`. So the feeling
//! counters, the numbered gate flags and the `BS****` back-bookmarks share one
//! map, and a name can hold `VT_I4` or `VT_BOOL` depending on which setter
//! last wrote it — both appear in the player's own saves.
//!
//! Read from the shipped reader and writer, `FUN_004336c0` and `FUN_00433340`
//! in `SCHOOLDAYS HQ.exe`, and `FUN_004250e0` and `FUN_00423370` in
//! `SHINYDAYS.exe`.
//!
//! # The version field is the only thing the two titles spell differently
//!
//! Tags 3 and 4 are the same records in the same order in both. Tag 1's second
//! field is not: `SCHOOLDAYS HQ.exe` reads four raw bytes and compares them as
//! a float, and `SHINYDAYS.exe`'s `FUN_004250e0` reads a length-prefixed wide
//! string and compares it with `wcscmp`.
//!
//! That is the route module's doing, not the save's. Both titles' tag 1 is
//! checked against `_GetVersionToRoute@4`, an export of the player's own route
//! module, and the two exports return different *types* while choosing between
//! the same two values:
//!
//! ```text
//! RouteProcSDHQ.dll  _GetVersionToRoute@4 @ 0x10006840
//!     call [edx+0x34]; test eax,eax
//!     non-zero -> FLD dword ptr [0x100554f0]   = 0.01
//!     zero     -> FLD1                         = 1.0
//!
//! RouteProcSD.dll    _GetVersionToRoute@4 @ 0x10005b30
//!     call [edx+0x34]; test eax,eax
//!     non-zero -> MOV EAX, 0x1006468c          = L"0.01"
//!     zero     -> MOV EAX, 0x10064698          = L"1.0"
//! ```
//!
//! So the same rule, twice, in two representations. Every slot in either
//! install holds the zero branch -- `1.0` as a float, `"1.0"` as text.
//!
//! # How this reader tells them apart
//!
//! The shipped engines never have to: each executable ships exactly one of the
//! two readers, so the title decides. One engine reads both, so it decides per
//! file, on the `FlgH` magic that the store after the version field must begin
//! with. That is this engine's rule and not the original's, and it is
//! decidable rather than a guess: four raw bytes followed by `FlgH` is the
//! float form, and anything else is the string form, whose own length prefix
//! then has to land the store's magic in the same place.
//!
//! Both titles carry **two** slot readers, and which one runs is the same rule
//! in both: the following-record flag the replay play-data list raises, host
//! `+0x98` in School Days HQ and `+0xa4` in Shiny Days, picks a plain load
//! against one that follows the slot's own recorded answers. School Days HQ
//! spends it in `FUN_00423a70` on `FUN_0042b250`/`FUN_00428ab0`, which hand the
//! stream to `FUN_004336c0` or `FUN_00434020`; Shiny Days spends it in
//! `FUN_0041eb10` on `FUN_00419420`/`FUN_00419240`, which hand it to
//! `FUN_004250e0` or `FUN_00425830`. In Shiny Days the flag is member `+0x1e8`,
//! read by the getter `FUN_0041da10` and written by the setter `FUN_0041da20`.
//!
//! School Days HQ's two readers agree about tag 1: both take the version as
//! four raw bytes, `FUN_00434020` through `FUN_00434fe0`. Shiny Days' two do
//! **not** — `FUN_004250e0` takes the wide string described above and
//! `FUN_00425830` takes four raw bytes. **What that means for a Shiny Days slot
//! loaded with its answers followed is not established here**: it has not been
//! checked against the retail game, and this engine does not reach that path.
//! It is written down as what the disassembly says and no more.
//!
//! Every slot in the player's install is the string form, so that is the form
//! this engine reads and writes.

use crate::{Error, FlagStore, Reader, Writer};
use std::collections::BTreeMap;

/// Magic at the head of a save slot.
pub const MAGIC: [u8; 4] = *b"SLog";

/// What went wrong reading a slot.
#[derive(Debug, thiserror::Error)]
pub enum SlotError {
    #[error("not a save slot: expected magic {expected:?}, found {found:?}")]
    BadMagic { expected: [u8; 4], found: [u8; 4] },
    #[error("undefined backlog entry: record tag {tag} at offset {offset}")]
    UnknownTag { tag: i32, offset: usize },
    #[error("the slot has no record saying where the player is")]
    NoPosition,
    #[error("{unread} bytes left over after the end record")]
    TrailingBytes { unread: usize },
    #[error(transparent)]
    Store(#[from] Error),
}

/// One story point the player reached, and the state they reached it in.
#[derive(Debug, Clone, PartialEq)]
pub struct Mark {
    /// The script the story point is at.
    pub script: String,
    /// The `SP***` flag its marker set. This is the map's key, so it is unique
    /// within a slot.
    pub story: String,
    /// How many story points had been recorded before this one.
    pub order: i32,
    /// The save's variable store as it was at that point.
    pub store: FlagStore,
}

/// The script version a slot records, in the form its title's reader expects.
///
/// The two are the same rule in two representations, and which one a file
/// carries follows the title's route module: `_GetVersionToRoute@4` returns a
/// float from `RouteProcSDHQ.dll` and a wide string from `RouteProcSD.dll`.
/// Both choose between the same two values, `1.0` and `0.01`.
#[derive(Debug, Clone, PartialEq)]
pub enum Version {
    /// School Days HQ: four raw bytes, compared as a float. `FUN_004336c0`.
    Number(f32),
    /// Shiny Days: a length-prefixed wide string, compared with `wcscmp`.
    /// `FUN_004250e0`.
    Text(String),
}

impl Default for Version {
    /// The form School Days HQ writes, and the value every slot in either
    /// install holds.
    fn default() -> Self {
        Version::Number(1.0)
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Version::Number(v) => write!(f, "{v}"),
            Version::Text(v) => write!(f, "{v}"),
        }
    }
}

/// The decoded contents of a save slot.
#[derive(Debug, Clone, Default)]
pub struct Slot {
    /// The script the player is in, as the route tables spell it.
    pub script: String,
    /// The script engine's version. The game refuses a slot whose version is
    /// not the one `_GetVersionToRoute@4` reports, with a message box.
    pub version: Version,
    /// The save's variable store: `ROUTE`, `SCENE`, the feeling counters, the
    /// numbered gate flags and the `BS****` back-bookmarks.
    pub store: FlagStore,
    /// The story points reached, keyed by their `SP***` flag.
    pub marks: BTreeMap<String, Mark>,
    /// The choice made at each script, `-1` where the box settled without one.
    pub choices: BTreeMap<String, i32>,
}

impl Slot {
    /// Decodes a whole `Save/SaveFileNNN.DAT`.
    pub fn parse(bytes: &[u8]) -> Result<Slot, SlotError> {
        let mut r = Reader::new(bytes);
        let magic = r.take(4, "the magic")?;
        let magic: [u8; 4] = magic.try_into().expect("take(4) returns 4 bytes");
        if magic != MAGIC {
            return Err(SlotError::BadMagic {
                expected: MAGIC,
                found: magic,
            });
        }

        let mut slot = Slot::default();
        let mut positioned = false;
        loop {
            let offset = bytes.len() - r.remaining();
            match r.varint("a record tag")? {
                0 => break,
                1 => {
                    slot.script = r.string(0)?;
                    slot.version = r.version()?;
                    slot.store = r.store()?;
                    positioned = true;
                }
                3 => {
                    let script = r.string(0)?;
                    let story = r.string(0)?;
                    let order = r.varint("a story point's order")?;
                    let store = r.store()?;
                    slot.marks.insert(
                        story.clone(),
                        Mark {
                            script,
                            story,
                            order,
                            store,
                        },
                    );
                }
                4 => {
                    let script = r.string(0)?;
                    let choice = r.varint("a recorded choice")?;
                    slot.choices.insert(script, choice);
                }
                tag => return Err(SlotError::UnknownTag { tag, offset }),
            }
        }
        if !positioned {
            return Err(SlotError::NoPosition);
        }
        let unread = r.remaining();
        if unread != 0 {
            return Err(SlotError::TrailingBytes { unread });
        }
        Ok(slot)
    }

    /// Encodes the slot the way the game writes it.
    ///
    /// The record order is the writer's: the position first, then the story
    /// points in `std::map` order by their flag, then the choices in map order
    /// by script, then the end tag. A slot read and written back reproduces
    /// its file byte for byte.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut w = Writer::default();
        w.raw(&MAGIC);

        w.varint(1);
        w.string(&self.script);
        match &self.version {
            Version::Number(v) => w.raw(&v.to_le_bytes()),
            Version::Text(v) => w.string(v),
        }
        self.store.write_into(&mut w);

        for mark in self.marks.values() {
            w.varint(3);
            w.string(&mark.script);
            w.string(&mark.story);
            w.varint(mark.order);
            mark.store.write_into(&mut w);
        }
        for (script, choice) in &self.choices {
            w.varint(4);
            w.string(script);
            w.varint(*choice);
        }
        w.varint(0);
        w.bytes
    }

    /// The story points in the order the player reached them.
    pub fn in_order(&self) -> Vec<&Mark> {
        let mut marks: Vec<&Mark> = self.marks.values().collect();
        marks.sort_by_key(|m| m.order);
        marks
    }
}

impl Reader<'_> {
    /// Tag 1's version field, in whichever of the two forms the file carries.
    ///
    /// Told apart by the store magic that has to follow it. The shipped
    /// engines never do this -- each executable ships one reader and the title
    /// decides -- but one engine reads both titles' saves, so it decides per
    /// file. See the module docs.
    fn version(&mut self) -> Result<Version, Error> {
        if self.rest().get(4..8) == Some(&crate::MAGIC) {
            let raw = self.take(4, "the script version")?;
            let raw: [u8; 4] = raw.try_into().expect("take(4) returns 4 bytes");
            return Ok(Version::Number(f32::from_le_bytes(raw)));
        }
        Ok(Version::Text(self.string(0)?))
    }

    /// A whole `FlgH` store embedded in a record.
    fn store(&mut self) -> Result<FlagStore, Error> {
        let (store, read) = FlagStore::parse_embedded(self.rest())?;
        self.skip(read);
        Ok(store)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Value;

    fn slot() -> Slot {
        let mut s = Slot {
            script: "05/05-A2-Z00".into(),
            version: Version::Number(1.0),
            store: FlagStore::from_entries([
                ("001".to_owned(), Value::Int(69)),
                ("002".to_owned(), Value::Int(62)),
                ("946".to_owned(), Value::Bool(true)),
                ("ROUTE".to_owned(), Value::Int(39)),
                ("SCENE".to_owned(), Value::Int(6)),
            ]),
            ..Default::default()
        };
        s.marks.insert(
            "SP100".into(),
            Mark {
                script: "00/00-00-A00".into(),
                story: "SP100".into(),
                order: 0,
                store: FlagStore::from_entries([("001".to_owned(), Value::Int(0))]),
            },
        );
        s.marks.insert(
            "SP101".into(),
            Mark {
                script: "00/00-00-A03".into(),
                story: "SP101".into(),
                order: 1,
                store: FlagStore::from_entries([("001".to_owned(), Value::Int(5))]),
            },
        );
        s.choices.insert("00/00-00-A03".into(), 0);
        s.choices.insert("01/01-00-J01".into(), -1);
        s
    }

    #[test]
    fn a_slot_survives_a_round_trip() {
        let bytes = slot().to_bytes();
        let back = Slot::parse(&bytes).expect("parses");
        assert_eq!(back.script, "05/05-A2-Z00");
        assert_eq!(back.version, Version::Number(1.0));
        assert_eq!(back.store.get("001"), Some(&Value::Int(69)));
        assert_eq!(back.store.get("946"), Some(&Value::Bool(true)));
        assert_eq!(back.marks.len(), 2);
        assert_eq!(back.marks["SP101"].script, "00/00-00-A03");
        assert_eq!(back.choices["01/01-00-J01"], -1);
        assert_eq!(back.to_bytes(), bytes);
    }

    #[test]
    fn either_title_s_version_field_survives_the_store_magic_that_follows_it() {
        // The two forms are told apart by the `FlgH` the store must begin
        // with, so the case that matters is the one where the string form's
        // own bytes could be mistaken for four raw bytes and back.
        let mut hq = slot();
        hq.version = Version::Number(1.0);
        let mut sd = slot();
        sd.version = Version::Text("1.0".into());

        let hq_bytes = hq.to_bytes();
        let sd_bytes = sd.to_bytes();
        assert_ne!(hq_bytes, sd_bytes);
        assert_eq!(
            Slot::parse(&hq_bytes).unwrap().version,
            Version::Number(1.0)
        );
        assert_eq!(
            Slot::parse(&sd_bytes).unwrap().version,
            Version::Text("1.0".into())
        );
        // And the other value the route module can report, whose text form is
        // four characters and so eight bytes rather than four.
        let mut other = slot();
        other.version = Version::Text("0.01".into());
        let bytes = other.to_bytes();
        assert_eq!(
            Slot::parse(&bytes).unwrap().version,
            Version::Text("0.01".into())
        );
        assert_eq!(Slot::parse(&bytes).unwrap().to_bytes(), bytes);
    }

    #[test]
    fn the_story_points_come_back_in_the_order_they_were_reached() {
        let mut s = slot();
        // A slot that has been rewound and replayed has story points whose
        // order does not match their sorted names.
        s.marks.get_mut("SP100").unwrap().order = 7;
        let order: Vec<&str> = s.in_order().iter().map(|m| m.story.as_str()).collect();
        assert_eq!(order, ["SP101", "SP100"]);
    }

    #[test]
    fn a_file_that_is_not_a_slot_is_named_as_such() {
        let err = Slot::parse(b"FlgH\0").unwrap_err();
        assert!(matches!(err, SlotError::BadMagic { .. }));
    }

    #[test]
    fn an_undefined_record_tag_is_refused_rather_than_skipped() {
        let mut bytes = slot().to_bytes();
        // Overwrite the end tag with one the game calls an undefined backlog
        // entry. There is no length prefix, so skipping is not possible.
        let last = bytes.len() - 1;
        bytes[last] = 9;
        bytes.push(0);
        assert!(matches!(
            Slot::parse(&bytes),
            Err(SlotError::UnknownTag { tag: 9, .. })
        ));
    }

    #[test]
    fn a_slot_with_no_position_is_refused() {
        let bytes = [MAGIC.as_slice(), &[0]].concat();
        assert!(matches!(Slot::parse(&bytes), Err(SlotError::NoPosition)));
    }
}
