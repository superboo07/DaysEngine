//! Reader for `Save/GlobalFlag.DAT`, FILMEngine's global flag store.
//!
//! # What the file is
//!
//! One `std::map<wstring, VARIANT>` written out whole. The game keeps every
//! piece of persistent progress in it: a flag per script the player has seen,
//! a flag per replay scene unlocked, the clear flags the title screen reads,
//! and the display strings the save/load screen shows for each slot.
//!
//! ```text
//! "FlgH"                     4 bytes, written but not checked on read
//! varint  count              number of entries
//! count x
//!     wstring name           enciphered, see below
//!     varint  flags          always -1 in every file seen
//!     varint  vt             VARIANT type tag
//!     value                  by vt, see below
//! ```
//!
//! There is no index and no length prefix per entry: the reader walks the
//! whole file. The trailing byte count is exact, so a short read is corruption
//! rather than an early end.
//!
//! # varint
//!
//! Seven bits per byte, most significant group first. The top bit of every
//! byte but the last is set, and `0x40` **of the first byte only** marks a
//! negative number, encoded as `-1 - magnitude`:
//!
//! ```text
//! 0x05        ->  5
//! 0x40        -> -1        (0x40, magnitude 0)
//! 0x90 0x7e   ->  2174     ((0x10 << 7) | 0x7e)
//! ```
//!
//! So `VARIANT_TRUE`, which is `-1`, is the single byte `0x40` — which is why
//! a fresh look at the file shows long runs of `40 0b 40`.
//!
//! # Strings
//!
//! A varint length in **characters**, then that many UTF-16LE code units, each
//! XORed with its own index:
//!
//! ```text
//! unit[i] ^= i as u16
//! ```
//!
//! The index is the character position and never wraps, so a 40-character
//! value is XORed with 0..40. This is why names look like readable text with
//! holes punched in it: `"01-34(67%H:;"` is `"00/00-00-A00"`. It is
//! obfuscation, not encryption — there is no key.
//!
//! # Value types
//!
//! The tag is a Windows `VARIANT` type:
//!
//! | vt | Type | Encoding |
//! |---|---|---|
//! | 3 | `VT_I4` | varint |
//! | 4 | `VT_R4` | 4 raw little-endian bytes |
//! | 8 | `VT_BSTR` | string, as above |
//! | 11 | `VT_BOOL` | varint; `-1` true, `0` false |
//!
//! Anything else is written as `VT_I4` zero, so no other tag can appear.
//!
//! All of the above is transcribed from the shipped reader and writer
//! (`FUN_0045fe90` and `FUN_004600d0` for the container, `FUN_004350b0` for
//! the varint, `FUN_00435010` for the string cipher, `FUN_0045c890` for the
//! value tags, all in `SCHOOLDAYS HQ.exe`) rather than inferred from the bytes.

#![forbid(unsafe_code)]

pub mod slot;

pub use slot::{Mark, Slot};

use std::collections::BTreeMap;

/// Magic at the head of `GlobalFlag.DAT`.
///
/// The game writes it and then, on read, checks only that four bytes came back
/// — it never compares them. We do compare, because a wrong file here is a bug
/// worth naming rather than a confusing parse failure further in.
pub const MAGIC: [u8; 4] = *b"FlgH";

/// What went wrong reading a flag store.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("not a flag store: expected magic {expected:?}, found {found:?}")]
    BadMagic { expected: [u8; 4], found: [u8; 4] },
    #[error("file ends in the middle of {what}")]
    Truncated { what: &'static str },
    #[error("varint at offset {offset} does not terminate within 5 bytes")]
    VarintTooLong { offset: usize },
    #[error("entry {index} ({name:?}) has unknown VARIANT type {vt}")]
    UnknownType { index: usize, name: String, vt: i32 },
    #[error("entry {index} declares a {len}-character string, longer than the rest of the file")]
    StringTooLong { index: usize, len: i32 },
    #[error("{unread} bytes left over after the declared {count} entries")]
    TrailingBytes { count: usize, unread: usize },
}

/// One stored value, as the `VARIANT` type tag found on disk.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    /// `VT_BOOL`. Stored as `-1` for true; any other value reads as false.
    Bool(bool),
    /// `VT_I4`.
    Int(i32),
    /// `VT_R4`.
    Float(f32),
    /// `VT_BSTR`.
    Str(String),
}

impl Value {
    /// The value as a flag, which is what almost every entry is.
    ///
    /// A non-boolean entry is not a flag and reads as `false` rather than as
    /// some coercion of its own type: the game's own getter is typed.
    pub fn as_bool(&self) -> bool {
        matches!(self, Value::Bool(true))
    }

    /// The value as an integer, or `None` if this entry is not `VT_I4`.
    pub fn as_int(&self) -> Option<i32> {
        match self {
            Value::Int(n) => Some(*n),
            _ => None,
        }
    }

    /// The value as a string, or `None` if this entry is not `VT_BSTR`.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }
}

/// The decoded contents of `GlobalFlag.DAT`.
///
/// Ordered, because the file is a `std::map` and therefore sorted by name;
/// keeping that order means a dump reads the way the game wrote it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FlagStore {
    entries: BTreeMap<String, Value>,
}

impl FlagStore {
    /// Decodes a flag store that is the whole of `bytes`.
    ///
    /// A `GlobalFlag.DAT` is exactly one store, so anything left over is
    /// corruption rather than a store that happens to be shorter. Use
    /// [`FlagStore::parse_embedded`] for the copy inside a save slot, which is
    /// followed by the rest of the slot.
    pub fn parse(bytes: &[u8]) -> Result<Self, Error> {
        let (store, read) = Self::parse_embedded(bytes)?;
        let unread = bytes.len() - read;
        if unread != 0 {
            return Err(Error::TrailingBytes {
                count: store.len(),
                unread,
            });
        }
        Ok(store)
    }

    /// Decodes a store that is followed by other data, returning it and how
    /// many bytes it took.
    ///
    /// `Save/SaveFile00N.DAT` embeds a whole store partway through itself, so
    /// there the end of the store is not the end of the file.
    pub fn parse_embedded(bytes: &[u8]) -> Result<(Self, usize), Error> {
        let mut r = Reader::new(bytes);

        let magic = r.take(4, "the magic")?;
        let magic: [u8; 4] = magic.try_into().expect("take(4) returns 4 bytes");
        if magic != MAGIC {
            return Err(Error::BadMagic {
                expected: MAGIC,
                found: magic,
            });
        }

        let count = r.varint("the entry count")?.max(0) as usize;
        let mut entries = BTreeMap::new();
        for index in 0..count {
            let name = r.string(index)?;
            // Present in every entry of every file seen, always -1. The game
            // reads it back into the VARIANT wrapper's own member rather than
            // into the value, so it is not part of the value.
            let _flags = r.varint("an entry's flag word")?;
            let vt = r.varint("a value type tag")?;
            let value = match vt {
                3 => Value::Int(r.varint("an integer value")?),
                4 => {
                    let raw = r.take(4, "a float value")?;
                    Value::Float(f32::from_le_bytes(
                        raw.try_into().expect("take(4) returns 4 bytes"),
                    ))
                }
                8 => Value::Str(r.string(index)?),
                11 => Value::Bool(r.varint("a boolean value")? == -1),
                vt => return Err(Error::UnknownType { index, name, vt }),
            };
            // A duplicate name cannot come out of a std::map, so the last one
            // winning is a formality rather than a policy.
            entries.insert(name, value);
        }

        Ok((Self { entries }, bytes.len() - r.remaining()))
    }

    /// Builds a store from entries already in hand.
    ///
    /// The file is a `std::map`, so a store is fully described by its entries;
    /// this is the constructor for one that did not come off disk — a caller
    /// synthesising save state, or a test that wants a particular save without
    /// hand-assembling the encoding.
    pub fn from_entries(entries: impl IntoIterator<Item = (String, Value)>) -> Self {
        Self {
            entries: entries.into_iter().collect(),
        }
    }

    /// Looks a value up by its plain (deciphered) name.
    pub fn get(&self, name: &str) -> Option<&Value> {
        self.entries.get(name)
    }

    /// Writes a value, creating the name if it is not there.
    ///
    /// The game's store does the same: `FUN_00460810` creates a missing name
    /// on demand, which is why an unread counter reads zero rather than
    /// failing.
    pub fn set(&mut self, name: &str, value: Value) {
        self.entries.insert(name.to_owned(), value);
    }

    /// An integer by name, zero when the name is absent.
    ///
    /// Zero rather than `None` because that is what the game's getter returns
    /// for a name it has just created. A name holding some other type reads as
    /// zero too, matching a typed getter against the wrong tag.
    pub fn int(&self, name: &str) -> i32 {
        self.get(name).and_then(Value::as_int).unwrap_or(0)
    }

    /// Sets an integer by name.
    pub fn set_int(&mut self, name: &str, value: i32) {
        self.set(name, Value::Int(value));
    }

    /// Sets a boolean by name, as `VT_BOOL`.
    pub fn set_flag(&mut self, name: &str, value: bool) {
        self.set(name, Value::Bool(value));
    }

    /// Removes a name entirely.
    pub fn remove(&mut self, name: &str) -> Option<Value> {
        self.entries.remove(name)
    }

    /// Whether a boolean flag is set. Missing and non-boolean both read false.
    ///
    /// This matches the game, whose getter returns false for a name it does
    /// not find: an absent flag and a cleared flag are the same thing.
    pub fn flag(&self, name: &str) -> bool {
        self.get(name).is_some_and(Value::as_bool)
    }

    /// Every entry, in the file's own (sorted) order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &Value)> {
        self.entries.iter().map(|(k, v)| (k.as_str(), v))
    }

    /// How many entries the store holds.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the store holds no entries.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

impl FlagStore {
    /// Encodes the store the way the game writes it.
    ///
    /// Byte for byte: the varint writer picks the shortest of its five lengths
    /// and the entries come out in the `std::map`'s sorted order, so a store
    /// read from a file and written back reproduces the file exactly. That is
    /// checked against the player's own saves by `days save --roundtrip`.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut w = Writer::default();
        self.write_into(&mut w);
        w.bytes
    }

    pub(crate) fn write_into(&self, w: &mut Writer) {
        w.raw(&MAGIC);
        w.varint(self.entries.len() as i32);
        for (name, value) in &self.entries {
            w.string(name);
            // -1 in every entry of every file seen; the game writes the
            // VARIANT wrapper's own member here, which it never sets otherwise.
            w.varint(-1);
            match value {
                Value::Int(n) => {
                    w.varint(3);
                    w.varint(*n);
                }
                Value::Float(f) => {
                    w.varint(4);
                    w.raw(&f.to_le_bytes());
                }
                Value::Str(s) => {
                    w.varint(8);
                    w.string(s);
                }
                Value::Bool(b) => {
                    w.varint(11);
                    w.varint(if *b { -1 } else { 0 });
                }
            }
        }
    }
}

/// The encoder, mirroring `FUN_004350b0` and `FUN_00434f70`.
#[derive(Debug, Default)]
pub(crate) struct Writer {
    pub(crate) bytes: Vec<u8>,
}

impl Writer {
    pub(crate) fn raw(&mut self, bytes: &[u8]) {
        self.bytes.extend_from_slice(bytes);
    }

    /// Seven bits per byte, most significant group first, shortest encoding.
    ///
    /// The first byte carries only six bits of magnitude because `0x40` is the
    /// sign, so the length thresholds are `0x40`, `0x2000`, `0x100000` and
    /// `0x8000000` — the game's own ladder, not a general LEB128.
    pub(crate) fn varint(&mut self, value: i32) {
        let (sign, magnitude) = if value < 0 {
            (0x40u8, (-1i64 - i64::from(value)) as u32)
        } else {
            (0, value as u32)
        };
        let groups = match magnitude {
            0..0x40 => 1,
            0x40..0x2000 => 2,
            0x2000..0x100000 => 3,
            0x100000..0x8000000 => 4,
            _ => 5,
        };
        for i in (0..groups).rev() {
            let shift = i * 7;
            // The first group is six bits wide; the rest are seven.
            let mask = if i == groups - 1 { 0x3f } else { 0x7f };
            let mut byte = ((magnitude >> shift) as u8) & mask;
            if i == groups - 1 {
                byte |= sign;
            }
            if i != 0 {
                byte |= 0x80;
            }
            self.bytes.push(byte);
        }
    }

    /// Length in UTF-16 code units, then the units XORed with their index.
    pub(crate) fn string(&mut self, s: &str) {
        let units: Vec<u16> = s.encode_utf16().collect();
        self.varint(units.len() as i32);
        for (i, u) in units.iter().enumerate() {
            self.bytes.extend_from_slice(&(u ^ i as u16).to_le_bytes());
        }
    }
}

/// A cursor over the file, with the two primitives everything else is built on.
pub(crate) struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    pub(crate) fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, pos: 0 }
    }

    pub(crate) fn remaining(&self) -> usize {
        self.bytes.len() - self.pos
    }

    pub(crate) fn rest(&self) -> &'a [u8] {
        &self.bytes[self.pos..]
    }

    pub(crate) fn skip(&mut self, n: usize) {
        self.pos = (self.pos + n).min(self.bytes.len());
    }

    pub(crate) fn take(&mut self, n: usize, what: &'static str) -> Result<&'a [u8], Error> {
        let end = self.pos.checked_add(n).ok_or(Error::Truncated { what })?;
        let slice = self
            .bytes
            .get(self.pos..end)
            .ok_or(Error::Truncated { what })?;
        self.pos = end;
        Ok(slice)
    }

    /// Seven bits per byte, high bit continues, `0x40` of the first byte means
    /// negative. Five bytes is the most the writer can emit, for a full i32.
    pub(crate) fn varint(&mut self, what: &'static str) -> Result<i32, Error> {
        let offset = self.pos;
        let first = *self.bytes.get(self.pos).ok_or(Error::Truncated { what })?;
        self.pos += 1;

        let negative = first & 0x40 != 0;
        let mut value = i64::from(first & 0x3f);
        let mut byte = first;
        while byte & 0x80 != 0 {
            if self.pos - offset >= 5 {
                return Err(Error::VarintTooLong { offset });
            }
            byte = *self.bytes.get(self.pos).ok_or(Error::Truncated { what })?;
            self.pos += 1;
            value = (value << 7) | i64::from(byte & 0x7f);
        }

        // The magnitude of a 5-byte encoding can exceed i32, which the writer
        // never produces; wrap rather than fail, so a hostile file is still a
        // parse and not a panic.
        Ok(if negative {
            (-1i64 - value) as i32
        } else {
            value as i32
        })
    }

    /// Length in characters, then UTF-16LE units each XORed with their index.
    pub(crate) fn string(&mut self, index: usize) -> Result<String, Error> {
        let len = self.varint("a string length")?;
        if len < 0 || len as usize > self.remaining() / 2 {
            return Err(Error::StringTooLong { index, len });
        }
        let raw = self.take(len as usize * 2, "a string")?;
        let units: Vec<u16> = raw
            .as_chunks::<2>()
            .0
            .iter()
            .enumerate()
            .map(|(i, pair)| u16::from_le_bytes(*pair) ^ (i as u16))
            .collect();
        // Lone surrogates would be corruption; replace rather than reject, so
        // one bad display string cannot cost the player the whole file.
        Ok(String::from_utf16_lossy(&units))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a store the way the game does, so the tests exercise the real
    /// encoding rather than a convenient one.
    fn encode(entries: &[(&str, Value)]) -> Vec<u8> {
        fn varint(out: &mut Vec<u8>, value: i32) {
            let (negative, magnitude) = if value < 0 {
                (true, (-1i64 - i64::from(value)) as u64)
            } else {
                (false, value as u64)
            };
            let mut groups = Vec::new();
            let mut left = magnitude;
            loop {
                groups.push((left & 0x7f) as u8);
                left >>= 7;
                if left == 0 {
                    break;
                }
            }
            groups.reverse();
            let last = groups.len() - 1;
            for (i, group) in groups.iter().enumerate() {
                let mut byte = *group;
                if i == 0 {
                    byte &= 0x3f;
                    if negative {
                        byte |= 0x40;
                    }
                }
                if i != last {
                    byte |= 0x80;
                }
                out.push(byte);
            }
        }
        fn string(out: &mut Vec<u8>, text: &str) {
            let units: Vec<u16> = text.encode_utf16().collect();
            varint(out, units.len() as i32);
            for (i, unit) in units.iter().enumerate() {
                out.extend_from_slice(&(unit ^ (i as u16)).to_le_bytes());
            }
        }

        let mut out = MAGIC.to_vec();
        varint(&mut out, entries.len() as i32);
        for (name, value) in entries {
            string(&mut out, name);
            varint(&mut out, -1);
            match value {
                Value::Int(n) => {
                    varint(&mut out, 3);
                    varint(&mut out, *n);
                }
                Value::Float(f) => {
                    varint(&mut out, 4);
                    out.extend_from_slice(&f.to_le_bytes());
                }
                Value::Str(s) => {
                    varint(&mut out, 8);
                    string(&mut out, s);
                }
                Value::Bool(b) => {
                    varint(&mut out, 11);
                    varint(&mut out, if *b { -1 } else { 0 });
                }
            }
        }
        out
    }

    #[test]
    fn round_trips_every_value_type() {
        let entries = [
            ("AllClear", Value::Bool(true)),
            ("EndClear", Value::Bool(false)),
            ("EndNo", Value::Int(20)),
            ("Ratio", Value::Float(1.5)),
            (
                "FILMEngine/SaveFile000_Sub",
                Value::Str("FINAL - TE AMO".into()),
            ),
        ];
        let store = FlagStore::parse(&encode(&entries)).expect("parses");
        assert_eq!(store.len(), entries.len());
        for (name, value) in &entries {
            assert_eq!(store.get(name), Some(value), "{name}");
        }
    }

    /// The bytes at the head of a real `GlobalFlag.DAT`, which is the only
    /// check that the varint and the cipher agree with the shipped writer.
    #[test]
    fn decodes_the_head_of_a_real_file() {
        let bytes = [
            b'F', b'l', b'g', b'H', // magic
            0x90, 0x7e, // count: 2174
            0x00, 0x40, 0x0b, 0x40, // "" = VT_BOOL true
            0x0c, // a 12-character name follows
            0x30, 0x00, 0x31, 0x00, 0x2d, 0x00, 0x33, 0x00, 0x34, 0x00, 0x28, 0x00, 0x36, 0x00,
            0x37, 0x00, 0x25, 0x00, 0x48, 0x00, 0x3a, 0x00, 0x3b, 0x00, //
            0x40, 0x0b, 0x40, // VT_BOOL true
        ];
        // The count says 2174, so parsing the whole prefix must fail; read the
        // pieces directly instead.
        let mut r = Reader::new(&bytes);
        assert_eq!(r.take(4, "magic").unwrap(), MAGIC);
        assert_eq!(r.varint("count").unwrap(), 2174);
        assert_eq!(r.string(0).unwrap(), "");
        assert_eq!(r.varint("flags").unwrap(), -1);
        assert_eq!(r.varint("vt").unwrap(), 11);
        assert_eq!(r.varint("value").unwrap(), -1);
        assert_eq!(r.string(1).unwrap(), "00/00-00-A00");
    }

    #[test]
    fn varint_encodes_the_documented_examples() {
        assert_eq!(Reader::new(&[0x05]).varint("x").unwrap(), 5);
        assert_eq!(Reader::new(&[0x40]).varint("x").unwrap(), -1);
        assert_eq!(Reader::new(&[0x90, 0x7e]).varint("x").unwrap(), 2174);
    }

    #[test]
    fn rejects_a_foreign_file() {
        let err = FlagStore::parse(b"DFLT\x78\x9c").unwrap_err();
        assert!(matches!(err, Error::BadMagic { .. }), "{err}");
    }

    #[test]
    fn rejects_a_truncated_file() {
        let bytes = encode(&[("AllClear", Value::Bool(true))]);
        let err = FlagStore::parse(&bytes[..bytes.len() - 1]).unwrap_err();
        assert!(matches!(err, Error::Truncated { .. }), "{err}");
    }

    #[test]
    fn rejects_trailing_bytes() {
        let mut bytes = encode(&[("AllClear", Value::Bool(true))]);
        bytes.push(0);
        let err = FlagStore::parse(&bytes).unwrap_err();
        assert!(matches!(err, Error::TrailingBytes { .. }), "{err}");
    }

    #[test]
    fn a_missing_flag_reads_false() {
        let store = FlagStore::parse(&encode(&[("EndNo", Value::Int(20))])).expect("parses");
        assert!(!store.flag("AllClear"));
        // An entry that exists but is not boolean is not a flag either.
        assert!(!store.flag("EndNo"));
    }
}
