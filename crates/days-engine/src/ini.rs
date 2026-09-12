//! Reader for the engine's `.INI` files.
//!
//! The format is not Windows INI despite the extension — there are no sections,
//! and every line is `[Key]="value"`, sometimes with a trailing `;`:
//!
//! ```text
//! [StartScript]="00/00-00-A00"
//! [UseEnglish]="1";
//! ```
//!
//! Keys repeat: `DX9GRAPHIC.INI` lists `[DisplayPixelMode]` twice to offer two
//! formats. So lookups return the first occurrence and [`Ini::all`] exposes the
//! rest.

use std::collections::HashMap;

/// A parsed `.INI` file.
#[derive(Debug, Default)]
pub struct Ini {
    /// Lowercased key -> values, in file order.
    entries: HashMap<String, Vec<String>>,
}

impl Ini {
    /// Parses a file. Malformed lines are skipped rather than fatal — these are
    /// hand-edited data files and one bad line should not stop the game.
    pub fn parse(text: &str) -> Ini {
        let mut entries: HashMap<String, Vec<String>> = HashMap::new();
        for line in text.lines() {
            let line = line.trim().trim_start_matches('\u{feff}');
            let Some(rest) = line.strip_prefix('[') else {
                continue;
            };
            let Some(close) = rest.find("]=") else {
                continue;
            };
            let key = rest[..close].trim().to_ascii_lowercase();
            let value = rest[close + 2..]
                .trim()
                .trim_end_matches(';')
                .trim()
                .trim_matches('"')
                .to_string();
            entries.entry(key).or_default().push(value);
        }
        Ini { entries }
    }

    /// Parses from raw bytes, accepting the UTF-16 files the Japanese build ships.
    pub fn parse_bytes(bytes: &[u8]) -> Ini {
        let text = if let Some(rest) = bytes.strip_prefix(&[0xff, 0xfe]) {
            let units: Vec<u16> = rest
                .as_chunks::<2>()
                .0
                .iter()
                .map(|p| u16::from_le_bytes(*p))
                .collect();
            String::from_utf16_lossy(&units)
        } else {
            String::from_utf8_lossy(bytes).into_owned()
        };
        Ini::parse(&text)
    }

    /// First value for a key, case-insensitively.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.entries
            .get(&key.to_ascii_lowercase())
            .and_then(|v| v.first())
            .map(String::as_str)
    }

    /// Every value for a key, in file order.
    pub fn all(&self, key: &str) -> &[String] {
        self.entries
            .get(&key.to_ascii_lowercase())
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    pub fn get_u32(&self, key: &str) -> Option<u32> {
        self.get(key)?.trim().parse().ok()
    }

    /// `"1"` is true, everything else false.
    pub fn get_bool(&self, key: &str) -> Option<bool> {
        Some(self.get(key)?.trim() == "1")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verbatim from the retail STARTSCRIPT.INI and DX9GRAPHIC.INI.
    const SAMPLE: &str = "\u{feff}[Version]=\"Ver1.00\"\n\n\
        [StartMode]=\"Title\"\n\n\
        [StartScript]=\"00/00-00-A00\"\n\n\
        [UseEnglish]=\"1\";\n\n\
        [DisplayWidthSize]=\"800\"\n\
        [DisplayPixelMode]=\"X8R8G8B8\"\n\
        [DisplayPixelMode]=\"R5G6B5\"\n\
        [TrialDemo]=\"\"\n";

    #[test]
    fn parses_keys_values_and_a_bom() {
        let ini = Ini::parse(SAMPLE);
        assert_eq!(ini.get("Version"), Some("Ver1.00"));
        assert_eq!(ini.get("StartScript"), Some("00/00-00-A00"));
        assert_eq!(ini.get_u32("DisplayWidthSize"), Some(800));
        assert_eq!(ini.get("TrialDemo"), Some(""));
        assert_eq!(ini.get("missing"), None);
    }

    /// A trailing `;` is not part of the value.
    #[test]
    fn strips_trailing_semicolons() {
        let ini = Ini::parse(SAMPLE);
        assert_eq!(ini.get("UseEnglish"), Some("1"));
        assert_eq!(ini.get_bool("UseEnglish"), Some(true));
    }

    /// Repeated keys keep every value; `get` returns the first.
    #[test]
    fn repeated_keys_are_all_retained() {
        let ini = Ini::parse(SAMPLE);
        assert_eq!(ini.get("DisplayPixelMode"), Some("X8R8G8B8"));
        assert_eq!(ini.all("DisplayPixelMode"), ["X8R8G8B8", "R5G6B5"]);
    }

    #[test]
    fn lookups_ignore_case() {
        let ini = Ini::parse(SAMPLE);
        assert_eq!(ini.get("startscript"), ini.get("StartScript"));
    }

    #[test]
    fn utf16_files_parse() {
        let mut bytes = vec![0xff, 0xfe];
        for u in "[StartScript]=\"00/00-00-A00\"".encode_utf16() {
            bytes.extend_from_slice(&u.to_le_bytes());
        }
        assert_eq!(
            Ini::parse_bytes(&bytes).get("StartScript"),
            Some("00/00-00-A00")
        );
    }
}
