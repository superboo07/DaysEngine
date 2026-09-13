//! `[Input]` — which keys and which controller buttons do what.
//!
//! The original is keyboard and mouse. It has no input API at all: the menu
//! DLL imports none, and the executable reads the keyboard through the window
//! procedure. So nothing in this module is recovered behaviour — it is an
//! engine feature, and it is here because a game that can only be played with
//! two hands on a keyboard and a mouse cannot be played by everyone. A
//! controller is the difference between "plays School Days" and "cannot".
//!
//! The default table reproduces the original's keys exactly (see
//! [`Bindings::default`]), so a player who never opens `DaysEngine.ini` gets
//! the game the way it shipped, and adds a controller layout beside them.
//!
//! # The file
//!
//! ```text
//! [Input]
//! Confirm = return, keypad enter, space, pad:a
//! Cancel  = escape, pad:b
//! Pause   = space, pad:start
//! ```
//!
//! Naming an action **replaces** its whole list, so a player who wants only
//! their own binding gets only their own binding. An action the file does not
//! name keeps its defaults. An empty value unbinds the action entirely.
//!
//! A trigger is one of:
//!
//! - a key, by SDL's own name for it, case-insensitively: `up`, `space`,
//!   `return`, `r`, `escape`, `keypad enter`. A few spellings a person is
//!   likely to reach for are accepted as aliases — see [`Trigger::parse`].
//! - a controller button, `pad:` and SDL's own name: `pad:a`, `pad:dpup`,
//!   `pad:start`, `pad:leftshoulder`.
//! - a controller axis pushed one way, `pad:` a sign and SDL's own name:
//!   `pad:-lefty` is the left stick pushed up, `pad:+righttrigger` is the
//!   right trigger pulled.
//!
//! The names are not checked here. SDL owns them, and this module is not
//! allowed to know about SDL — [`crate::install`] is the player's install, not
//! the window. A name SDL does not know is a warning from the engine at
//! startup and a trigger that never fires, which is the same rule every other
//! unreadable setting follows.

use std::fmt;

/// Something the player can ask for, wherever they are.
///
/// One list, not one per screen, because the screens overlap: the title menu
/// and the control bar both have a confirm, and a player who rebinds it means
/// both. Where an action means nothing — [`Action::Auto`] on the title screen —
/// it is simply not read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Action {
    /// Move the selection up. Menus, and the choice box's previous label.
    Up,
    /// Move the selection down. Menus, and the choice box's next label.
    Down,
    /// Move the selection left. In playback with nothing selected this is
    /// [`Action::SeekBack`], which is what the keyboard has always done.
    Left,
    /// Move the selection right, or seek forward. See [`Action::Left`].
    Right,
    /// Activate what is selected.
    ///
    /// During playback there is often nothing selected: the control bar is a
    /// strip the pointer hovers, not something that holds a selection of its
    /// own, and most of a script has no choice box up. **With nothing to
    /// confirm, this pauses** — which is what Space has always done here, and
    /// what stops the face button under your thumb being dead for most of the
    /// game. See [`Action::Pause`], which is the same thing asked for
    /// directly.
    Confirm,
    /// Back out. The menus' cancel; the choice box's refusal.
    Cancel,
    /// Close the game. Bound to Escape during playback, as it always was.
    Quit,
    /// Pause or resume playback. The control bar's widget 1.
    ///
    /// Bound only to a dedicated button, because [`Action::Confirm`] already
    /// reaches this whenever nothing else takes it. Unlike that fall-through
    /// this one always pauses, selection or choice box notwithstanding: a
    /// player who bound a key to "pause" asked for pause.
    Pause,
    /// Jump five seconds back.
    SeekBack,
    /// Jump five seconds on.
    SeekForward,
    /// Play this script again from its first frame. Bar widget 2, latch and
    /// all: a second press inside `RESTART_LATCH_FRAMES` leaves the script
    /// instead, because this presses the widget rather than short-circuiting
    /// to what the widget usually does.
    Restart,
    /// Toggle auto-advance. Bar widget 0.
    Auto,
    /// One step up the bar's speed list. Bar widgets 5 to 9.
    Faster,
    /// One step down it.
    Slower,
    /// Jump to the choice this script raises. Bar widget 4.
    SkipToChoice,
    /// Open the save screen over playback. Bar widget 10.
    SaveMenu,
    /// Open the load screen. Bar widget 11.
    LoadMenu,
    /// Open the Option screen. Bar widget 13.
    OptionMenu,
    /// Stop playing and go back to the title. Bar widget 14.
    LeavePlayback,
    /// Take the selection to the control bar, or off it again.
    ///
    /// The bar is a pointer-driven strip: it drops down when the pointer is
    /// over it and every widget is hit-tested. A player without a pointer needs
    /// a way in, and this is it — see the bar focus in `daysengine`'s playback
    /// loop.
    FocusBar,
}

impl Action {
    /// The key this action is written under in `DaysEngine.ini`.
    pub fn key(self) -> &'static str {
        match self {
            Action::Up => "Up",
            Action::Down => "Down",
            Action::Left => "Left",
            Action::Right => "Right",
            Action::Confirm => "Confirm",
            Action::Cancel => "Cancel",
            Action::Quit => "Quit",
            Action::Pause => "Pause",
            Action::SeekBack => "SeekBack",
            Action::SeekForward => "SeekForward",
            Action::Restart => "Restart",
            Action::Auto => "Auto",
            Action::Faster => "Faster",
            Action::Slower => "Slower",
            Action::SkipToChoice => "SkipToChoice",
            Action::SaveMenu => "SaveMenu",
            Action::LoadMenu => "LoadMenu",
            Action::OptionMenu => "OptionMenu",
            Action::LeavePlayback => "LeavePlayback",
            Action::FocusBar => "FocusBar",
        }
    }

    /// Every action, in the order the template writes them.
    pub const ALL: [Action; 20] = [
        Action::Up,
        Action::Down,
        Action::Left,
        Action::Right,
        Action::Confirm,
        Action::Cancel,
        Action::Quit,
        Action::Pause,
        Action::SeekBack,
        Action::SeekForward,
        Action::Restart,
        Action::Auto,
        Action::Faster,
        Action::Slower,
        Action::SkipToChoice,
        Action::SaveMenu,
        Action::LoadMenu,
        Action::OptionMenu,
        Action::LeavePlayback,
        Action::FocusBar,
    ];

    /// Whether holding this down should repeat, the way a held arrow key does.
    ///
    /// Only the four directions. A held confirm that repeated would answer a
    /// choice box and then answer the next one.
    pub fn repeats(self) -> bool {
        matches!(
            self,
            Action::Up | Action::Down | Action::Left | Action::Right
        )
    }

    fn parse(key: &str) -> Option<Action> {
        Action::ALL
            .into_iter()
            .find(|a| a.key().eq_ignore_ascii_case(key))
    }
}

/// Which way an axis has to be pushed for it to count as pressed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Sign {
    /// Towards the axis' negative end: up on a stick's Y, left on its X.
    Negative,
    /// Towards its positive end. Triggers only ever go this way.
    Positive,
}

impl Sign {
    /// The sign a raw axis reading has.
    pub fn of(value: i16) -> Sign {
        if value < 0 {
            Sign::Negative
        } else {
            Sign::Positive
        }
    }
}

/// One physical thing a player can press.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Trigger {
    /// A key, by SDL's name for it, lowercased.
    Key(String),
    /// A controller button, by SDL's name for it, lowercased.
    Button(String),
    /// A controller axis pushed past the deadzone one way.
    Axis(String, Sign),
}

impl Trigger {
    /// Reads one trigger, or `None` for something that is not one at all.
    ///
    /// The alias list is short and deliberately so: it covers the spellings a
    /// person reaches for that SDL does not use, and nothing else. Everything
    /// else is SDL's own name, because inventing a second vocabulary for
    /// twenty-six controller buttons would mean maintaining it.
    pub fn parse(text: &str) -> Option<Trigger> {
        let text = text.trim().to_ascii_lowercase();
        if text.is_empty() {
            return None;
        }
        let Some(pad) = text.strip_prefix("pad:") else {
            let name = match text.as_str() {
                "enter" => "return",
                "esc" => "escape",
                "kp_enter" | "kpenter" | "numpad enter" => "keypad enter",
                other => other,
            };
            return Some(Trigger::Key(name.to_string()));
        };
        let pad = pad.trim();
        match pad.strip_prefix('-') {
            Some(axis) => Some(Trigger::Axis(axis.trim().to_string(), Sign::Negative)),
            None => match pad.strip_prefix('+') {
                Some(axis) => Some(Trigger::Axis(axis.trim().to_string(), Sign::Positive)),
                None => Some(Trigger::Button(pad.to_string())),
            },
        }
    }
}

impl fmt::Display for Trigger {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Trigger::Key(name) => write!(f, "{name}"),
            Trigger::Button(name) => write!(f, "pad:{name}"),
            Trigger::Axis(name, Sign::Negative) => write!(f, "pad:-{name}"),
            Trigger::Axis(name, Sign::Positive) => write!(f, "pad:+{name}"),
        }
    }
}

/// How far a stick has to move before it counts as pressed, of 32767.
///
/// SDL's own recommended figure for a left stick. A stick resting off-centre —
/// which every worn stick does — would otherwise hold a direction down for
/// ever.
pub const DEFAULT_DEADZONE: i16 = 8000;

/// How long a direction has to be held before it starts repeating, in ms.
pub const DEFAULT_REPEAT_DELAY: u32 = 400;

/// How often it repeats after that, in ms.
pub const DEFAULT_REPEAT_INTERVAL: u32 = 90;

/// How fast the right stick moves the pointer, in screen pixels per second.
pub const DEFAULT_CURSOR_SPEED: f32 = 900.0;

/// What each action is bound to.
#[derive(Debug, Clone, PartialEq)]
pub struct Bindings {
    /// One entry per action in [`Action::ALL`] order, so a lookup is an index.
    triggers: Vec<Vec<Trigger>>,
    /// How far a stick has to move to count. See [`DEFAULT_DEADZONE`].
    pub deadzone: i16,
    /// Milliseconds a held direction waits before repeating.
    pub repeat_delay: u32,
    /// Milliseconds between repeats after that.
    pub repeat_interval: u32,
    /// Pixels per second the right stick moves the pointer; 0 turns the
    /// stick-driven pointer off.
    pub cursor_speed: f32,
}

impl Default for Bindings {
    /// The original's keys, and a controller layout beside them.
    ///
    /// The keyboard half is exactly what this engine did before there was a
    /// binding table at all: arrows navigate, Return/Enter/Space confirm,
    /// Escape backs out of a menu and closes the game during playback, Space
    /// pauses, left and right seek five seconds and R restarts.
    ///
    /// Space pausing is not a second binding, and neither is A. It is
    /// [`Action::Confirm`]'s own fall-through: with nothing focused there is
    /// nothing to confirm, and a confirm with nothing to confirm pauses. One
    /// rule covers both, and it is why [`Action::Pause`] needs only the
    /// dedicated button for a player who wants one.
    ///
    /// The controller half is the layout every console reader uses: the face
    /// button under your thumb confirms, the one right of it goes back, the
    /// d-pad and the left stick navigate, the shoulders seek and the triggers
    /// change speed.
    fn default() -> Bindings {
        let of = |action: Action| -> Vec<Trigger> {
            let names: &[&str] = match action {
                Action::Up => &["up", "pad:dpup", "pad:-lefty"],
                Action::Down => &["down", "pad:dpdown", "pad:+lefty"],
                Action::Left => &["left", "pad:dpleft", "pad:-leftx"],
                Action::Right => &["right", "pad:dpright", "pad:+leftx"],
                Action::Confirm => &["return", "keypad enter", "space", "pad:a"],
                Action::Cancel => &["escape", "pad:b"],
                Action::Quit => &["escape"],
                Action::Pause => &["pad:start"],
                Action::SeekBack => &["pad:leftshoulder"],
                Action::SeekForward => &["pad:rightshoulder"],
                Action::Restart => &["r"],
                Action::Auto => &["a", "pad:y"],
                Action::Faster => &["pad:+righttrigger"],
                Action::Slower => &["pad:+lefttrigger"],
                Action::SkipToChoice => &["s", "pad:x"],
                Action::SaveMenu => &["f2"],
                Action::LoadMenu => &["f3"],
                Action::OptionMenu => &["f4", "pad:back"],
                Action::LeavePlayback => &[],
                Action::FocusBar => &["tab", "pad:rightstick"],
            };
            names.iter().filter_map(|n| Trigger::parse(n)).collect()
        };
        Bindings {
            triggers: Action::ALL.into_iter().map(of).collect(),
            deadzone: DEFAULT_DEADZONE,
            repeat_delay: DEFAULT_REPEAT_DELAY,
            repeat_interval: DEFAULT_REPEAT_INTERVAL,
            cursor_speed: DEFAULT_CURSOR_SPEED,
        }
    }
}

impl Bindings {
    /// What `action` is bound to.
    pub fn triggers(&self, action: Action) -> &[Trigger] {
        let at = Action::ALL.iter().position(|a| *a == action);
        at.map_or(&[], |at| self.triggers[at].as_slice())
    }

    /// Every binding, action by action, in [`Action::ALL`] order.
    pub fn all(&self) -> impl Iterator<Item = (Action, &[Trigger])> {
        Action::ALL
            .into_iter()
            .zip(self.triggers.iter().map(Vec::as_slice))
    }

    /// Replaces one action's whole list. Naming an action in the file is
    /// saying what it is, not adding to what it was.
    fn set(&mut self, action: Action, triggers: Vec<Trigger>) {
        if let Some(at) = Action::ALL.iter().position(|a| *a == action) {
            self.triggers[at] = triggers;
        }
    }

    /// Reads one `[Input]` line. Returns false for a key nothing here reads,
    /// so the caller can warn about it the way it warns about any other.
    ///
    /// `line` is only for the warnings.
    pub fn set_from_ini(&mut self, key: &str, value: &str, line: usize) -> bool {
        match key {
            "deadzone" => match value.trim().parse::<i16>() {
                Ok(n) if n >= 0 => self.deadzone = n,
                _ => log::warn!(
                    "{} line {line}: {value:?} is not a deadzone; 0 to 32767",
                    super::engine::FILE
                ),
            },
            "repeatdelay" => match value.trim().parse::<u32>() {
                Ok(n) => self.repeat_delay = n,
                _ => log::warn!(
                    "{} line {line}: {value:?} is not a number of milliseconds",
                    super::engine::FILE
                ),
            },
            "repeatinterval" => match value.trim().parse::<u32>() {
                // A zero interval would repeat once per pass round the loop,
                // which is hundreds of times a second.
                Ok(n) if n > 0 => self.repeat_interval = n,
                _ => log::warn!(
                    "{} line {line}: {value:?} is not a number of milliseconds above zero",
                    super::engine::FILE
                ),
            },
            "cursorspeed" => match value.trim().parse::<f32>() {
                Ok(n) if n >= 0.0 && n.is_finite() => self.cursor_speed = n,
                _ => log::warn!(
                    "{} line {line}: {value:?} is not a speed in pixels per second",
                    super::engine::FILE
                ),
            },
            _ => {
                let Some(action) = Action::parse(key) else {
                    return false;
                };
                let mut triggers = Vec::new();
                for part in value.split(',') {
                    if part.trim().is_empty() {
                        continue;
                    }
                    match Trigger::parse(part) {
                        Some(trigger) => triggers.push(trigger),
                        None => log::warn!(
                            "{} line {line}: {part:?} is not a key or a pad button",
                            super::engine::FILE
                        ),
                    }
                }
                self.set(action, triggers);
            }
        }
        true
    }

    /// The `[Input]` section as it goes into the template, every default
    /// spelled out.
    pub fn template_body(&self) -> String {
        let mut out = String::new();
        let width = Action::ALL
            .into_iter()
            .map(|a| a.key().len())
            .max()
            .unwrap_or(0);
        for (action, triggers) in self.all() {
            let list = triggers
                .iter()
                .map(Trigger::to_string)
                .collect::<Vec<_>>()
                .join(", ");
            out.push_str(&format!("{:<width$} = {list}\n", action.key()));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Naming an action replaces its list rather than adding to it, so a
    /// player who binds one key gets one key.
    #[test]
    fn a_named_action_replaces_its_whole_list() {
        let mut bindings = Bindings::default();
        assert!(bindings.triggers(Action::Confirm).len() > 1);
        assert!(bindings.set_from_ini("confirm", "pad:x", 1));
        assert_eq!(
            bindings.triggers(Action::Confirm),
            [Trigger::Button("x".into())]
        );
        // And an action nobody named is untouched.
        assert_eq!(
            bindings.triggers(Action::Cancel),
            Bindings::default().triggers(Action::Cancel)
        );
    }

    /// An empty value is how a player turns an action off.
    #[test]
    fn an_empty_value_unbinds() {
        let mut bindings = Bindings::default();
        assert!(bindings.set_from_ini("quit", "", 1));
        assert!(bindings.triggers(Action::Quit).is_empty());
    }

    /// The three shapes of trigger, and the sign that makes a stick a button.
    #[test]
    fn a_trigger_is_a_key_a_button_or_a_pushed_axis() {
        assert_eq!(Trigger::parse("Up"), Some(Trigger::Key("up".into())));
        assert_eq!(Trigger::parse(" pad:A "), Some(Trigger::Button("a".into())));
        assert_eq!(
            Trigger::parse("pad:-lefty"),
            Some(Trigger::Axis("lefty".into(), Sign::Negative))
        );
        assert_eq!(
            Trigger::parse("pad:+righttrigger"),
            Some(Trigger::Axis("righttrigger".into(), Sign::Positive))
        );
        // And every one of them round-trips through the template.
        for text in ["up", "pad:a", "pad:-lefty", "pad:+righttrigger"] {
            assert_eq!(Trigger::parse(text).unwrap().to_string(), text);
        }
    }

    /// A held confirm must not repeat: it would answer a choice box and then
    /// answer whatever went up next.
    #[test]
    fn only_the_directions_repeat() {
        for action in Action::ALL {
            assert_eq!(
                action.repeats(),
                matches!(
                    action,
                    Action::Up | Action::Down | Action::Left | Action::Right
                ),
                "{}",
                action.key()
            );
        }
    }

    /// The template is the defaults, so writing it and writing nothing mean
    /// the same thing — the same rule `DaysEngine.ini` as a whole follows.
    #[test]
    fn the_template_parses_back_to_the_defaults() {
        let defaults = Bindings::default();
        let mut read = Bindings::default();
        // Start from something else, so a parse that did nothing would fail.
        for action in Action::ALL {
            read.set(action, Vec::new());
        }
        for (number, line) in defaults.template_body().lines().enumerate() {
            let (key, value) = line.split_once('=').expect("every line is key = value");
            assert!(read.set_from_ini(key.trim().to_ascii_lowercase().as_str(), value, number));
        }
        assert_eq!(read, defaults);
    }
}
