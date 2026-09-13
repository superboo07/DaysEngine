//! The affection counters the branch system keeps, and the two tables that
//! drive them.
//!
//! # Five counters, of which two are on screen
//!
//! `RouteProcSDHQ.dll` keeps five named integers in the save's variable store,
//! alongside `ROUTE` and `SCENE`. Their names are declared in the head of both
//! feeling tables, which agree:
//!
//! ```text
//! [Number]="5"
//! [flag0]="002"  [flag1]="000"  [flag2]="001"  [flag3]="003"  [flag4]="004"
//! ```
//!
//! They are plain `VT_I4` entries — a save mid-game holds `001 = 89`,
//! `002 = 78` — and what each is worth differs sharply:
//!
//! | Name | Drawn | Read by |
//! |---|---|---|
//! | `001` (Sekai), `002` (Kotonoha) | **both gauge bars** | the branch test below, at 25 routes; thresholds at 10 scripts |
//! | `004` | no | thresholds at 3 scripts, and nothing else |
//! | `000` | no | nothing: it is the filler, see below |
//! | `003` | no | nothing |
//!
//! `000` is named in 1,144 of the 1,438 delta slots in `FEELINGSCRIPT.INI` and
//! **its amount is always zero**. That is not a coincidence: the reader
//! (`FUN_10005eb0`) always consumes exactly two `(name, amount)` pairs, so an
//! entry that only wants one writes `000, 0` into the other. `003` is named
//! once, also with zero. Neither is read anywhere; `ZeroReset` sets them along
//! with the rest and that is all that ever touches them.
//!
//! The shipped table never uses the second pair for anything real: across all
//! 719 entries there are 283 non-zero amounts spread over 283 entries, so no
//! script moves two counters at once. [`Deltas::for_script`] still returns both
//! pairs and [`credit`] still credits both, because that is what the
//! reader does — but the second being live is untested by the retail data.
//!
//! Sixteen of the 719 entries key on a script that no route table names, so
//! nothing can reach them.
//!
//! # How a branch uses them
//!
//! Two mechanisms, and neither is a bar filling up to a threshold.
//!
//! **Relative.** 25 of the 55 routes contain exactly one site of this, and
//! every one of the 25 is character-for-character the same
//! ([`first_leads`]):
//!
//! ```text
//! a = get(L"001");  b = get(L"002");
//! if (b < a)  -> one scene   else  -> another
//! ```
//!
//! So it is which counter is **ahead**, never how far — and a tie takes the
//! `else`. Every other branch in the game is decided by the choice the player
//! made, not by these.
//!
//! **Absolute.** `STANDERDSCRIPT.INI` gives 13 named scripts a single
//! `(name, amount)` pair, and `FUN_10006000` answers whether the counter is
//! past it ([`Thresholds::passes`]). The test is `cmp eax,[ebp-0x14]` followed
//! by `jle`, so it is **strictly greater**: `[01-00-N05]="002, 11"` passes at
//! 12, not at 11. `FUN_10006000` has exactly 13 call sites, one per entry.
//!
//! # Why only two bars are drawn
//!
//! Nothing in the bar decides that. `FUN_10005c60`, which applies one delta,
//! ends with:
//!
//! ```text
//! if (name == L"001" || name == L"002")  host->slot_0x30(1);
//! ```
//!
//! Host slot `+0x30` writes `engine + 0x79c`, and slot `+0x154` — the one the
//! control bar asks before drawing the gauge over a faded bar — reads that
//! same member back. So the gauge is shown precisely when a delta moved `001`
//! or `002`, and `FUN_10026050` clears it again through slot `+0x30(0)` once
//! it has read the two values. [`credit`] returns that flag.
//!
//! # The table format
//!
//! Both files are read by substring search, not by parsing. Given a script
//! path the reader drops the **first three characters** — `00/00-00-A04`
//! becomes `00-00-A04` — builds the literal `[00-00-A04]="`, and looks for it
//! anywhere in the file's text. From just past the match it reads fields
//! separated by `, `, each ending at a `,` or a `"`:
//!
//! ```text
//! FEELINGSCRIPT.INI   [00-00-A04]="002, 5, 000, 0"    two (name, amount) pairs
//! STANDERDSCRIPT.INI  [01-00-N05]="002, 11"           one (name, amount) pair
//! ```
//!
//! There are no sections and the `[Number]` / `[flag%d]` head is found the same
//! way. A script with no entry simply has no effect — `FUN_10005eb0` returns
//! without touching anything, and `FUN_10006000` answers false.
//!
//! Provenance: `FUN_10005eb0` (apply), `FUN_10005d60` (fields), `FUN_10005c60`
//! and `FUN_10005ce0` (add and subtract), `FUN_10006000` (threshold),
//! `FUN_10006230` (the head), `_ZeroReset@4`, all in `RouteProcSDHQ.dll`.

use days_save::FlagStore;

/// Sekai's counter: the right-hand, orange half of the gauge, and the left side
/// of the branch test.
pub const FIRST: &str = "001";

/// Kotonoha's counter: the left-hand, green half of the gauge, and the right
/// side of the branch test.
pub const SECOND: &str = "002";

/// Characters the readers drop from the front of a script path before building
/// a key: `00/00-00-A04` is looked up as `00-00-A04`.
///
/// `FID_conflict_erase(s, 0, 3)` in both `FUN_10005eb0` and `FUN_10006000`.
const PREFIX: usize = 3;

/// The key for a script path, as the readers build it.
///
/// Returns `None` for a path too short to have a prefix to drop, which no
/// shipped name is.
fn key(script: &str) -> Option<String> {
    let bare = script.get(PREFIX..)?;
    Some(format!("[{bare}]=\""))
}

/// Reads the fields after a matched key.
///
/// `FUN_10005d60` appends characters until it sees a `,` or a `"`, then steps
/// two characters on — past the separator and the space that always follows
/// it. It does that twice to make one `(name, amount)` pair. The amount goes
/// through a `VARIANT` coercion to `VT_I4`, which yields zero for anything
/// that is not a number rather than failing.
fn pairs(rest: &str, want: usize) -> Vec<(String, i32)> {
    let mut out = Vec::with_capacity(want);
    let mut it = rest.split([',', '"']);
    for _ in 0..want {
        let (Some(name), Some(amount)) = (it.next(), it.next()) else {
            break;
        };
        out.push((
            name.trim().to_string(),
            amount.trim().parse::<i32>().unwrap_or(0),
        ));
    }
    out
}

/// The `[Number]` / `[flag%d]` head both tables carry.
///
/// `FUN_10006230` reads `[Number]="N"` and then `[flag0]` upward, appending
/// each value to one list shared by both files — the second file's names are
/// already there, so it contributes nothing. The list is what `ZeroReset`
/// walks.
fn head(text: &str) -> Vec<String> {
    let Some(count) = field(text, "[Number]=\"").and_then(|v| v.parse::<usize>().ok()) else {
        return Vec::new();
    };
    (0..count)
        .filter_map(|i| field(text, &format!("[flag{i}]=\"")))
        .map(str::to_string)
        .collect()
}

/// The text between a key and the `"` that closes it.
fn field<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    let at = text.find(key)? + key.len();
    let rest = &text[at..];
    Some(&rest[..rest.find('"').unwrap_or(rest.len())])
}

/// `FEELINGSCRIPT.INI`: what each script adds to which counters.
#[derive(Debug, Clone)]
pub struct Deltas {
    text: String,
    names: Vec<String>,
}

impl Deltas {
    /// Reads the table. The file is 8-bit; the game widens it on load.
    pub fn parse(bytes: &[u8]) -> Deltas {
        let text = String::from_utf8_lossy(bytes).into_owned();
        let names = head(&text);
        Deltas { text, names }
    }

    /// The five counter names, in the order `[flag0]` upward.
    pub fn names(&self) -> &[String] {
        &self.names
    }

    /// The two `(name, amount)` pairs credited for a script, in file order.
    ///
    /// Empty for a script the table does not mention, which is most of them:
    /// 719 of the 1,857 scripts have an entry.
    pub fn for_script(&self, script: &str) -> Vec<(String, i32)> {
        let Some(k) = key(script) else {
            return Vec::new();
        };
        match self.text.find(&k) {
            Some(at) => pairs(&self.text[at + k.len()..], 2),
            None => Vec::new(),
        }
    }
}

/// `STANDERDSCRIPT.INI`: the thirteen scripts gated on a counter's value.
#[derive(Debug, Clone, Default)]
pub struct Thresholds {
    text: String,
}

impl Thresholds {
    pub fn parse(bytes: &[u8]) -> Thresholds {
        Thresholds {
            text: String::from_utf8_lossy(bytes).into_owned(),
        }
    }

    /// The `(name, amount)` a script is gated on, if it is gated at all.
    pub fn for_script(&self, script: &str) -> Option<(String, i32)> {
        let k = key(script)?;
        let at = self.text.find(&k)?;
        pairs(&self.text[at + k.len()..], 1).into_iter().next()
    }
}

/// The save's variable store is where the counters live.
///
/// Not a store of their own: host slots `+0x08`/`+0x0c` (integers) and
/// `+0x10`/`+0x14` (booleans) all reach the same member, `host + 0x14`, so the
/// five counters share one `std::map<wstring, VARIANT>` with `ROUTE`, `SCENE`,
/// the numbered gate flags and the `BS****` back-bookmarks. A player's own
/// save has `001` as `VT_I4` and `946` as `VT_BOOL` side by side in it.
///
/// So these are operations **on that store** rather than a type of their own.
/// A name never written reads as zero, which is what the game's store does:
/// `FUN_00460810` creates a missing name on demand and the typed getter
/// coerces it.
///
/// Credits a script's deltas, as `_SetFeeling@8` does with mode 1.
///
/// Returns whether the gauge should be shown — true when a delta moved
/// [`FIRST`] or [`SECOND`], which is the condition `FUN_10005c60` signals
/// through host slot `+0x30`. A zero amount is skipped entirely
/// (`if (delta != 0)` guards the whole body), so the filler pairs never raise
/// it.
pub fn credit(store: &mut FlagStore, deltas: &Deltas, script: &str) -> bool {
    apply(store, deltas, script, 1)
}

/// Takes a script's deltas back off again, which is `FUN_10005ce0` — the
/// mode-0 half of the same call, used when the player moves backwards.
pub fn uncredit(store: &mut FlagStore, deltas: &Deltas, script: &str) -> bool {
    apply(store, deltas, script, -1)
}

fn apply(store: &mut FlagStore, deltas: &Deltas, script: &str, sign: i32) -> bool {
    let mut gauge = false;
    for (name, amount) in deltas.for_script(script) {
        if amount == 0 {
            continue;
        }
        store.set_int(&name, store.int(&name) + sign * amount);
        if name == FIRST || name == SECOND {
            gauge = true;
        }
    }
    gauge
}

/// Sets every counter the table head names to zero, as `_ZeroReset@4` does at
/// the start of a new game.
pub fn zero_reset(store: &mut FlagStore, names: &[String]) {
    for name in names {
        store.set_int(name, 0);
    }
}

/// Whether a script's threshold is met, as `FUN_10006000` answers it.
///
/// **Strictly greater**, and false for a script with no entry — both straight
/// off the disassembly, where the `jle` takes the false arm and the no-match
/// path falls into the same zero.
pub fn passes(store: &FlagStore, thresholds: &Thresholds, script: &str) -> bool {
    match thresholds.for_script(script) {
        Some((name, amount)) => store.int(&name) > amount,
        None => false,
    }
}

/// The branch test the 25 route handlers share: `if (get("002") < get("001"))`.
///
/// True when [`FIRST`] is ahead of [`SECOND`] — the `if` arm. A tie is false,
/// because the comparison is a strict `<`.
pub fn first_leads(store: &FlagStore) -> bool {
    store.int(SECOND) < store.int(FIRST)
}

/// The two values the gauge draws, `(001, 002)`.
///
/// `FUN_10026050` reads exactly these two names through host slot `+8` and
/// stores them as floats.
pub fn gauge(store: &FlagStore) -> (i32, i32) {
    (store.int(FIRST), store.int(SECOND))
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEAD: &str = "[Number]=\"5\"\n[flag0]=\"002\"\n[flag1]=\"000\"\n[flag2]=\"001\"\n\
                        [flag3]=\"003\"\n[flag4]=\"004\"\n\n";

    fn deltas(body: &str) -> Deltas {
        Deltas::parse(format!("{HEAD}{body}").as_bytes())
    }

    #[test]
    fn the_head_names_five_counters_in_flag_order() {
        let d = deltas("");
        assert_eq!(d.names(), ["002", "000", "001", "003", "004"]);
    }

    /// The key drops the `00/` directory the script path carries.
    #[test]
    fn a_script_is_looked_up_without_its_directory() {
        let d = deltas("[00-00-A04]=\"002, 5, 000, 0\"\n");
        assert_eq!(
            d.for_script("00/00-00-A04"),
            [("002".into(), 5), ("000".into(), 0)]
        );
    }

    #[test]
    fn a_script_with_no_entry_credits_nothing() {
        let d = deltas("[00-00-A04]=\"002, 5, 000, 0\"\n");
        assert!(d.for_script("00/00-00-Z99").is_empty());
    }

    /// Both amounts count when both are real; this is the shape used by the
    /// entries that move two counters at once.
    #[test]
    fn both_pairs_are_credited() {
        let d = deltas("[01-00-B01]=\"001, 5, 002, 3\"\n");
        let mut f = FlagStore::default();
        assert!(credit(&mut f, &d, "01/01-00-B01"));
        assert_eq!(f.int("001"), 5);
        assert_eq!(f.int("002"), 3);
    }

    /// The filler pair is skipped by the `!= 0` guard, so it never counts as a
    /// change and never raises the gauge on its own.
    #[test]
    fn the_filler_pair_changes_nothing_and_does_not_raise_the_gauge() {
        let d = deltas("[00-00-A06]=\"000, 0, 000, 0\"\n");
        let mut f = FlagStore::default();
        assert!(!credit(&mut f, &d, "00/00-00-A06"));
        assert_eq!(f.iter().count(), 0);
    }

    /// A counter that is not 001 or 002 still accrues, but draws nothing.
    #[test]
    fn a_hidden_counter_accrues_without_raising_the_gauge() {
        let d = deltas("[04-SE-K00]=\"004, 1, 000, 0\"\n");
        let mut f = FlagStore::default();
        assert!(!credit(&mut f, &d, "04/04-SE-K00"));
        assert_eq!(f.int("004"), 1);
    }

    #[test]
    fn undo_takes_the_same_deltas_back_off() {
        let d = deltas("[01-00-B01]=\"001, 5, 002, 3\"\n");
        let mut f = FlagStore::default();
        credit(&mut f, &d, "01/01-00-B01");
        assert!(uncredit(&mut f, &d, "01/01-00-B01"));
        assert_eq!(f.int("001"), 0);
        assert_eq!(f.int("002"), 0);
    }

    #[test]
    fn zero_reset_clears_every_name_the_head_declares() {
        let d = deltas("");
        let mut f = FlagStore::default();
        f.set_int("001", 40);
        f.set_int("004", 2);
        zero_reset(&mut f, d.names());
        assert_eq!(f.int("001"), 0);
        assert_eq!(f.int("004"), 0);
        assert_eq!(f.iter().count(), 5);
    }

    /// The branch test is a strict `<`, so equal counters take the `else`.
    #[test]
    fn the_branch_test_is_strict_so_a_tie_goes_the_other_way() {
        let mut f = FlagStore::default();
        f.set_int("001", 30);
        f.set_int("002", 30);
        assert!(!first_leads(&f));
        f.set_int("002", 29);
        assert!(first_leads(&f));
        f.set_int("002", 31);
        assert!(!first_leads(&f));
    }

    /// `jle` takes the false arm, so the threshold is passed only above it.
    #[test]
    fn a_threshold_is_strictly_greater_not_at_least() {
        let t = Thresholds::parse(format!("{HEAD}[01-00-N05]=\"002, 11\"\n").as_bytes());
        let mut f = FlagStore::default();
        f.set_int("002", 11);
        assert!(!passes(&f, &t, "01/01-00-N05"));
        f.set_int("002", 12);
        assert!(passes(&f, &t, "01/01-00-N05"));
    }

    #[test]
    fn an_ungated_script_does_not_pass() {
        let t = Thresholds::parse(format!("{HEAD}[01-00-N05]=\"002, 11\"\n").as_bytes());
        let f = FlagStore::default();
        assert!(!passes(&f, &t, "05/05-KB-A00"));
    }

    #[test]
    fn the_gauge_reads_the_two_named_counters() {
        let mut f = FlagStore::default();
        f.set_int("001", 89);
        f.set_int("002", 78);
        f.set_int("004", 4);
        assert_eq!(gauge(&f), (89, 78));
    }
}
