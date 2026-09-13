//! `daysengine` — plays a School Days HQ script.
//!
//! Drop this next to `SCHOOLDAYS HQ.exe` and run it. It reads the start script
//! out of the game's own `STARTSCRIPT.INI`, so with no arguments it does what
//! the game would do: put up the title screen, and play from there.
//!
//! Naming a script on the command line skips the menus and plays it directly,
//! which is the development path.

#![forbid(unsafe_code)]

use anyhow::{bail, Context, Result};
use days_font::Font;
use days_script::{Frame, Script, FPS};
use daysengine::install::binding::{Action as Control, Bindings, Sign, Trigger};
use daysengine::install::config::{Channel, Config, Flag};
use daysengine::install::engine::Settings;
use daysengine::install::progress::Progress;
use daysengine::install::save::FlagStore;
use daysengine::install::vfs::Vfs;
use daysengine::media::AudioBuffer;
use daysengine::media::ImageScaler;
use daysengine::playback::lipsync::compose_mouths;
use daysengine::playback::som::{self, Device};
use daysengine::ui::bar::{self, Bar};
use daysengine::ui::comment;
use daysengine::ui::ending;
use daysengine::ui::menu::{Action, Menu, Mode, SaveState, Session, SystemSe};
use daysengine::ui::options::{self, Dir, Display, Som};
use daysengine::ui::replay::{self, Scenes};
use daysengine::ui::saveload::{self, Slots};
use daysengine::ui::screen::Resolution;
use daysengine::ui::select::{self, Choice, Input, Select};
use daysengine::{install::ini::Ini, playback::text, Mixer, Stage};
use sdl3::audio::{AudioCallback, AudioFormat, AudioSpec, AudioStream};
use sdl3::event::Event;
use sdl3::gamepad::{Axis, Button, Gamepad};
use sdl3::joystick::JoystickId;
use sdl3::keyboard::Keycode;
use sdl3::mouse::MouseButton;
use sdl3::pixels::{Color, PixelFormat};
use sdl3::render::{BlendMode, Canvas, FRect, ScaleMode, Texture, TextureCreator};
use sdl3::video::{Window, WindowContext};
use sdl3::EventPump;
use sdl3::GamepadSubsystem;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Presentation size. Movies decode at 800x452 and still backgrounds are
/// 800x450; everything is drawn into this box and the box is letterboxed into
/// the window, so the two never disagree about aspect.
const STAGE_WIDTH: u32 = 800;
const STAGE_HEIGHT: u32 = 452;

/// Pulls mixed audio for SDL.
struct MixerSource {
    mixer: Mixer,
    scratch: Vec<f32>,
}

impl AudioCallback<f32> for MixerSource {
    fn callback(&mut self, stream: &mut AudioStream, requested: i32) {
        let n = requested.max(0) as usize;
        self.scratch.resize(n, 0.0);
        self.mixer.render(&mut self.scratch);
        // A failed queue push means the device went away; the next callback (or
        // the event loop) will surface that. Dropping a block of audio is the
        // right response here — blocking the callback would stutter everything.
        if let Err(err) = stream.put_data_f32(&self.scratch) {
            log::warn!("audio device rejected a block: {err}");
        }
    }
}

/// Decoded audio, kept so a sound is not re-decoded every time it is played.
///
/// The menu's click and move sounds fire many times a second while the pointer
/// crosses widgets, and each one is a Vorbis stream.
#[derive(Default)]
struct Sounds {
    cache: HashMap<String, Option<Arc<AudioBuffer>>>,
}

impl Sounds {
    /// Decodes `path`, or returns the already-decoded buffer.
    ///
    /// A sound that will not load is remembered as missing and logged once: a
    /// menu that falls silent is a much better failure than one that stops.
    fn get(&mut self, vfs: &Vfs, path: &str) -> Option<Arc<AudioBuffer>> {
        if let Some(cached) = self.cache.get(path) {
            return cached.clone();
        }
        let buffer = vfs
            .resolve_as(path, "ogg")
            .and_then(|h| vfs.read(h).ok())
            .and_then(|bytes| match daysengine::media::decode_audio(bytes) {
                Ok(buffer) => Some(Arc::new(buffer)),
                Err(err) => {
                    log::warn!("decoding {path}: {err}");
                    None
                }
            });
        if buffer.is_none() {
            log::warn!("system sound {path} is missing");
        }
        self.cache.insert(path.to_string(), buffer.clone());
        buffer
    }
}

/// The system sounds `FILMENGINE.INI` names, resolved once.
struct SystemSounds {
    paths: HashMap<&'static str, String>,
}

impl SystemSounds {
    fn from_ini(film: &Ini) -> SystemSounds {
        let paths = SystemSe::ALL
            .into_iter()
            .filter_map(|se| se.path(film).map(|p| (se.key(), p.to_string())))
            .collect();
        SystemSounds { paths }
    }

    /// Plays one, on the channel the menus own.
    ///
    /// Not one of the script's five `[PlaySe]` slots: a menu the control bar
    /// opened is up over a script that is only paused, and its sounds are a
    /// different object in the original — see [`Mixer::play_system_se`].
    fn play(&self, se: SystemSe, vfs: &Vfs, sounds: &mut Sounds, mixer: &Mixer) {
        let Some(path) = self.paths.get(se.key()) else {
            return;
        };
        if let Some(buffer) = sounds.get(vfs, path) {
            mixer.play_system_se(buffer);
        }
    }
}

/// How long a rumble effect is asked for, in milliseconds.
///
/// SDL effects lapse on their own, which is the right default for a game that
/// might crash — nobody wants a controller left buzzing. A `[MoveSom]` window
/// can run for seconds, so the effect is renewed while it lasts; this is the
/// length of one renewal and [`RUMBLE_RENEW`] is how often.
const RUMBLE_MS: u32 = 1000;

/// How often a rumble effect is renewed, comfortably inside [`RUMBLE_MS`].
const RUMBLE_RENEW: Duration = Duration::from_millis(400);

/// The controllers SDL has found, and which one is holding a level.
///
/// This is the [`Device`] behind the Option screen's SOMCON tab. The tab was
/// written for a toy on a COM port — nine `Port number` buttons, a find
/// button, a release button and a test — and every one of those questions has
/// an answer here: a port is a connected controller, finding one is asking
/// each in turn whether it can rumble, and a level is a level. See
/// [`daysengine::playback::som`] for the levels themselves and where they come
/// from.
struct Pads {
    /// `None` when SDL could not start its gamepad subsystem at all, which
    /// costs the player controller support and nothing else.
    subsystem: Option<GamepadSubsystem>,
    /// Every controller SDL has opened, in the order it announced them. The
    /// tab's `Port number` buttons stand for this list.
    open: Vec<(JoystickId, Gamepad)>,
    /// Which of them is holding the level, or `None` for no port.
    held: Option<usize>,
    /// The level last asked for, 0 to 255.
    level: u8,
    /// `[Rumble] Strength`, a percentage of what the script asked for.
    strength: u16,
    /// When the effect was last sent, so it can be renewed before it lapses.
    sent: Option<Instant>,
}

impl Pads {
    fn new(subsystem: Option<GamepadSubsystem>, strength: u16) -> Pads {
        let mut pads = Pads {
            subsystem,
            open: Vec::new(),
            held: None,
            level: 0,
            strength,
            sent: None,
        };
        // Whatever is already plugged in. SDL only sends an added event for a
        // controller that arrives after the subsystem is up.
        match pads.subsystem.as_ref().map(GamepadSubsystem::gamepads) {
            Some(Ok(ids)) => {
                for id in ids {
                    pads.added(id);
                }
            }
            Some(Err(err)) => log::warn!("asking SDL for the controllers: {err}"),
            None => {}
        }
        pads
    }

    /// Opens a controller SDL has just announced.
    fn added(&mut self, id: JoystickId) {
        if self.open.iter().any(|(known, _)| *known == id) {
            return;
        }
        let Some(subsystem) = &self.subsystem else {
            return;
        };
        match subsystem.open(id) {
            Ok(pad) => {
                log::info!(
                    "controller {}: {}",
                    self.open.len() + 1,
                    pad.name().unwrap_or_else(|| "unnamed".to_string())
                );
                self.open.push((id, pad));
            }
            Err(err) => log::warn!("opening controller {id:?}: {err}"),
        }
    }

    /// Forgets one that has gone away, and lets its port go with it.
    fn removed(&mut self, id: JoystickId) {
        let Some(at) = self.open.iter().position(|(known, _)| *known == id) else {
            return;
        };
        self.open.remove(at);
        match self.held {
            // The port in hand was unplugged. Nothing is holding a level any
            // more, which is exactly what the DLL's `+0x31c` going to zero
            // means to every screen that reads it.
            Some(held) if held == at => {
                self.held = None;
                self.level = 0;
                self.sent = None;
            }
            Some(held) if held > at => self.held = Some(held - 1),
            _ => {}
        }
    }

    /// A raw axis reading from the first controller, for the stick-driven
    /// pointer.
    fn axis(&self, axis: Axis) -> i16 {
        self.open.first().map_or(0, |(_, pad)| pad.axis(axis))
    }

    /// Whether any controller is connected at all.
    fn any(&self) -> bool {
        !self.open.is_empty()
    }

    /// Which port is holding the level, if one is. The engine's answer to
    /// `_GetSomFlag@0`.
    fn holding(&self) -> Option<usize> {
        self.held
    }

    /// Renews the effect before it lapses, and lets it lapse once the level is
    /// zero.
    fn renew(&mut self, now: Instant) {
        if self.level == 0 || self.held.is_none() {
            return;
        }
        if self
            .sent
            .is_some_and(|at| now.duration_since(at) < RUMBLE_RENEW)
        {
            return;
        }
        self.send();
    }

    /// Puts the level on the controller in hand. A controller that refuses is
    /// not an error worth stopping for: the player loses the rumble and keeps
    /// the game.
    fn send(&mut self) {
        let magnitude = som::scaled(self.level, self.strength);
        let Some((_, pad)) = self.held.and_then(|at| self.open.get_mut(at)) else {
            return;
        };
        // Both motors together. The original's device has one level and one
        // motor; splitting it across a controller's two would be inventing a
        // second number the scripts never carried.
        if let Err(err) = pad.set_rumble(magnitude, magnitude, RUMBLE_MS) {
            log::warn!("the controller would not take a level: {err}");
        }
        self.sent = Some(Instant::now());
    }

    /// Whether a controller will take a level at all, asked the only way that
    /// does not need `unsafe`: by sending it nothing and seeing whether it was
    /// accepted.
    ///
    /// SDL answers this through a property, and reading a property means
    /// `SDL_GetGamepadProperties` — which the safe binding only offers behind
    /// `unsafe`, and `src/media` is the only module in this engine allowed
    /// that. `SDL_RumbleGamepad` reports the same refusal through its return
    /// value, so this asks it that way.
    fn takes_a_level(&mut self, at: usize) -> bool {
        self.open
            .get_mut(at)
            .is_some_and(|(_, pad)| pad.set_rumble(0, 0, 1).is_ok())
    }
}

impl Device for Pads {
    fn ports(&self) -> Vec<String> {
        self.open
            .iter()
            .map(|(_, pad)| pad.name().unwrap_or_else(|| "unnamed".to_string()))
            .collect()
    }

    fn open(&mut self, port: usize) -> bool {
        // `FUN_10021070` opens the port and its caller sends `s00` straight
        // away, so a device that was already moving stops as it is taken. The
        // capability check above has already sent this one a zero.
        if !self.takes_a_level(port) {
            return false;
        }
        self.stop();
        self.held = Some(port);
        self.level = 0;
        self.sent = None;
        true
    }

    fn detect(&mut self) -> Option<usize> {
        // `FUN_10007850` tries indices 0 upward and keeps the first that
        // answers. The list here is shorter than its nine more often than not.
        (0..self.open.len().min(daysengine::ui::options::SOM_PORTS)).find(|port| self.open(*port))
    }

    fn close(&mut self) {
        self.stop();
        self.held = None;
    }

    fn set_level(&mut self, level: u8) {
        // The original sends `s%02x` on every tick the statement covers and
        // the device makes nothing of the repeats. A rumble effect is not
        // idempotent that way — re-sending it restarts it — so the level is
        // sent when it changes and renewed on [`Pads::renew`]'s clock.
        if self.level == level {
            return;
        }
        self.level = level;
        self.send();
    }

    fn stop(&mut self) {
        self.level = 0;
        self.sent = None;
        if let Some((_, pad)) = self.held.and_then(|at| self.open.get_mut(at)) {
            if let Err(err) = pad.set_rumble(0, 0, 0) {
                log::warn!("the controller would not stop: {err}");
            }
        }
    }
}

/// The binding table, resolved against SDL and tracking what is held down.
///
/// [`daysengine::install::binding`] is the table itself and knows nothing
/// about SDL, which is the right place for it — it is a file in the player's
/// install, not a window. This is the other half: SDL's names looked up once
/// at startup, the deadzone that turns a stick into a button, and the repeat
/// a held direction needs because a controller has no key repeat of its own.
struct Controls {
    /// Lowercased SDL key name to the actions it presses.
    keys: HashMap<String, Vec<Control>>,
    buttons: HashMap<Button, Vec<Control>>,
    axes: HashMap<(Axis, Sign), Vec<Control>>,
    /// Which way each axis is currently pushed, past the deadzone.
    pushed: HashMap<Axis, Sign>,
    /// Directions being held, each with the moment it next repeats.
    repeating: Vec<(Control, Instant)>,
    deadzone: i16,
    delay: Duration,
    interval: Duration,
    /// Pixels a second the right stick moves the pointer; 0 for not at all.
    cursor_speed: f32,
}

impl Controls {
    /// Resolves a binding table. A name SDL does not know is a warning and a
    /// trigger that never fires, which is the rule every unreadable setting in
    /// `DaysEngine.ini` follows.
    fn new(bindings: &Bindings) -> Controls {
        let mut controls = Controls {
            keys: HashMap::new(),
            buttons: HashMap::new(),
            axes: HashMap::new(),
            pushed: HashMap::new(),
            repeating: Vec::new(),
            deadzone: bindings.deadzone,
            delay: Duration::from_millis(u64::from(bindings.repeat_delay)),
            interval: Duration::from_millis(u64::from(bindings.repeat_interval)),
            cursor_speed: bindings.cursor_speed,
        };
        for (action, triggers) in bindings.all() {
            for trigger in triggers {
                match trigger {
                    Trigger::Key(name) => {
                        controls.keys.entry(name.clone()).or_default().push(action)
                    }
                    Trigger::Button(name) => match Button::from_string(name) {
                        Some(button) => controls.buttons.entry(button).or_default().push(action),
                        None => log::warn!("{trigger} is not a controller button SDL knows"),
                    },
                    Trigger::Axis(name, sign) => match Axis::from_string(name) {
                        Some(axis) => controls.axes.entry((axis, *sign)).or_default().push(action),
                        None => log::warn!("{trigger} is not a controller axis SDL knows"),
                    },
                }
            }
        }
        controls
    }

    /// The actions one SDL event presses.
    ///
    /// Nothing in this engine acts on a release, so a release produces no
    /// actions — it only takes the repeat off whatever was held.
    fn take(&mut self, event: &Event, now: Instant) -> Vec<Control> {
        match event {
            // SDL's own key repeat already does for the keyboard what
            // [`Controls::due`] does for a controller, so a repeat is a press
            // and registers nothing further.
            Event::KeyDown {
                keycode: Some(key),
                repeat,
                ..
            } => {
                let actions = self.for_key(*key);
                if !repeat {
                    self.hold(&actions, now);
                }
                actions
            }
            Event::KeyUp {
                keycode: Some(key), ..
            } => {
                let actions = self.for_key(*key);
                self.let_go(&actions);
                Vec::new()
            }
            Event::GamepadButtonDown { button, .. } => {
                let actions = self.buttons.get(button).cloned().unwrap_or_default();
                self.hold(&actions, now);
                actions
            }
            Event::GamepadButtonUp { button, .. } => {
                let actions = self.buttons.get(button).cloned().unwrap_or_default();
                self.let_go(&actions);
                Vec::new()
            }
            Event::GamepadAxisMotion { axis, value, .. } => {
                let now_pushed = (value.unsigned_abs() >= self.deadzone.unsigned_abs())
                    .then(|| Sign::of(*value));
                let was = self.pushed.get(axis).copied();
                if was == now_pushed {
                    return Vec::new();
                }
                if let Some(sign) = was {
                    let actions = self.axes.get(&(*axis, sign)).cloned().unwrap_or_default();
                    self.let_go(&actions);
                }
                match now_pushed {
                    Some(sign) => {
                        self.pushed.insert(*axis, sign);
                        let actions = self.axes.get(&(*axis, sign)).cloned().unwrap_or_default();
                        self.hold(&actions, now);
                        actions
                    }
                    None => {
                        self.pushed.remove(axis);
                        Vec::new()
                    }
                }
            }
            _ => Vec::new(),
        }
    }

    fn for_key(&self, key: Keycode) -> Vec<Control> {
        self.keys
            .get(&key.name().to_ascii_lowercase())
            .cloned()
            .unwrap_or_default()
    }

    /// Starts the repeat clock on whichever of these repeat at all.
    fn hold(&mut self, actions: &[Control], now: Instant) {
        for action in actions {
            if !action.repeats() || self.repeating.iter().any(|(held, _)| held == action) {
                continue;
            }
            self.repeating.push((*action, now + self.delay));
        }
    }

    fn let_go(&mut self, actions: &[Control]) {
        self.repeating.retain(|(held, _)| !actions.contains(held));
    }

    /// The held directions whose repeat has come round again.
    fn due(&mut self, now: Instant) -> Vec<Control> {
        let mut out = Vec::new();
        for (action, at) in &mut self.repeating {
            if now >= *at {
                *at = now + self.interval;
                out.push(*action);
            }
        }
        out
    }

    /// Nothing is held any more — for a loop that is handing over to another
    /// one, so a direction held across the change does not repeat into it.
    fn clear(&mut self) {
        self.repeating.clear();
        self.pushed.clear();
    }

    /// How far the right stick moves the pointer this pass, in window pixels.
    ///
    /// Squared response, so small movements are fine and a stick pushed to its
    /// stop is fast. Returns `None` while the stick is inside the deadzone,
    /// which is what leaves the pointer alone.
    fn cursor(&self, pads: &Pads, elapsed: Duration) -> Option<(f32, f32)> {
        if self.cursor_speed <= 0.0 || !pads.any() {
            return None;
        }
        let read = |axis| {
            let raw = f32::from(pads.axis(axis)) / f32::from(i16::MAX);
            if raw.abs() * f32::from(i16::MAX) < f32::from(self.deadzone) {
                0.0
            } else {
                raw * raw.abs()
            }
        };
        let (x, y) = (read(Axis::RightX), read(Axis::RightY));
        if x == 0.0 && y == 0.0 {
            return None;
        }
        let step = self.cursor_speed * elapsed.as_secs_f32();
        Some((x * step, y * step))
    }
}

/// The dialogue currently on screen, cached on the line it was built from.
///
/// Each line is uploaded at the layout's own size; where it lands and how far
/// it is stretched comes from `playback::text::place`, so the texture's own
/// dimensions are not needed again once it is made.
///
/// Keyed on the script's own line rather than on the wrapped result, so a pass
/// round the loop that changes nothing does not re-wrap it to find that out.
struct DialogueBlock<'r> {
    source: String,
    lines: Vec<String>,
    drawn: Vec<Texture<'r>>,
}

/// The choice labels currently on screen, cached on the labels and which of
/// them is lit.
///
/// Same reason as [`DialogueBlock`], and more urgently: this was rendering both
/// labels and creating a texture for each of them on every pass round the loop,
/// hundreds of times a second, for a box that changes when the pointer moves.
struct ChoiceLabels<'r> {
    labels: Vec<String>,
    /// The colour each label was rendered in, without its alpha: the alpha is
    /// a modulation on the texture, so a fade is not a re-render.
    colours: Vec<Option<[u8; 3]>>,
    drawn: Vec<(u32, u32, Texture<'r>)>,
}

/// The movie texture and which picture is in it.
///
/// Uploaded once per picture rather than once per pass round the loop, which
/// spins far faster than the 24 fps a clip is played at. `clip` and `index` are
/// what say two passes are looking at the same picture; `window` is the size
/// the decoder was asked for, so a resize counts as a different one.
struct MovieFrame<'r> {
    clip: String,
    index: u64,
    window: (u32, u32),
    /// The texture's own size, which is what the decoder actually gave.
    size: (u32, u32),
    texture: Texture<'r>,
}

/// The background texture, and everything that decided the pixels in it.
///
/// The mouth patches are part of the key because they are part of the image:
/// they are written into the background's own surface before it is scaled, the
/// way `FUN_00444b80` writes them into the original's, so a mouth changing
/// image means a new background. Everything else about a still changes rarely —
/// a background lasts seconds — and a mouth flaps eight times a second, which
/// is still far short of once a pass.
struct StillFrame<'r> {
    path: String,
    size: (u32, u32),
    /// Each patch rectangle and which of the three images it is showing.
    mouths: Vec<(usize, usize, usize, usize, usize)>,
    texture: Texture<'r>,
}

/// Everything the two loops both need.
struct Player<'a> {
    /// `DaysEngine.ini`: the choices that are the engine's, not the game's.
    settings: Settings,
    vfs: &'a Vfs,
    font: &'a Font,
    mixer: &'a Mixer,
    sounds: Sounds,
    system_se: SystemSounds,
    /// The user's own `SysMenuSDHQ.dll`, which holds every widget table.
    dll: Vec<u8>,
    /// What the player has unlocked, out of their `Save/GlobalFlag.DAT`.
    flags: FlagStore,
    /// The install root, which is where `Config.DAT` is written back.
    game: PathBuf,
    /// `FILMENGINE.INI`, which names the choice box's hit maps among much else.
    film: &'a Ini,
    /// SDL's text input, which is what carries an IME into the save-comment
    /// dialog. Off except while that dialog is up.
    text_input: sdl3::keyboard::TextInputUtil,
    /// The display mode in force, which the Option screen's Def tab both shows
    /// and changes. The original keeps the same pair on the engine object and
    /// answers them through host `+0xb8` and `+0xbc`.
    display: Display,
    /// `Config.DAT`'s `TypeMiniNote`, which picks the 1024x576 art over the
    /// 1280x720 art when full screen. Host `+0xc8`, via `DAT_0050b314`.
    mini_note: bool,
    /// Whether a choice box nobody answers should take the answer the loaded
    /// slot recorded, which is what the replay screen's play-data list starts.
    ///
    /// The film object's `+0x1e0`: host `+0x94` raises it (`FUN_0042bf10`) and
    /// `+0x98` reports it. `FUN_00431740` reads it at every box — an unanswered
    /// one resolves to `FUN_00428a80`, the recorded answer, and the moment the
    /// player answers one themselves it comes back down.
    following_record: bool,
    /// The controllers, and whichever one the SOMCON tab has taken. This is
    /// the [`Device`] the game's own `[MoveSom]` statements drive.
    pads: Pads,
    /// The binding table resolved against SDL, and what is held down.
    controls: Controls,
    /// The index the last choice box settled on, which is what a replay's
    /// branch table is read by. The film object's `+0x1f8`: `FUN_004388c0`
    /// sets it to -2 once, when the object is constructed, and only a settling
    /// choice box writes it after that — so it outlives the script it was made
    /// in, and lives as long as the session does.
    last_choice: i32,
}

impl Player<'_> {
    /// The art set to load for the mode in force. See
    /// [`Resolution::for_display`].
    ///
    /// Whole-number scaling takes the **native** set instead, and that is a
    /// deliberate departure from the recovered rule. The four sets are one
    /// layout at four sizes — `FORMATS.md` shows the 1024x576 and 1280x720 hit
    /// maps are the 800x450 one scaled by 1.28 and 1.6, and there is only ever
    /// one `.PNG` behind them — so composing at 1.6 and then scaling that by a
    /// whole number would put a resample back in the middle of the one path
    /// whose whole point is not having one. Composing at 1.0 loses nothing:
    /// same art, same layout, same widgets, and the hit map that matches.
    fn resolution(&self) -> Resolution {
        if self.settings.pixel_perfect() {
            return if self.display.wide {
                Resolution::Wide
            } else {
                Resolution::Standard
            };
        }
        Resolution::for_display(self.display.wide, self.display.full_screen, self.mini_note)
    }

    /// Whether the picture is scaled by a whole number. See
    /// [`daysengine::install::engine::Settings::pixel_perfect`].
    fn whole_pixels(&self) -> bool {
        self.settings.pixel_perfect()
    }
}

fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let mut args = std::env::args().skip(1);
    let mut game: Option<PathBuf> = None;
    let mut script_name: Option<String> = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--game" | "-g" => game = args.next().map(PathBuf::from),
            "--help" | "-h" => {
                println!("daysengine [--game <dir>] [<script name>]");
                println!();
                println!("With no script name, starts where STARTSCRIPT.INI says — the");
                println!("title screen, unless it names another start mode.");
                println!("Menu:   arrows or pointer to choose, Enter or click to confirm,");
                println!("        Esc to back out.");
                println!("Script: Space pause, Left/Right seek 5s, R restart, Esc quit.");
                println!("A controller works everywhere: d-pad or left stick to choose, A to");
                println!("confirm, B to back out, Up for the control bar, right stick for the");
                println!("pointer. During playback A pauses, unless the bar has the selection");
                println!("or a choice is up. Every binding is in DaysEngine.ini under [Input].");
                return Ok(());
            }
            other if other.starts_with('-') => bail!("unknown option {other}"),
            other => script_name = Some(other.to_string()),
        }
    }

    let game = match game {
        Some(dir) => dir,
        None => discover_game_dir()?,
    };
    let vfs = Vfs::mount(&game)?;
    log::info!("ffmpeg {}", daysengine::media::ffmpeg_version());

    let start = Ini::parse_bytes(&vfs.read_path("Ini/STARTSCRIPT.INI")?);
    let film = Ini::parse_bytes(&vfs.read_path("Ini/FILMENGINE.INI")?);
    let english = film.get_bool("UseEnglish").unwrap_or(false);

    let font = Font::parse(
        vfs.read_path("System/System/FONTDATA_ENG.DAT")
            .or_else(|_| vfs.read_path("System/System/FONTDATA.DAT"))?,
    )?;

    // The engine's own settings, which are not the game's — `DaysEngine.ini`
    // beside this binary. The UI kernel is fixed here, before a single screen
    // is composed, because every scaler built afterwards reads it.
    let settings = Settings::load();
    daysengine::playback::scale::set_kernel(settings.ui_scaler.kernel());

    let sdl = sdl3::init().map_err(|e| anyhow::anyhow!("SDL init: {e}"))?;
    let video = sdl.video().map_err(|e| anyhow::anyhow!("SDL video: {e}"))?;
    let window = video
        .window("DaysEngine", STAGE_WIDTH, STAGE_HEIGHT)
        .position_centered()
        .resizable()
        .build()
        .context("creating window")?;
    // Present in step with the display rather than as fast as the loop can go.
    // A movie is 24 frames a second and the bar's ramps are a few hundred
    // milliseconds; past the rate the panel can show them, a present is work
    // nobody sees, and an unsynchronised one tears. This is a hint rather than
    // a call because the safe SDL binding does not expose `SDL_SetRenderVSync`,
    // and `src/media` is the only module here allowed to reach past it —
    // [`Cadence`] paces the loop for the case where it is refused.
    sdl3::hint::set("SDL_RENDER_VSYNC", "1");
    let mut canvas = window.into_canvas();
    let creator = canvas.texture_creator();

    let audio = sdl.audio().map_err(|e| anyhow::anyhow!("SDL audio: {e}"))?;
    let mixer = Mixer::new();
    let spec = AudioSpec {
        freq: Some(daysengine::media::SAMPLE_RATE as i32),
        channels: Some(daysengine::media::CHANNELS as i32),
        format: Some(AudioFormat::f32_sys()),
    };
    let stream = audio
        .open_playback_stream(
            &spec,
            MixerSource {
                mixer: mixer.clone(),
                scratch: Vec::new(),
            },
        )
        .map_err(|e| anyhow::anyhow!("opening audio device: {e}"))?;
    stream
        .resume()
        .map_err(|e| anyhow::anyhow!("starting audio: {e}"))?;

    let mut events = sdl
        .event_pump()
        .map_err(|e| anyhow::anyhow!("SDL event pump: {e}"))?;

    // The display mode the player left the game in. `Config.DAT` carries all
    // three keys the original reads, and `FUN_0040cbb0` is where it reads them:
    // `DisplayType` is the aspect — 0 gives the 4:3 800x600 back buffer, 1 the
    // 800x450 wide one — `WindowMode` is windowed versus full screen, and
    // `TypeMiniNote` picks the 1024x576 art over the 1280x720 art.
    //
    // `WindowMode` is the argument `FUN_0040db00` is called with by all three
    // of its callers, and that function is the one that sets the window style:
    // 1 gives `WS_POPUP | WS_VISIBLE` at `HWND_TOPMOST` over the monitor's own
    // rectangle, anything else the captioned window at the saved
    // `WindowPosX`/`WindowPosY`. So 1 is full screen and 0 is windowed, and
    // the game reopens in whichever the player left it in.
    // Controllers. A subsystem that will not start costs the player controller
    // support and nothing else, so it is a warning and an empty list — the
    // same rule a missing asset follows.
    let pads = Pads::new(
        match sdl.gamepad() {
            Ok(subsystem) => Some(subsystem),
            Err(err) => {
                log::warn!("SDL gamepad: {err} — no controller support this session");
                None
            }
        },
        settings.rumble_strength,
    );
    let controls = Controls::new(&settings.bindings);

    let boot_config = Config::load(&game);
    let mut player = Player {
        settings,
        pads,
        controls,
        game: game.clone(),
        display: Display {
            wide: boot_config
                .get("DisplayType")
                .is_none_or(|v| v.trim() != "0"),
            full_screen: boot_config
                .get("WindowMode")
                .is_some_and(|v| v.trim() == "1"),
        },
        mini_note: boot_config
            .get("TypeMiniNote")
            .is_some_and(|v| v.trim() != "0"),
        following_record: false,
        last_choice: replay::NO_CHOICE,
        // Started only while the save-comment dialog is up, so ordinary key
        // presses stay key presses everywhere else.
        text_input: video.text_input(),
        vfs: &vfs,
        font: &font,
        mixer: &mixer,
        sounds: Sounds::default(),
        film: &film,
        system_se: SystemSounds::from_ini(&film),
        flags: daysengine::install::save::load_flags(&game, &film),
        // The widget tables are only needed for menus. A missing DLL is not
        // fatal to playing a script, so this is reported and left empty.
        dll: match std::fs::read(game.join("SysMenuSDHQ.dll")) {
            Ok(bytes) => bytes,
            Err(err) => {
                log::warn!(
                    "reading {}: {err} — the menus need it and will be skipped",
                    game.join("SysMenuSDHQ.dll").display()
                );
                Vec::new()
            }
        },
    };

    if player.display.full_screen {
        // `Config.DAT` said full screen. A refusal is not fatal — the window is
        // already open and playable — so it is logged and the player is left
        // windowed, with the setting corrected to match what they can see.
        if let Err(err) = canvas.window_mut().set_fullscreen(true) {
            log::warn!("could not reopen full screen: {err}");
            player.display.full_screen = false;
        }
    }

    // A named script is the development path: play it and stop. Otherwise the
    // game's own start mode decides, and the title screen owns the session.
    let wanted = script_name.clone().unwrap_or_else(|| start_script(&start));
    let menus = script_name.is_none()
        && start.get("StartMode").unwrap_or("Title") == "Title"
        && !player.dll.is_empty();

    // The branch graph, out of the player's own `RouteProcSDHQ.dll`. Without
    // it a script plays and stops, which is what the engine did before the
    // route system was recovered — so a missing or unreadable DLL is a warning
    // and not a refusal to start.
    let mut progress = match std::fs::read(game.join("RouteProcSDHQ.dll"))
        .map_err(|e| e.to_string())
        .and_then(|dll| Progress::load(&vfs, &dll, player.flags.clone()).map_err(|e| e.to_string()))
    {
        Ok(p) => Some(p),
        Err(err) => {
            log::warn!("the branch graph is unavailable, so scripts will not chain: {err}");
            None
        }
    };

    let mut next = wanted.clone();
    // Set when the last script chained into this one, in which case the
    // position is already the graph's and must not be looked up again: a
    // script that several routes list would resolve back to the first of them.
    let mut chained = false;
    // The replay scene being played and how far through its list, or `None`
    // for ordinary playback. The original keeps the same pair on the replay
    // module: the scene at `+0x2a8` and the step at `+0x2ac`.
    let mut replaying: Option<(replay::Run, usize)> = None;
    // Set while the skip button is still looking for a choice, which makes the
    // loop below pass over scripts that raise none instead of playing them.
    let mut chasing_choice = false;
    let mut passed_over = 0usize;
    loop {
        if menus && !chained {
            // The menus reached from here are the whole screen. The control
            // bar's own buttons open theirs over playback, from inside the
            // playback loop, and never come through this call.
            match run_menu(
                &mut player,
                &mut canvas,
                &creator,
                &mut events,
                &start,
                MenuEntry::Title,
                progress.as_mut(),
            )? {
                Outcome::Quit => break,
                // Leaving the menus for a script is a film run starting, and
                // a run starts from nothing: see `Progress::film_start`. A New
                // Game after a finished route must not inherit that route's
                // flags, and this is where the original drops them.
                Outcome::Play | Outcome::Finished | Outcome::SkipToChoice => {
                    next = wanted.clone();
                    if let Some(p) = progress.as_mut() {
                        p.film_start();
                    }
                }
                // A replay names its own script, which the DLL's table spells
                // as a path; `find_script` wants the trailing name.
                // A scene is a sequence, not one script. `FUN_1001f270` starts
                // it at step 0 and `FUN_1001f0d0` is asked for the next one
                // each time a script ends, so the whole list is carried and
                // walked here rather than the first of it being played alone.
                Outcome::Replay(run) => {
                    let Some(first) = run.script(0) else {
                        continue;
                    };
                    next = first.rsplit('/').next().unwrap_or(first).to_string();
                    log::info!(
                        "replaying {next}, {} script(s) in the scene, {}",
                        run.scripts.len(),
                        match &run.branch {
                            Some(table) => format!("branching over {} steps", table.len()),
                            None => "played in order".to_string(),
                        }
                    );
                    replaying = Some((run, 0));
                    // A replay is a film run like any other — it reaches
                    // playback through the same mode — so it starts from
                    // nothing too, and leaves nothing behind when it ends.
                    if let Some(p) = progress.as_mut() {
                        p.film_start();
                    }
                }
                // Loading from the title puts the player wherever the slot
                // says, so the position comes from the slot and not from a
                // fresh `searchRoot` on the name.
                // The route map's cells are inert from the title — the module
                // computes what can be picked only when a run is in play — so
                // this arrives from the bar's copy of the screen. It is handled
                // here too because the outcome is the menus', not playback's.
                Outcome::LoadStory(story) => {
                    if let Some(script) = enter_story(&mut player, progress.as_mut(), story) {
                        next = script;
                        chained = true;
                    }
                    continue;
                }
                // A slot that will not read starts nothing, and the menus
                // keep the screen.
                Outcome::LoadSlot { slot, recorded } => {
                    if let Some(script) = enter_slot(&mut player, progress.as_mut(), slot, recorded)
                    {
                        next = script;
                        chained = true;
                    }
                    continue;
                } // There is nothing to save from the title — the player has no
                  // position — so this only happens from playback, where the
                  // save is taken before the menus open.
            }
        }
        if !chained {
            if let Some(p) = progress.as_mut() {
                p.enter(&next);
            }
        }
        chained = false;
        let (name, path) = find_script(&vfs, &next, english)?;
        log::info!("playing {name} from {path}");
        let script = Script::parse(&name, &vfs.read_path(&path)?)?;
        log::info!(
            "{} events, length {} ({:.1}s)",
            script.events.len(),
            script.length,
            script.length.as_seconds()
        );
        // Still chasing a choice: a script that raises none is passed over
        // without being played, and the graph is asked for the one after it.
        // Bounded because the route graph is the player's own data and a cycle
        // in it must cost a log line, not the session.
        if chasing_choice {
            if script.skip_to < script.length {
                log::info!("the skip found a choice in {name}");
                chasing_choice = false;
            } else if passed_over >= MAX_SCRIPTS_PASSED_OVER {
                log::warn!(
                    "the skip passed over {passed_over} scripts without finding a choice; \
                     playing {name} rather than chasing further"
                );
                chasing_choice = false;
                passed_over = 0;
            } else if let Some(after) = progress.as_mut().and_then(Progress::advance) {
                passed_over += 1;
                log::info!("the skip passes over {name}, which raises no choice");
                next = after.rsplit('/').next().unwrap_or(&after).to_string();
                chained = true;
                continue;
            } else {
                // The route ended. There is no choice ahead, so this is where
                // the chase stops: play what the graph last named.
                log::info!("the skip reached the end of the route without a choice");
                chasing_choice = false;
                passed_over = 0;
            }
        }
        if !chasing_choice {
            passed_over = 0;
        }
        canvas
            .window_mut()
            .set_title(&format!("DaysEngine — {name}"))?;
        let outcome = run_script(
            &mut player,
            &mut canvas,
            &creator,
            &mut events,
            script,
            &start,
            progress.as_mut(),
        )?;
        canvas.window_mut().set_title("DaysEngine")?;
        // Playback has been left, whichever way. Host `+0x100` (`FUN_0042a500`,
        // the bar's leave button) and the end of a script both go through
        // `FUN_00424e20`, which pauses every stream the script owns, and
        // nothing on the way to the menus resumes them — `FUN_00424eb0`'s six
        // call sites are all paths back into playback. The playback object is
        // then handed to `setSystemInit` and torn down, so the pause is the end
        // of that sound: the title screen is silent but for its own `[TitleBGM]`
        // rather than carrying the scene's music and voices into it.
        player.mixer.stop_all();
        // A script that reached its end hands over to the branch graph, which
        // is what the executable's state 4 does. When the graph names nothing
        // — the route ended, or the script was not in it — the session goes
        // back to the title, as the game does. Quitting ends it either way.
        // The skip button chases a choice across scripts: `FUN_00425bf0`'s
        // case 7 loads the next script and, when that one's `[SkipFRAME]`
        // equals its `[Next]`, stays in case 7 and loads the one after it. So
        // the chase passes over whole scripts without playing them until it
        // reaches one that raises a choice, and plays that from its start —
        // `FUN_00431250(next, +0x53c + 1)`, the script's own beginning, not
        // the choice's frame.
        //
        // Case 7 is only ever reached from case 6, so this chaining is the
        // skip and nothing else: an ordinary end-of-script does not do it.
        // Cleared here and set only on a chain step below, so a chase that
        // runs out of route cannot leak into whatever is played next.
        chasing_choice = false;
        // A slot picked in the menus the control bar opened leaves playback,
        // and what it leaves to is the slot — not the title. `FUN_0041d7f0` is
        // the mode that plays a film: its case 4 asks host `+0x44`
        // (`FUN_00427ad0`) for the slot the load screen stored and returns its
        // own mode number when there is one, so the shell comes straight back
        // round to case 0, which reads that slot again and hands it to
        // `FUN_00427850` — the call that starts the film engine on it. Mode 2,
        // the title, is where the *absence* of a slot goes: case 4 returns it
        // only when `+0x44` is -1.
        //
        // A load replaces the position outright, so a replay scene that was
        // running ends here rather than carrying its list across.
        // A story point picked on the route map leaves playback exactly as a
        // slot does — `+0x48` then `+0x4c(8)` — so it comes back to the same
        // place and is answered the same way.
        if let Outcome::LoadStory(story) = outcome {
            replaying = None;
            if let Some(script) = enter_story(&mut player, progress.as_mut(), story) {
                next = script;
                chained = true;
                continue;
            }
        }
        if let Outcome::LoadSlot { slot, recorded } = outcome {
            replaying = None;
            if let Some(script) = enter_slot(&mut player, progress.as_mut(), slot, recorded) {
                next = script;
                chained = true;
                continue;
            }
            // The slot would not read. The script that was playing has already
            // been torn down, so there is nothing to go back to and this falls
            // through to where a finished session goes.
        }
        // The end of a script is where the original re-reads the two affection
        // counters and puts the gauge down again: `FUN_00424020` calls MenuBar
        // vtable `+0x38` — `FUN_10026050`, which sizes the pieces from the
        // counters and then clears the flag through host `+0x30`.
        if outcome == Outcome::Finished {
            if let Some(p) = progress.as_mut() {
                p.lower_gauge();
            }
        }
        if outcome == Outcome::Finished || outcome == Outcome::SkipToChoice {
            // A replay walks the scene's own list, by its branch table where it
            // has one and straight down the list where it does not —
            // `FUN_1001ee20`'s `default` arm. The column is the choice the
            // player last answered, which the original keeps for the life of
            // the film object and so outlives the script it was made in.
            if let Some((run, step)) = &mut replaying {
                if let Some(next_step) = run.next(*step, player.last_choice) {
                    *step = next_step;
                    let script = run.script(*step).unwrap_or_default();
                    next = script.rsplit('/').next().unwrap_or(script).to_string();
                    log::info!("replay step {step}: {next}");
                    chained = true;
                    continue;
                }
                // The list ran out, which is the chain ending. Back to the
                // menus rather than into the route graph: a replay is not a
                // position in the story.
                log::info!("the replay scene ended");
                replaying = None;
                continue;
            }
            if let Some(script) = progress.as_mut().and_then(Progress::advance) {
                next = script.rsplit('/').next().unwrap_or(&script).to_string();
                chained = true;
                // A replay never reaches here — it returned above — so this is
                // ordinary playback, where a skip keeps chasing.
                chasing_choice = outcome == Outcome::SkipToChoice;
                continue;
            }
        }
        // Anything else leaving playback ends the replay too.
        if outcome != Outcome::Finished && outcome != Outcome::SkipToChoice {
            replaying = None;
        }
        if outcome == Outcome::Quit || !menus {
            break;
        }
    }

    // Never leave a controller buzzing. The effect lapses on its own inside
    // [`RUMBLE_MS`], but an engine that closed cleanly should not need it to.
    player.pads.close();
    Ok(())
}

/// How the menus were entered, which is what decides where leaving one goes.
///
/// The original has two menu drivers and this picks between them. See
/// [`daysengine::ui::menu::Entry`] for the evidence; the short of it is that a
/// screen the control bar opened lives in the playback object's own menu layer
/// and returns to playback, while the title-rooted shell walks
/// `_getNextMode@8`'s graph and ends at the title.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MenuEntry {
    /// The menus own the screen, starting at the title.
    Title,
    /// One screen over live playback, as host `+0xf8(code)` opens it.
    OverPlayback(Mode, saveload::Kind),
}

/// How many scripts a skip will pass over before giving up and playing one.
///
/// The chase has a natural end — the route graph runs out — but the graph comes
/// out of the player's own `RouteProcSDHQ.dll`, and a cycle in it would
/// otherwise spin here forever. The longest retail route is 116 scripts, so
/// this is well clear of anything the real data asks for.
const MAX_SCRIPTS_PASSED_OVER: usize = 512;

/// Why a loop gave up control.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Outcome {
    /// Start playing the script.
    Play,
    /// The script reached its end, so the branch graph decides what follows.
    Finished,
    /// The skip button was pressed and this script had no choice ahead of the
    /// clock. Like [`Outcome::Finished`], except the chain keeps going — past
    /// whole scripts, without playing them — until it reaches one that raises
    /// a choice. `FUN_00425bf0`'s case 7 looping on itself; see
    /// [`daysengine::playback::stage::Stage::skip_target`].
    SkipToChoice,
    /// Play a replay scene: the scripts it runs through and how it walks them.
    Replay(replay::Run),
    /// Jump to a story point of the run, from the route map.
    ///
    /// The same leave the load screen's rows make, with a story number rather
    /// than a slot: `FUN_00423a70` sends a number of 100 or more to
    /// `FUN_00428400`, which restores the mark the run recorded for it.
    LoadStory(u32),
    /// Load a save slot and play what it names.
    ///
    /// `recorded` is a row of the replay screen's play-data list rather than
    /// the load screen: the same load, plus the slot's own answers followed.
    /// See [`Player::following_record`].
    LoadSlot { slot: u32, recorded: bool },
    /// Close the game.
    Quit,
}

/// The start script `STARTSCRIPT.INI` names, as a bare script name.
///
/// The file gives a path like `00/00-00-A00`; the route layer and our CLI both
/// use the trailing name.
fn start_script(start: &Ini) -> String {
    start
        .get("StartScript")
        .unwrap_or("00/00-00-A00")
        .rsplit('/')
        .next()
        .unwrap_or("00-00-A00")
        .to_string()
}

/// Puts the player where a slot says, and names the script to play from there.
///
/// Loading is one rule wherever it is asked for, because the engine only ever
/// does it in one place. Whichever screen picked the slot stores it on the
/// engine through host `+0x48` (`FUN_0042c020`, writing `engine + 0x2d0`) and
/// then leaves the menus; the mode that plays a film reads it back through
/// `+0x44` and hands it to `FUN_00427850`, and `FUN_00423a70` is what opens
/// the file — `[SaveFileName]` formatted with the slot number.
///
/// Which of two loads it runs is host `+0x98`, the flag the play-data list
/// raises with `+0x94(1)`: `FUN_0042b250` while it is clear, and
/// `FUN_00428ab0` — the same file, read so the slot's own answers are followed
/// — while it is set.
///
/// `None` means the slot would not read, and nothing was started.
fn enter_slot(
    player: &mut Player,
    progress: Option<&mut Progress>,
    slot: u32,
    recorded: bool,
) -> Option<String> {
    player.following_record = recorded;
    let game = player.game.clone();
    match progress.and_then(|p| p.load_from(&game, player.film, slot)) {
        Some(script) => {
            if recorded {
                log::info!("playing slot {slot} back by its own answers");
            }
            Some(script.rsplit('/').next().unwrap_or(&script).to_string())
        }
        None => {
            log::warn!("slot {slot} would not load");
            player.following_record = false;
            None
        }
    }
}

/// Puts the player at a story point the run has passed, as the route map does.
///
/// The jump is a load whose state comes from the run rather than from a file:
/// `FUN_00428400` restores the mark and opens its script. A story point this
/// run never passed has no mark, and the route map does not offer one — it
/// greys any cell the save's store has no flag for — so this is a refusal
/// that leaves the player where they were.
fn enter_story(player: &mut Player, progress: Option<&mut Progress>, story: u32) -> Option<String> {
    player.following_record = false;
    match progress.and_then(|p| p.from_story(story)) {
        Some(script) => Some(script.rsplit('/').next().unwrap_or(&script).to_string()),
        None => {
            log::warn!("this run never reached SP{story:03}");
            None
        }
    }
}

/// Loads the picture that goes behind the title.
///
/// This belongs to the engine, not to the menu module: `Title.png` is
/// transparent around the logo and the engine picks what goes under it from
/// save state — see [`daysengine::ui::ending`]. A card that will not load costs the
/// player the picture and nothing else, as any missing asset does.
fn load_title_backdrop(player: &Player, start: &Ini) -> Option<days_ui::Image> {
    let list = ending::load_list(player.vfs, start);
    let base = start.get("BaseFile").unwrap_or_default();
    let chosen = ending::title_backdrop(&list, &player.flags, base);
    log::info!("title backdrop {} ({:?})", chosen.path, chosen.reason);
    match ending::load_image(player.vfs, &chosen.path) {
        Ok(image) => Some(image),
        Err(err) => {
            log::warn!("loading title backdrop {}: {err}", chosen.path);
            None
        }
    }
}

/// Gathers everything the menus ask the host about.
///
/// A scene table that cannot be recovered is not fatal: the replay screen shows
/// an empty grid and says why, which is the same rule every other missing asset
/// follows.
fn build_session(player: &Player, start: &Ini, english: bool, run: Option<&Progress>) -> Session {
    let config = Config::load(&player.game);
    let scenes = match Scenes::recover(&player.dll) {
        Ok(scenes) => scenes,
        Err(err) => {
            log::warn!("no replay scene table: {err}");
            Scenes::from_scenes(Vec::new())
        }
    };
    Session {
        save: SaveState::from_flags(&player.flags, start),
        flags: player.flags.clone(),
        scenes,
        // The Def tab shows which value is in force and greys the other, so
        // this has to be the engine's real mode rather than a fixed answer.
        // The original asks the same two questions through host `+0xb8` and
        // `+0xbc`.
        display: player.display,
        // The SOMCON tab shows what the engine is really holding, the way
        // `FUN_100073a0` draws it from `_GetSomFlag@0` and `+0x324` rather
        // than from the settings file. `UseSOM` is the one half that is stored.
        som: Som {
            enabled: config.flag(Flag::UseSom),
            attached: player.pads.holding().is_some(),
            port: player.pads.holding().unwrap_or(0),
            testing: false,
        },
        english,
        config,
        text_input: player.film.get_bool("TextInput").unwrap_or(false),
        // The screen reports a slot as present when its file opens, and takes
        // the line it shows from the global store.
        slots: Slots::read(&player.game, player.film, &player.flags, english),
        // The route map asks this one whether the run has passed a story
        // point. From the title there is no run and the screen does not ask.
        run: run.map(|p| p.store().clone()),
    }
}

/// Pushes the settings' volumes into the mixer.
///
/// The original hands a decibel figure per channel to DirectSound; this engine
/// has one master gain, so until the mixer grows per-channel gain the quietest
/// of the three is what it can honestly apply. Muting is exact either way.
fn apply_settings(session: &Session, mixer: &Mixer) {
    apply_volumes(&session.config, mixer);
}

/// Hands the mixer the gain each group of sounds plays at.
///
/// Every sound in the original carries the level of the category it asks
/// `_GetMasterVolume@4` for, and `Mute` swaps a fixed level 2 in for the
/// script's three — but not for the menus', which keep `SeVolume`. See
/// [`daysengine::install::config::Config::centibels`].
fn apply_volumes(config: &Config, mixer: &Mixer) {
    mixer.set_gains(daysengine::playback::mixer::Gains {
        bgm: config.gain(Channel::Bgm),
        se: config.gain(Channel::Se),
        voice: config.gain(Channel::Voice),
        system: config.system_se_gain(),
    });
}

/// Runs the menus until they start a script or the game is closed.
///
/// The menu is a still image that only changes when the selection does, so this
/// recomposites on demand rather than per frame: a frame is a 800x450 software
/// composite and there is nothing animating between clicks.
fn run_menu(
    player: &mut Player,
    canvas: &mut Canvas<Window>,
    creator: &TextureCreator<WindowContext>,
    events: &mut EventPump,
    start: &Ini,
    entry: MenuEntry,
    mut progress: Option<&mut Progress>,
) -> Result<Outcome> {
    // `[UseEnglish]` decides which way round the save line's date reads, and
    // how many characters of it are the chapter.
    let english = player.film.get_bool("UseEnglish").unwrap_or(false);
    let session = build_session(player, start, english, progress.as_deref());
    apply_settings(&session, player.mixer);
    // A bar-opened screen is not a title menu that then navigates: host
    // `+0xf8` puts the playback object straight into its menu layer with that
    // one module's code, so the module is the first screen there is.
    let mut menu = match entry {
        MenuEntry::Title => Menu::open(
            player.vfs,
            &player.dll,
            Mode::TITLE,
            session,
            player.resolution(),
        )
        .context("opening the title screen")?,
        MenuEntry::OverPlayback(mode, kind) => Menu::open_over_playback(
            player.vfs,
            &player.dll,
            mode,
            kind,
            session,
            player.resolution(),
        )
        .with_context(|| format!("opening menu mode {} over playback", mode.0))?,
    };

    let backdrop = load_title_backdrop(player, start);
    // Resampled into the screen's space once rather than on every composite:
    // it covers the whole frame, and a hover that relights one label would
    // otherwise pay for scaling all of it again. Rebuilt when the display mode
    // changes, which is the only thing that changes the size it goes into.
    let mut under = backdrop.as_ref().map(|b| menu.screen().to_display(b));

    // Only the title shell has music of its own. `[TitleBGM]` is read in one
    // place in the executable — `FUN_0041f600`, the same function that reads
    // `[StartScript]`, with one caller — and a screen the control bar opens is
    // the other driver entirely: host `+0xf8` (`FUN_0042a430`) puts the
    // playback object into its menu layer, over a script whose sound it has
    // paused, and plays nothing of its own. So that screen is quiet but for its
    // own clicks, and the scene starts again where it stopped on the way out.
    if entry == MenuEntry::Title {
        play_menu_bgm(player, start.get("TitleBGM"));
    }

    let mut texture: Option<Texture> = None;
    // The slot the player picked on the save screen, waiting for the
    // comment dialog to confirm or abandon it.
    let mut naming: Option<u32> = None;
    // Whole-number scaling, which decides both where the screen lands and how
    // it gets there.
    let whole = player.whole_pixels();
    // Where the stick-driven pointer is, in window pixels. Only the stick
    // moves it; the mouse is read from its own events, as it always was.
    let mut cursor = (0.0f32, 0.0f32);
    // A direction held across the way in, so it does not repeat into a screen
    // the player has only just opened.
    player.controls.clear();
    // Paced to the display, and deciding on its grid rather than on whatever
    // moment the last pass ended.
    let mut cadence = Cadence::new(canvas);
    loop {
        let now = cadence.tick();
        // Everything the player asked for this pass: the events SDL had, and
        // the repeats of whatever is still held down.
        let mut asked: Vec<Control> = Vec::new();
        let mut pointed: Option<(i32, i32)> = None;
        let mut clicked = false;
        for event in events.poll_iter() {
            match &event {
                Event::Quit { .. } => return Ok(Outcome::Quit),
                Event::GamepadAdded { which, .. } => player.pads.added(*which),
                Event::GamepadRemoved { which, .. } => player.pads.removed(*which),
                Event::MouseMotion { x, y, .. } => {
                    cursor = (*x, *y);
                    pointed = Some((*x as i32, *y as i32));
                }
                Event::MouseButtonDown {
                    mouse_btn: MouseButton::Left,
                    x,
                    y,
                    ..
                } => {
                    cursor = (*x, *y);
                    pointed = Some((*x as i32, *y as i32));
                    clicked = true;
                }
                Event::MouseButtonDown {
                    mouse_btn: MouseButton::Right,
                    ..
                } => asked.push(Control::Cancel),
                _ => {}
            }
            asked.extend(player.controls.take(&event, now));
        }
        asked.extend(player.controls.due(now));
        // The right stick moves the pointer, for the screens a selection
        // cannot reach every part of.
        if let Some((dx, dy)) = player.controls.cursor(&player.pads, cadence.interval()) {
            let (w, h) = canvas.window().size();
            cursor.0 = (cursor.0 + dx).clamp(0.0, w.saturating_sub(1) as f32);
            cursor.1 = (cursor.1 + dy).clamp(0.0, h.saturating_sub(1) as f32);
            pointed = Some((cursor.0 as i32, cursor.1 as i32));
        }

        // The pointer first, so a confirm in the same pass acts on what it is
        // over — which is what a click is: the original acts on whatever the
        // cursor is on, not on whatever the selection last was.
        let mut actions: Vec<Action> = Vec::new();
        if let Some((x, y)) = pointed {
            actions.push(match to_screen(canvas, &menu, whole, x as f32, y as f32) {
                Some((sx, sy)) => menu.point_at(sx, sy),
                None => menu.point_away(),
            });
        }
        if clicked {
            actions.push(confirm(&mut menu, player)?);
        }
        for control in asked {
            actions.push(match control {
                Control::Up => menu.navigate(Dir::Up),
                Control::Down => menu.navigate(Dir::Down),
                Control::Left => menu.navigate(Dir::Left),
                Control::Right => menu.navigate(Dir::Right),
                Control::Confirm => confirm(&mut menu, player)?,
                Control::Cancel => menu.cancel(player.vfs, &player.dll)?,
                _ => Action::Stay,
            });
        }

        for action in actions {
            match action {
                Action::Play => return Ok(Outcome::Play),
                Action::PlayReplay(script) => return Ok(Outcome::Replay(script)),
                Action::PlayRecorded(slot) => {
                    return Ok(Outcome::LoadSlot {
                        slot,
                        recorded: true,
                    })
                }
                Action::Load(slot) => {
                    return Ok(Outcome::LoadSlot {
                        slot,
                        recorded: false,
                    })
                }
                Action::LoadStory(story) => return Ok(Outcome::LoadStory(story)),
                // Naming the save is what confirms it. The original hands this
                // to Windows; `run_comment` draws the same dialog out of the
                // executable's own template. Cancelling calls nothing, so no
                // save happens — which is what `FUN_0042e4a0` does by simply
                // not reaching `_CommentSet@4`.
                Action::Save(slot) => naming = Some(slot),
                Action::Quit => return Ok(Outcome::Quit),
                // Each change is already in the settings; this is where the
                // engine picks the new volumes up.
                Action::SettingsChanged => {
                    apply_settings(menu.session(), player.mixer);
                    texture = None;
                }
                // The Option screen's close button. `FUN_10007ef0` widget 3
                // flushes the config object and then leaves the menus with
                // `+0x4c(0)` — the same code the save/load screen's Close
                // uses — so the flush happens here and where it lands is the
                // entry's question, exactly as for every other screen.
                Action::SettingsSaved => {
                    apply_settings(menu.session(), player.mixer);
                    if menu.session().config.dirty() {
                        let mut config = menu.session().config.clone();
                        if let Err(err) = config.save(&player.game) {
                            log::warn!("could not write the settings: {err}");
                        }
                        menu.session_mut().config = config;
                    }
                    match menu.leave(player.vfs, &player.dll)? {
                        Action::Play => return Ok(Outcome::Play),
                        _ => texture = None,
                    }
                }
                // The Def tab does not change the display itself: it raises a
                // flag and the executable acts on it. `FUN_004279e0` polls
                // `GetFullFlag`, flips full screen to the opposite of what
                // `FUN_0040e830` reports and clears the flag with
                // `SetFullFlag(0)`; `FUN_00427a90` does the same pair for
                // `GetWideFlag`. Acting on arrival instead of polling reaches
                // the same place, because the flag is raised and cleared
                // without anything else getting a look in between.
                //
                // See `daysengine::ui::options::DisplayRequest`.
                Action::Display(request) => {
                    apply_display(player, canvas, request)?;
                    // Into the settings, where the Option screen's close button
                    // will flush them — the same two steps the original takes,
                    // `FUN_0040db00` and `FUN_0040c700` storing the keys as the
                    // mode is applied and the close flushing the file.
                    remember_display(&mut menu.session_mut().config, player.display);
                    // Every screen's art is chosen by the mode, so whatever is
                    // showing has to be reloaded at the new size.
                    menu.set_resolution(player.vfs, &player.dll, player.resolution())?;
                    menu.session_mut().display = player.display;
                    // Full screen can land the window on a panel that refreshes
                    // at another rate.
                    cadence = Cadence::new(canvas);
                    texture = None;
                }
                // The SOMCON tab asked something of the device, and the
                // engine is the half that knows what is really there.
                // `FUN_10007850` walks the ports itself and `_GetSomFlag@0`
                // reports what it found, so the answer goes straight back into
                // the screen and the art follows it.
                Action::Som(request) => {
                    let mut som = menu.session().som;
                    som.enabled = menu.session().config.flag(Flag::UseSom);
                    match request {
                        options::SomRequest::Detect => match player.pads.detect() {
                            Some(port) => {
                                log::info!("rumble: port {} took the level", port + 1);
                                som.attached = true;
                                som.port = port;
                            }
                            None => {
                                log::info!("rumble: no controller would take a level");
                                som.attached = false;
                            }
                        },
                        options::SomRequest::Release => {
                            player.pads.close();
                            som.attached = false;
                            som.testing = false;
                        }
                        options::SomRequest::Port(port) => {
                            som.attached = player.pads.open(port);
                            som.port = port;
                            som.testing = false;
                        }
                        // The test is the DLL's own `s96`, held until the
                        // player stops it.
                        options::SomRequest::Test(on) => {
                            som.testing = on && som.attached;
                            player
                                .pads
                                .set_level(if som.testing { som::TEST_LEVEL } else { 0 });
                        }
                    }
                    // `UseSOM` off is no device at all, whatever was held.
                    if !som.enabled {
                        player.pads.close();
                        som.attached = false;
                        som.testing = false;
                    }
                    menu.set_som(player.vfs, &player.dll, som)?;
                    apply_settings(menu.session(), player.mixer);
                    texture = None;
                }
                Action::Sound(se) => {
                    player
                        .system_se
                        .play(se, player.vfs, &mut player.sounds, player.mixer)
                }
                // A screen this engine cannot draw yet leaves the player
                // where they were; saying so beats a silent dead key.
                Action::Unavailable(mode) => {
                    log::warn!("menu mode {} is not built yet", mode.0);
                    player.system_se.play(
                        SystemSe::Cancel,
                        player.vfs,
                        &mut player.sounds,
                        player.mixer,
                    );
                }
                Action::Opened(mode) => {
                    log::info!("menu mode {}", mode.0);
                    player.system_se.play(
                        SystemSe::Open,
                        player.vfs,
                        &mut player.sounds,
                        player.mixer,
                    );
                    texture = None;
                }
                Action::Stay => {}
            }
        }

        // Outside the event loop: the dialog runs its own, and the borrow of
        // the pump has to have ended first.
        //
        // `FILMENGINE.INI [TextInput]` is what decides whether there is a
        // dialog at all. `FUN_10014990` asks the host through `+0xd8`, and with
        // the key clear it raises `+0x98` on the spot — no dialog, and
        // `FUN_10011c30` writes `L""` as the comment. Only with the key set
        // does it hand `+0xdc` a default and wait for `_CommentSet@4`.
        if let Some(slot) = naming.take() {
            let taken = if menu.session().text_input {
                let existing = menu
                    .session()
                    .slots
                    .get(slot)
                    .map(|line| line.comment.clone())
                    .unwrap_or_default();
                // Cancelling calls nothing, so nothing is taken and no save
                // happens — `FUN_0042e4a0` simply never reaches `_CommentSet@4`.
                run_comment(player, canvas, creator, events, &mut menu, &existing)?
            } else {
                Some(String::new())
            };
            texture = None;
            if let Some(comment) = taken {
                menu.begin_save(slot, comment);
            }
        }

        // The tick after the save was taken writes it, refreshes the page and
        // puts `+0x98` down, leaving the player on the save screen.
        if let Some((slot, comment)) = menu
            .pending_save()
            .map(|(slot, comment)| (slot, comment.to_owned()))
        {
            let line = match progress.as_deref_mut() {
                Some(p) => {
                    let game = player.game.clone();
                    match p.save_to(&game, player.film, slot, english, Some(&comment)) {
                        Ok(()) => {
                            // The global store now holds the slot's display
                            // line, so the menus have to see it.
                            player.flags = p.global().clone();
                            menu.session_mut().flags = p.global().clone();
                            Some(saveload::Line::read(player.film, p.global(), slot, english))
                        }
                        Err(err) => {
                            log::warn!("could not write slot {slot}: {err}");
                            None
                        }
                    }
                }
                // Reached from the title, where the menus open the load screen
                // and there is no position to write. Nothing is saved, and the
                // screen comes back rather than sticking on the notice.
                None => {
                    log::info!("slot {slot} cannot be written without a position");
                    None
                }
            };
            menu.finish_save(slot, line.unwrap_or_default());
            texture = None;
        }

        // Where the screen lands, and then the screen composited at exactly
        // that size — one pass from the 800x450 art to the pixels the player
        // sees, rather than one into the hit map's size and another onto the
        // window. See `Screen::fit_to`.
        //
        // Whole-number scaling is the exception: there the composite stays at
        // its own size and the blit below multiplies each of its pixels into
        // the same square block, which is a scaling no filter is involved in.
        let (map_w, map_h) = menu.screen().map_size();
        let dst = letterbox(canvas, map_w, map_h, whole);
        let at = if whole {
            (map_w, map_h)
        } else {
            (dst.w.round().max(1.0) as u32, dst.h.round().max(1.0) as u32)
        };
        // Asked of the screen itself rather than remembered, because the size
        // moves when the screen does: `Menu::enter` loads a brand-new `Screen`
        // on every navigation and a new one composites at its hit map's size
        // until it is told otherwise. A remembered width does not change when
        // the screen swaps, so coming back to the title from the Option screen
        // left the title compositing at 1280x720 while the backdrop under it
        // was still sized for the window.
        if at.0 != menu.screen().size().0 {
            menu.set_output_size(at.0, at.1);
            // The backdrop is scaled into the screen's space, so it follows.
            under = backdrop.as_ref().map(|b| menu.screen().to_display(b));
            texture = None;
        }
        if menu.dirty() || texture.is_none() {
            // The backdrop is only the title's; every other screen draws its own
            // background or sits over black.
            let image = menu.compose(
                (menu.mode() == Mode::TITLE)
                    .then_some(under.as_ref())
                    .flatten(),
            );
            let mut new = new_texture(creator, image.width, image.height, art_sampling(whole))?;
            new.update(None, &image.rgba, image.width as usize * 4)?;
            texture = Some(new);
        }

        canvas.set_draw_color(Color::BLACK);
        canvas.clear();
        if let Some(texture) = &texture {
            canvas
                .copy(texture, None, dst)
                .map_err(|e| anyhow::anyhow!("drawing the menu: {e}"))?;
        }
        canvas.present();
        cadence.wait(now);
    }
}

/// Runs the save-comment dialog over the menu.
///
/// Returns the comment when the player accepts and `None` when they cancel —
/// and cancelling means no save at all, because in the original the dialog's OK
/// is what sets the member the save screen's next tick acts on.
///
/// Text arrives through SDL's text input rather than through key codes, which
/// is what carries an IME: the original is a Windows edit control and the
/// player is expected to type Japanese into it.
fn run_comment(
    player: &mut Player,
    canvas: &mut Canvas<Window>,
    creator: &TextureCreator<WindowContext>,
    events: &mut EventPump,
    menu: &mut Menu,
    existing: &str,
) -> Result<Option<String>> {
    let english = player.film.get_bool("UseEnglish").unwrap_or(false);
    let exe = match std::fs::read(player.game.join("SCHOOLDAYS HQ.exe")) {
        Ok(bytes) => bytes,
        Err(err) => {
            log::warn!("cannot read the executable for the comment dialog: {err}");
            return Ok(Some(existing.to_owned()));
        }
    };
    let mut dialog = match comment::Comment::open(&exe, english, existing) {
        Ok(dialog) => dialog,
        Err(err) => {
            log::warn!("the comment dialog is unavailable: {err}");
            return Ok(Some(existing.to_owned()));
        }
    };
    let base = comment::base_units(english);

    player.text_input.start(canvas.window());
    let result = comment_loop(player, canvas, creator, events, menu, &mut dialog, base);
    player.text_input.stop(canvas.window());
    result
}

fn comment_loop(
    player: &mut Player,
    canvas: &mut Canvas<Window>,
    creator: &TextureCreator<WindowContext>,
    events: &mut EventPump,
    menu: &mut Menu,
    dialog: &mut comment::Comment,
    base: (i32, i32),
) -> Result<Option<String>> {
    let mut texture: Option<Texture> = None;
    let mut size = (0u32, 0u32);
    let mut placed: Option<Placed> = None;
    let mut dirty = true;
    let whole = player.whole_pixels();

    // Paced to the display, and deciding on its grid rather than on whatever
    // moment the last pass ended.
    let cadence = Cadence::new(canvas);
    loop {
        let now = cadence.tick();
        // Collected first: the handlers below need the pump again for the
        // modifier state, and cannot hold its iterator while they do.
        let pending: Vec<Event> = events.poll_iter().collect();
        for event in pending {
            let act = match event {
                Event::Quit { .. } => return Ok(None),
                Event::TextInput { text, .. } => {
                    dialog.insert(&text);
                    dirty = true;
                    comment::Act::None
                }
                Event::TextEditing { text, .. } => {
                    dialog.compose(&text);
                    dirty = true;
                    comment::Act::None
                }
                Event::KeyDown {
                    keycode: Some(key),
                    keymod,
                    ..
                } => {
                    dirty = true;
                    match key {
                        Keycode::Return | Keycode::KpEnter => dialog.enter(),
                        Keycode::Escape => dialog.escape(),
                        Keycode::Backspace => {
                            dialog.backspace();
                            comment::Act::None
                        }
                        Keycode::Delete => {
                            dialog.delete();
                            comment::Act::None
                        }
                        Keycode::Left => {
                            dialog.left();
                            comment::Act::None
                        }
                        Keycode::Right => {
                            dialog.right();
                            comment::Act::None
                        }
                        Keycode::Home => {
                            dialog.home();
                            comment::Act::None
                        }
                        Keycode::End => {
                            dialog.end();
                            comment::Act::None
                        }
                        Keycode::Tab => {
                            dialog.tab(keymod.intersects(
                                sdl3::keyboard::Mod::LSHIFTMOD | sdl3::keyboard::Mod::RSHIFTMOD,
                            ));
                            comment::Act::None
                        }
                        _ => comment::Act::None,
                    }
                }
                Event::MouseButtonDown {
                    mouse_btn: MouseButton::Left,
                    x,
                    y,
                    ..
                } => {
                    dirty = true;
                    match to_dialog(canvas, size, placed, whole, x, y) {
                        Some(at) => dialog.click(at, base),
                        None => comment::Act::None,
                    }
                }
                _ => comment::Act::None,
            };
            match act {
                comment::Act::Accept(text) => return Ok(Some(text)),
                comment::Act::Cancel => return Ok(None),
                comment::Act::None => {}
            }
        }

        if dirty || texture.is_none() {
            let mut image = menu.compose(None);
            // The menu under it is composited straight at the size the window
            // will show — see `Screen::fit_to` — so the dialog is magnified by
            // the same factor. Drawn at its own pixel size it would keep
            // shrinking as the window grew, which is not what the screen it
            // sits on does.
            let scale = menu.screen().output_scale();
            let over = menu
                .screen()
                .to_output(&dialog.compose_image(player.font, base));
            // `WM_INITDIALOG` centres the dialog on the game window, so this
            // does too.
            let (w, h) = (over.width, over.height);
            let at = (
                (image.width as i64 - w as i64) / 2,
                (image.height as i64 - h as i64) / 2,
            );
            image.blit_scaled(&over, (0, 0, w, h), (at.0, at.1, w, h));
            // Written where it was drawn, so `to_dialog` cannot disagree with
            // the blit above about where the dialog is.
            placed = Some(Placed {
                at,
                size: (w, h),
                scale,
            });
            size = (image.width, image.height);
            let mut new = new_texture(creator, image.width, image.height, art_sampling(whole))?;
            new.update(None, &image.rgba, image.width as usize * 4)?;
            texture = Some(new);
            dirty = false;
        }

        canvas.set_draw_color(Color::BLACK);
        canvas.clear();
        if let Some(texture) = &texture {
            canvas
                .copy(texture, None, letterbox(canvas, size.0, size.1, whole))
                .map_err(|e| anyhow::anyhow!("drawing the comment dialog: {e}"))?;
        }
        canvas.present();
        cadence.wait(now);
    }
}

/// Where the save-comment dialog was last drawn in the composite, and by how
/// much it was magnified to get there.
///
/// Recorded by the pass that draws it and read by [`to_dialog`], because a hit
/// test that recomputes the placement is a hit test that can disagree with the
/// blit.
#[derive(Clone, Copy)]
struct Placed {
    /// Top-left in composite pixels.
    at: (i64, i64),
    /// Size in composite pixels.
    size: (u32, u32),
    /// Composite pixels per dialog pixel.
    scale: f64,
}

/// Maps a window pixel to the dialog's own space, or `None` outside it.
fn to_dialog(
    canvas: &Canvas<Window>,
    size: (u32, u32),
    placed: Option<Placed>,
    whole: bool,
    x: f32,
    y: f32,
) -> Option<(i32, i32)> {
    in_dialog(
        letterbox(canvas, size.0, size.1, whole),
        size,
        placed?,
        x,
        y,
    )
}

/// [`to_dialog`] once the window rectangle is known, which is the part a test
/// can reach without an SDL window.
fn in_dialog(dst: FRect, size: (u32, u32), placed: Placed, x: f32, y: f32) -> Option<(i32, i32)> {
    if size.0 == 0 || size.1 == 0 || dst.w <= 0.0 || dst.h <= 0.0 || placed.scale <= 0.0 {
        return None;
    }
    let sx = f64::from((x - dst.x) / dst.w) * f64::from(size.0);
    let sy = f64::from((y - dst.y) / dst.h) * f64::from(size.1);
    let (dx, dy) = (sx - placed.at.0 as f64, sy - placed.at.1 as f64);
    (dx >= 0.0 && dy >= 0.0 && dx < f64::from(placed.size.0) && dy < f64::from(placed.size.1))
        .then(|| ((dx / placed.scale) as i32, (dy / placed.scale) as i32))
}

/// Activates the menu's selection, playing the confirm sound when it takes.
fn confirm(menu: &mut Menu, player: &mut Player) -> Result<Action> {
    let action = menu.confirm(player.vfs, &player.dll)?;
    if action != Action::Stay {
        player.system_se.play(
            SystemSe::Click,
            player.vfs,
            &mut player.sounds,
            player.mixer,
        );
    }
    Ok(action)
}

/// Starts the menu music named by a `STARTSCRIPT.INI` key.
fn play_menu_bgm(player: &mut Player, path: Option<&str>) {
    let Some(path) = path else {
        return;
    };
    let Some(bgm) = player.vfs.resolve_bgm(path) else {
        log::warn!("menu BGM {path} is missing");
        return;
    };
    let load = |handle| {
        player
            .vfs
            .read(handle)
            .ok()
            .and_then(|bytes| daysengine::media::decode_audio(bytes).ok())
            .map(Arc::new)
    };
    let Some(looped) = load(bgm.looped) else {
        log::warn!("decoding menu BGM {path} failed");
        return;
    };
    let intro = bgm.intro.filter(|h| *h != bgm.looped).and_then(load);
    player.mixer.play_bgm(intro, looped);
}

/// Applies a display change the Option screen asked for.
///
/// Full screen goes to the **desktop's own resolution** rather than to one of
/// the two sizes `DX9GRAPHIC.INI` names. The original had to pick a mode the
/// adapter could set in 2005; there is no such constraint here, and the frame
/// is resampled to whatever it lands on by [`daysengine::playback::scale`], so
/// borrowing the desktop mode gives a sharper picture than stretching 1280x720
/// across a panel that is not 1280x720. Which art set is loaded still follows
/// the recovered rule in [`Resolution::for_display`] — that is about layout,
/// not about the size of the window it ends up in.
fn apply_display(
    player: &mut Player,
    canvas: &mut Canvas<Window>,
    request: options::DisplayRequest,
) -> Result<()> {
    match request {
        options::DisplayRequest::Wide => player.display.wide = true,
        options::DisplayRequest::Normal => player.display.wide = false,
        options::DisplayRequest::FullScreen | options::DisplayRequest::Windowed => {
            let want = request == options::DisplayRequest::FullScreen;
            canvas
                .window_mut()
                .set_fullscreen(want)
                .map_err(|e| anyhow::anyhow!("setting full screen: {e}"))?;
            player.display.full_screen = want;
        }
    }
    log::info!(
        "display is now {} and {}, art {}",
        if player.display.wide { "wide" } else { "4:3" },
        if player.display.full_screen {
            "full screen at the desktop's resolution"
        } else {
            "windowed"
        },
        player.resolution().name()
    );
    Ok(())
}

/// Writes the display mode into the settings, so the game reopens in it.
///
/// The original stores the same two keys as it applies a mode: `FUN_0040db00`
/// sets `WindowMode` to the argument it is switching to, and `FUN_0040c700`
/// writes `Format`, `WindowWidth`, `WindowHeight`, `DisplayType` and
/// `TypeMiniNote` back once the device has taken the mode. Only the two the
/// Option screen can ask for are written here: this engine picks its own window
/// size and surface format, and `TypeMiniNote` has no widget behind it.
///
/// `DisplayType` is 1 for wide and 0 for 4:3, from the back-buffer sizes
/// `FUN_0040cbb0` chooses between; `WindowMode` is 1 for full screen, from the
/// window style `FUN_0040db00` sets for each.
fn remember_display(config: &mut Config, display: Display) {
    config.set("DisplayType", if display.wide { "1" } else { "0" });
    config.set("WindowMode", if display.full_screen { "1" } else { "0" });
}

/// The display's presentation grid: when each refresh is, and which one a pass
/// is drawing for.
///
/// # Why a pass does not simply read the clock
///
/// A 24 fps clip on a 60 Hz panel shows one source frame for three refreshes
/// and the next for two, forever. That ratio is 2.5 and there is no way to
/// spend it evenly; every film on every 60 Hz screen does the same thing. What
/// turns that beat into *judder* is the pattern going irregular, and it goes
/// irregular when the frame to draw is chosen from a wall-clock reading taken
/// at whatever moment the last pass happened to finish. A frame boundary that
/// falls within a millisecond of a refresh then flips between one side of it
/// and the other as the work in a pass varies — and at 24 against 60, a
/// boundary lands on a refresh every other frame, so it flips constantly.
///
/// So the instant a pass decides with is the wall clock **snapped to the
/// nearest refresh**. Which frame is drawn becomes a function of the refresh
/// number and nothing else, the repeat pattern is the same every time round,
/// and the beat is as even as the ratio allows. The clock is still the wall
/// clock: the snap moves it by at most half a refresh, against a frame that
/// lasts 41ms.
///
/// # What it does not fix
///
/// The grid is predicted from the refresh rate the display *reports*. A panel
/// that says 60 and runs at 59.94 slides against it by one refresh every
/// seventeen seconds or so, and the snap corrects that in one step — a single
/// frame a refresh short, rather than a drift in playback. Correcting the
/// picture towards the wall clock is the right way round: the audio device is
/// not going to wait.
struct Cadence {
    /// Refresh zero. Every grid point is a whole number of intervals from here.
    anchor: Instant,
    /// One refresh, from the display or [`Cadence::FALLBACK_HZ`].
    interval: Duration,
}

impl Cadence {
    /// What a display that will not say what it does, or claims something
    /// absurd, is assumed to be doing. Wrong and smooth beats wrong and
    /// spinning.
    const FALLBACK_HZ: f32 = 60.0;

    /// Reads the rate of the display the window is on.
    fn new(canvas: &Canvas<Window>) -> Cadence {
        let hz = canvas
            .window()
            .get_display()
            .and_then(|display| display.get_mode())
            .map(|mode| mode.refresh_rate)
            .unwrap_or(0.0);
        let hz = if hz.is_finite() && (20.0..=1000.0).contains(&hz) {
            hz
        } else {
            Cadence::FALLBACK_HZ
        };
        log::info!("presenting at {hz:.2} Hz");
        Cadence {
            anchor: Instant::now(),
            interval: Duration::from_secs_f32(1.0 / hz),
        }
    }

    /// One refresh, which is how long a pass round the loop stands for.
    ///
    /// What the stick-driven pointer moves by: a speed in pixels a second is
    /// only a speed if it is multiplied by the time a pass covers.
    fn interval(&self) -> Duration {
        self.interval
    }

    /// The refresh this pass is drawing for, which is what it decides with.
    fn tick(&self) -> Instant {
        self.anchor + snap(self.anchor.elapsed(), self.interval)
    }

    /// Sleeps until the refresh after the one `tick` named.
    ///
    /// Vsync should already have blocked in `present` by the time this is
    /// reached, leaving nothing to wait for; this is what paces the loop when
    /// the driver refused it. Either way the deadline is a point on the grid
    /// and not an interval added to the end of the pass, so a pass that ran
    /// long does not push every pass after it.
    fn wait(&self, tick: Instant) {
        if let Some(left) = (tick + self.interval).checked_duration_since(Instant::now()) {
            std::thread::sleep(left);
        }
    }
}

/// The first control bar widget a selection can land on.
///
/// The bar's own order, so this is widget 0 — auto-advance — unless something
/// has turned it off, which nothing does.
fn first_bar_widget(state: bar::State) -> Option<usize> {
    (0..bar::WIDGETS).find(|widget| bar::enabled(*widget, state))
}

/// The next live control bar widget along, wrapping.
///
/// Dead widgets are stepped over rather than landed on: `FUN_10023fb0` answers
/// for each of them and a press on one is swallowed *and silent*, so a
/// selection that could rest on one would look like a control that had stopped
/// working. The bar is one strip, so this is the whole of its navigation.
fn step_bar(from: usize, forward: bool, state: bar::State) -> Option<usize> {
    (1..=bar::WIDGETS)
        .map(|step| {
            let step = if forward { step } else { bar::WIDGETS - step };
            (from + step) % bar::WIDGETS
        })
        .find(|widget| bar::enabled(*widget, state))
}

/// `elapsed` rounded to the nearest whole refresh.
///
/// Nearest rather than last: a pass wakes either side of the refresh it is
/// drawing for by a scheduler's worth of noise, and rounding is what makes
/// both answers the same one. See [`Cadence`].
fn snap(elapsed: Duration, interval: Duration) -> Duration {
    let ticks = (elapsed.as_secs_f64() / interval.as_secs_f64()).round();
    interval.mul_f64(ticks.max(0.0))
}

/// What the cached control-bar layer was composited from.
///
/// The record list is most of it, but not all: the gauge's three pieces are
/// sized from the two affection counters rather than from any record, so those
/// belong in the key too, and so do the two alphas for the one case where the
/// fade has to be composited in rather than modulated.
type BarLayer = (Vec<usize>, (f32, f32), Option<(u8, u8)>);

/// Where the control bar's strip lands in the window.
///
/// `strip` is the strip's size in the art set the display mode chose — 800x75
/// windowed, 1280x120 full screen — and it is the full width of the picture in
/// every one of them. So it scales by its own width. Scaling it by the stage's
/// instead, as though the two shared a ladder, drew the bar 1.6x oversized off
/// the right of a full-screen window and put every widget's hit box there too.
fn bar_strip(dst: FRect, strip: (u32, u32)) -> FRect {
    let scale = dst.w / strip.0.max(1) as f32;
    FRect::new(dst.x, dst.y, dst.w, strip.1 as f32 * scale)
}

/// Creates a streaming RGBA texture, filtered the way the original filters.
///
/// `FUN_0044a3d0` sets `D3DSAMP_MAGFILTER` and `D3DSAMP_MINFILTER` to
/// `D3DTEXF_LINEAR` on all eight sampler stages, so everything the engine draws
/// is filtered on its way onto the screen. Saying so at every call rather than
/// leaving it to SDL's default is the difference between a recovered choice and
/// an inherited one — and it is what lets whole-number scaling say otherwise;
/// see [`art_sampling`].
fn new_texture<'a>(
    creator: &'a TextureCreator<WindowContext>,
    width: u32,
    height: u32,
    mode: ScaleMode,
) -> Result<Texture<'a>> {
    let mut texture = creator.create_texture_streaming(
        PixelFormat::try_from(sdl3::sys::pixels::SDL_PIXELFORMAT_RGBA32)?,
        width,
        height,
    )?;
    texture.set_scale_mode(mode);
    Ok(texture)
}

/// How the game's own art is sampled on its way to the window.
///
/// Whole-number scaling means the destination is an exact multiple of the
/// source, and at an exact multiple point sampling *is* the right answer: every
/// source pixel becomes the same square block of destination pixels and no
/// value is invented. Asking for a filter there would only blur the edges of
/// blocks that are already where they belong.
fn art_sampling(whole: bool) -> ScaleMode {
    if whole {
        ScaleMode::Nearest
    } else {
        ScaleMode::Linear
    }
}

/// The rectangle a `width` x `height` image is drawn into, centred and scaled
/// to fit the window without distorting it.
fn letterbox(canvas: &Canvas<Window>, width: u32, height: u32, whole: bool) -> FRect {
    let window = canvas.output_size().unwrap_or((width, height));
    fit(window, (width, height), whole)
}

/// The rectangle `content` is drawn into inside `window`.
///
/// `whole` asks for a whole-number multiple: the largest that still fits,
/// centred, with a border around the rest. Every pixel of the content then
/// becomes the same square block of the window's, which is what makes point
/// sampling exact — see [`art_sampling`]. Below 1:1 there is no multiple to
/// take, so a window smaller than the game fits it the ordinary way rather than
/// putting a border around a picture that is already too small.
fn fit(window: (u32, u32), content: (u32, u32), whole: bool) -> FRect {
    let (win_w, win_h) = (window.0 as f32, window.1 as f32);
    let (w, h) = (content.0.max(1) as f32, content.1.max(1) as f32);
    let fitted = (win_w / w).min(win_h / h);
    let scale = if whole && fitted >= 1.0 {
        fitted.floor()
    } else {
        fitted
    };
    // Clamped to the window: `scale` is a ratio of the two, and at 2.4 the
    // product comes back a rounding step over the window it was derived from.
    // In whole-number mode the product is exact and this changes nothing.
    let (draw_w, draw_h) = ((w * scale).min(win_w), (h * scale).min(win_h));
    FRect::new(
        ((win_w - draw_w) / 2.0).round(),
        ((win_h - draw_h) / 2.0).round(),
        draw_w,
        draw_h,
    )
}

/// Maps a window pixel to the menu screen's own coordinate space.
///
/// Returns `None` outside the letterboxed image, where there is nothing to hit.
fn to_screen(
    canvas: &Canvas<Window>,
    menu: &Menu,
    whole: bool,
    x: f32,
    y: f32,
) -> Option<(u32, u32)> {
    // The rectangle follows the hit map's aspect; the point is handed on in the
    // space the screen composites at, which is what `Screen::hit` expects.
    let (map_w, map_h) = menu.screen().map_size();
    let dst = letterbox(canvas, map_w, map_h, whole);
    let (w, h) = menu.screen().size();
    let sx = (x - dst.x) / dst.w * w as f32;
    let sy = (y - dst.y) / dst.h * h as f32;
    (sx >= 0.0 && sy >= 0.0 && sx < w as f32 && sy < h as f32).then_some((sx as u32, sy as u32))
}

/// Plays one script to its end, or until the player quits.
fn run_script(
    player: &mut Player,
    canvas: &mut Canvas<Window>,
    creator: &TextureCreator<WindowContext>,
    events: &mut EventPump,
    script: Script,
    start_ini: &Ini,
    mut progress: Option<&mut Progress>,
) -> Result<Outcome> {
    player.mixer.stop_all();
    // Nothing carries over from the last script: `FUN_004236f0` clears the
    // peripheral's three members as it frees one.
    player.pads.stop();
    // A direction held on the way in would otherwise repeat into the first
    // frame of the script.
    player.controls.clear();

    // The background as it will be shown: mouth patches already in it, scaled
    // to the window. Keyed by everything that decides those pixels, so a pass
    // that changes none of them reuses the texture.
    let mut still_texture: Option<StillFrame<'_>> = None;
    let mut movie_texture: Option<MovieFrame<'_>> = None;
    // Stills go through libswscale, the same scaler and the same filter a movie
    // frame goes through — they are two ways of filling the same 800x452 stage,
    // and a still that went through a different filter did not match the clip
    // it cut to. See `daysengine::media::ImageScaler`.
    let mut scaler = ImageScaler::new(player.settings.video_scaler)
        .context("building the still-image scaler")?;
    // The wrapped lines and a texture each, cached on the script's own line.
    let mut text_texture: Option<DialogueBlock<'_>> = None;
    let mut choice_labels: Option<ChoiceLabels<'_>> = None;

    let mut stage = Stage::new(script);
    stage.set_video_scaler(player.settings.video_scaler);
    stage.set_video_filters(
        &player.settings.video_filters,
        &player.settings.video_filters_after,
        player.settings.video_grain,
    );
    // Whole-number scaling, and what it means for how the art is sampled.
    let whole = player.whole_pixels();
    let art = art_sampling(whole);
    let mut config = Config::load(&player.game);
    // `[UseEnglish]` decides the dialogue pitch and whether it wraps at all.
    let english = player.film.get_bool("UseEnglish").unwrap_or(false);
    // `[LeftArrangement]` picks per-line centring or a left-aligned block.
    let left_arrangement = player.film.get_bool("LeftArrangement").unwrap_or(false);

    // The control bar and the choice box both come out of the install, and a
    // missing one has to leave playback alone: this is the UI over a movie, not
    // the movie. So both are loaded with a warning and the loop checks for them.
    let mut control = match Bar::load(player.vfs, &player.dll, player.resolution()) {
        Ok(bar) => Some(bar),
        Err(err) => {
            log::warn!("the control bar is unavailable: {err}");
            None
        }
    };
    let mut bar_state = bar::State::from_config(&config);
    // `FUN_00423a70` calls MenuBar vtable `+0x38` when playback starts, which
    // puts the gauge at the counters as they stand, with no ramp, and lowers
    // it. The bar here is built once per script, so this is that call.
    if let Some(p) = progress.as_deref_mut() {
        let ((first, second), _) = p.gauge();
        if let Some(control) = &mut control {
            control.settle_gauge(first, second);
        }
        p.lower_gauge();
    }
    // A script always starts at 1x. The original does this twice over:
    // `FUN_00423130` initialises the rate member `+0x538` to 1.0 when a session
    // starts, and `FUN_004236f0` puts it back to 1.0 when a script is freed.
    // The mixer outlives both, so it has to be told.
    player.mixer.set_rate(bar_state.rate);
    // The bar's own fade, and the frame the auto flag was last set on.
    let mut auto_since = Instant::now();
    let mut hovered: Option<usize> = None;
    // Cached on the record list, so the strip is only recomposited when it
    // actually changes — which is on a hover, a state change or an auto frame.
    let mut bar_texture: Option<(BarLayer, u32, u32, Texture)> = None;
    let mut indicator_texture: Option<(u32, u32, Texture)> = None;
    let mut choice: Option<(Choice, Select)> = None;
    // The playback rate the player had when a choice went up, to put back when
    // they answer it. See the `Raised`/`Decided` arm below.
    let mut speed_before_choice: Option<usize> = None;

    // The clock is wall-clock based with an offset, so pausing and seeking are
    // both just adjustments to the offset rather than separate state machines.
    let mut origin = Instant::now();
    let mut offset = Frame::ZERO;
    let mut paused = false;
    let mut pointer = (0.0f32, 0.0f32);
    let mut buttons = (false, false);
    // Which control bar widget the selection is on, for a player who is not
    // using a pointer. `None` leaves the bar to the pointer, which is the only
    // way the original has of reaching it at all.
    let mut bar_focus: Option<usize> = None;
    // A wall clock for the bar's fade, which ramps in milliseconds of real time
    // and so cannot hang off the script clock: the script clock stops when
    // playback is paused, and the bar still has to fade.
    let start = Instant::now();

    // Paced to the display, and deciding on its grid rather than on whatever
    // moment the last pass ended.
    let mut cadence = Cadence::new(canvas);
    loop {
        let now = cadence.tick();
        // The rate the clock runs at, from the speed widget that is lit. Read
        // once per frame because every use of the clock in this iteration has
        // to agree about it.
        let rate = bar::SPEEDS[bar_state.speed.min(bar::SPEEDS.len() - 1)];

        // What the player asked for this pass, and where it goes. A control
        // bar widget the selection is on is pressed by pushing its number
        // here, which is the same list a click produces — so a button and a
        // click reach the bar's dispatch by one road, with the bar's own
        // enable rules in front of both.
        let mut asked: Vec<Control> = Vec::new();
        let mut pressed: Vec<usize> = Vec::new();
        let mut answer = Input::default();
        for event in events.poll_iter() {
            match &event {
                Event::Quit { .. } => return Ok(Outcome::Quit),
                Event::GamepadAdded { which, .. } => player.pads.added(*which),
                Event::GamepadRemoved { which, .. } => player.pads.removed(*which),
                Event::MouseMotion { x, y, .. } => {
                    pointer = (*x, *y);
                    // The pointer has moved, so it owns the bar again: two
                    // highlights disagreeing about which widget is hovered is
                    // worse than either alone.
                    bar_focus = None;
                }
                Event::MouseButtonDown {
                    mouse_btn, x, y, ..
                } => {
                    pointer = (*x, *y);
                    bar_focus = None;
                    match mouse_btn {
                        MouseButton::Left => buttons.0 = true,
                        MouseButton::Right => buttons.1 = true,
                        _ => {}
                    }
                }
                Event::MouseButtonUp { mouse_btn, .. } => match mouse_btn {
                    MouseButton::Left => buttons.0 = false,
                    MouseButton::Right => buttons.1 = false,
                    _ => {}
                },
                _ => {}
            }
            asked.extend(player.controls.take(&event, now));
        }
        asked.extend(player.controls.due(now));
        if let Some((dx, dy)) = player.controls.cursor(&player.pads, cadence.interval()) {
            let (w, h) = canvas.window().size();
            pointer.0 = (pointer.0 + dx).clamp(0.0, w.saturating_sub(1) as f32);
            pointer.1 = (pointer.1 + dy).clamp(0.0, h.saturating_sub(1) as f32);
            bar_focus = None;
        }

        // Where a direction goes depends on what is on screen, the same way
        // the original's own eight input slots do: with a choice box up they
        // are the box's four (`FUN_0044de50` reads slots 4 to 7), with the
        // selection on the bar they walk it, and with neither they seek, which
        // is what the arrow keys have always done here.
        //
        // Both snapshots are taken before the actions are read, because one
        // press can carry two actions — Escape is bound to Cancel and to Quit,
        // and a Cancel that took the selection off the bar must not let the
        // Quit behind it through.
        let answering = choice.is_some();
        let focused = bar_focus.is_some();
        for control in asked {
            match control {
                Control::Quit if !answering && !focused => return Ok(Outcome::Quit),
                Control::Cancel if answering => answer.cancel = true,
                Control::Cancel => bar_focus = None,
                Control::Confirm if answering => answer.confirm = true,
                // A confirm with nothing to confirm pauses. The bar is a strip
                // the pointer hovers rather than something that holds a
                // selection, and most of a script has no choice box up, so
                // without this the confirm button is dead for most of the
                // game — and pausing is what Space has always done here.
                Control::Confirm => match bar_focus {
                    Some(widget) => pressed.push(widget),
                    None => pressed.push(bar::widget::PAUSE),
                },
                Control::Up if answering => answer.prev = true,
                Control::Down if answering => answer.next = true,
                // Up reaches for the bar, which is where the bar is: a strip
                // along the top of the picture. Down lets it go again.
                Control::Up if !focused => bar_focus = first_bar_widget(bar_state),
                Control::Up => {}
                Control::Down => bar_focus = None,
                Control::FocusBar => {
                    bar_focus = if focused {
                        None
                    } else {
                        first_bar_widget(bar_state)
                    }
                }
                Control::Left if answering => answer.prev = true,
                Control::Right if answering => answer.next = true,
                Control::Left | Control::Right => match bar_focus {
                    Some(widget) => {
                        bar_focus = step_bar(widget, control == Control::Right, bar_state)
                    }
                    None => {
                        let at = clock(origin, now, offset, rate);
                        let delta = 5 * FPS;
                        offset = if control == Control::Right {
                            Frame(at.0 + delta)
                        } else {
                            Frame(at.0.saturating_sub(delta))
                        };
                        origin = now;
                    }
                },
                Control::SeekForward | Control::SeekBack => {
                    let at = clock(origin, now, offset, rate);
                    let delta = 5 * FPS;
                    offset = if control == Control::SeekForward {
                        Frame(at.0 + delta)
                    } else {
                        Frame(at.0.saturating_sub(delta))
                    };
                    origin = now;
                }
                // The rest are the bar's own widgets, pressed by number. The
                // bar decides whether each is live, plays the click and
                // dispatches, exactly as it does for a click on it.
                // Asked for directly, so it pauses whatever else is on
                // screen — unlike the confirm above, which only reaches this
                // when nothing else takes it.
                Control::Pause => pressed.push(bar::widget::PAUSE),
                Control::Auto => pressed.push(bar::widget::AUTO),
                Control::Restart => pressed.push(bar::widget::RESTART),
                Control::SkipToChoice => pressed.push(bar::widget::SKIP),
                Control::SaveMenu => pressed.push(bar::widget::SAVE),
                Control::LoadMenu => pressed.push(bar::widget::LOAD),
                Control::OptionMenu => pressed.push(bar::widget::OPTION),
                Control::LeavePlayback => pressed.push(bar::widget::LEAVE),
                Control::Faster => pressed.push(bar::widget::speed(bar_state.speed + 1)),
                Control::Slower => {
                    pressed.push(bar::widget::speed(bar_state.speed.saturating_sub(1)))
                }
                _ => {}
            }
        }

        let at = if paused {
            offset
        } else {
            clock(origin, now, offset, rate)
        };
        if stage.finished(at) {
            log::info!("script finished");
            return Ok(Outcome::Finished);
        }
        // Worked out here because the bar is handled while `visual` holds the
        // stage, and this has to be read off the script before that.
        let skip_target = stage.skip_target(at);

        // Where the picture lands, worked out before the frame is asked for:
        // the decoder scales to it, so it has to know first.
        let dst = letterbox(canvas, STAGE_WIDTH, STAGE_HEIGHT, player.whole_pixels());
        let scale = dst.h / STAGE_HEIGHT as f32;
        let window_px = (dst.w.round().max(1.0) as u32, dst.h.round().max(1.0) as u32);
        stage.set_video_size(window_px.0, window_px.1);

        // The three volume sliders, pushed at the sound every tick. That is
        // how the original does it — `FUN_0043ea80`, `FUN_00429250` and
        // `FUN_0043c900` each re-ask `_GetMasterVolume@4` for their category
        // and set it on every object they own, every frame — so a slider moved
        // on the Sound tab the bar just opened takes hold on the sound in hand
        // rather than at the next script. See `daysengine::playback::mixer`.
        apply_volumes(&config, player.mixer);
        // Male voice lines are refused while `MenVoice` is off, and the
        // original asks it per tick rather than per statement: `FUN_0043c900`
        // walks the live voice list every frame and `FUN_0044e800` calls
        // `GetMenVoice` on every clip that has not started yet. So the option
        // is handed over here, on the tick, and a line the player turns back on
        // part-way through starts then. See `Stage::set_men_voice`.
        stage.set_men_voice(config.flag(Flag::MenVoice));
        stage.seek_to(at, player.vfs, player.mixer)?;
        let visual = stage.visual_at(at);

        // The peripheral. `FUN_0043dbe0` asks two things of the host before it
        // moves it — the engine is on its playback tick, and the lit speed
        // widget is 1x — and the statement's own window is the rest:
        // `FUN_0042a250` stops the device once the frame reaches the end it
        // stored. Pausing is one of the states that is not the playback tick,
        // which is why it stops here too. See `daysengine::playback::som`.
        player.pads.set_level(
            visual
                .som
                .filter(|_| som::gated(!paused, bar_state.speed))
                .unwrap_or(0),
        );
        player.pads.renew(now);

        canvas.set_draw_color(Color::BLACK);
        canvas.clear();

        // The control bar's own space is 800x75 with its origin at the strip's
        // top-left corner; the engine places that strip, and the DLL's
        // `FUN_10021c20` gives the base sprite the same half-pixel inset every
        // other sprite gets, so the origin is the only placement there is.
        bar_state.paused = paused;
        // A wall clock, because the gauge's ramp and the bar's fade are both in
        // milliseconds of real time and neither stops when playback is paused.
        let now_ms = start.elapsed().as_millis().min(u128::from(u32::MAX)) as u32;
        // The gauge draws the two counters the branch system keeps, and shows
        // over a faded bar only while a delta has raised it.
        //
        // While it is up it is also *moving*: `FUN_10024c60`, the bar's
        // graphics-module update pass, steps the ramp every frame under that
        // same flag. The ramp is what plays the rise or the fall, slides the
        // gauge to the new lead, holds it there and then puts it down again —
        // so this is also where the flag is cleared in ordinary play.
        if let Some(p) = progress.as_deref_mut() {
            let ((first, second), raised) = p.gauge();
            bar_state.gauge = Some((first, second));
            bar_state.gauge_raised = raised;
            if raised {
                if let Some(control) = &mut control {
                    let tick = control.advance_gauge(now_ms, first, second);
                    if let Some(se) = tick.sound {
                        player
                            .system_se
                            .play(se, player.vfs, &mut player.sounds, player.mixer);
                    }
                    if tick.lowered {
                        p.lower_gauge();
                        bar_state.gauge_raised = false;
                    }
                }
            }
        }
        // Host `+0x98`, which is what the bar's right-hand box is about: the
        // slider lights up, its ten cells become pressable, and the REPLAYMODE
        // indicator goes on the picture.
        bar_state.following_record = player.following_record;
        // `set_speed` keeps these two in step; this is the resting case, for
        // a session that has not touched a speed widget yet.
        bar_state.rate = rate;
        if let Some(control) = &mut control {
            let (bw, bh) = control.strip();
            let strip = bar_strip(dst, (bw, bh));
            // The strip's own space, which is where its hit map is indexed.
            let bar_scale = strip.w / bw as f32;
            // Whether the pointer is inside the strip's own rectangle at all is
            // the question the bar's visibility turns on, so it is asked here
            // and not derived from whether a widget was hit.
            //
            // A selection on the bar stands in for the pointer being on it.
            // The fade, the caption strip and the dispatch are every one of
            // them asked about a *position* — the bar has no notion of a
            // selection, because the original has no way of reaching it
            // without a pointer — so the selection becomes the position its
            // widget occupies, and everything downstream is the pointer path
            // unchanged.
            let (sx, sy) = match bar_focus.and_then(|w| control.screen().widget_point(w)) {
                Some((x, y)) => (x as f32, y as f32),
                None => (
                    (pointer.0 - strip.x) / bar_scale,
                    (pointer.1 - strip.y) / bar_scale,
                ),
            };
            let over = (sx >= 0.0 && sy >= 0.0 && sx < bw as f32 && sy < bh as f32)
                .then_some((sx as u32, sy as u32));
            hovered = control.point_at(over, now_ms, bar_state.gauge_raised);

            // A click only reaches the bar while the bar is actually on
            // screen, and only the widget it lands on takes it. A press the
            // bar has no use for is left alone: the choice box reads the same
            // button from the same place this frame, through host slot
            // `+0x148`, and swallowing it here is why a choice could not be
            // clicked at all.
            if buttons.0 && control.fade().drawn() {
                if let Some(widget) = hovered {
                    // Consume the press, so a bar widget and a choice box
                    // under it do not both answer to one click.
                    buttons.0 = false;
                    pressed.push(widget);
                }
            }
            // Clicks and bound buttons both arrive here, as widget numbers, so
            // there is one dispatch and one set of enable rules for the two.
            for widget in std::mem::take(&mut pressed) {
                {
                    let act = control.press(widget, bar_state, at.0);
                    if act != bar::Act::None {
                        player.system_se.play(
                            SystemSe::Click,
                            player.vfs,
                            &mut player.sounds,
                            player.mixer,
                        );
                    }
                    match act {
                        bar::Act::ToggleAuto => {
                            bar_state.auto = !bar_state.auto;
                            auto_since = Instant::now();
                        }
                        // Host `+0xf4` is `FUN_00424f40`, the same
                        // `FUN_00424e20` / `FUN_00424eb0` pair every other
                        // suspension of playback uses, so pausing stops the
                        // script's sound where it stands and un-pausing starts
                        // it again there rather than a pause's worth of audio
                        // further on.
                        bar::Act::TogglePause => {
                            if paused {
                                origin = now;
                                player.mixer.resume_script();
                            } else {
                                offset = clock(origin, now, offset, rate);
                                player.mixer.pause_script();
                            }
                            paused = !paused;
                        }
                        // Host `+0x8c` is `FUN_00424f90`, and it does three
                        // things in this order: it stops the clock, sets the
                        // rate, and starts the clock again.
                        //
                        // Pressing the rate that is already selected is the
                        // one case that does none of that — the original
                        // compares against its `+0x504` first and only stores
                        // the lit index — so the clock is not disturbed by
                        // pressing 1x twice.
                        //
                        // Otherwise `FUN_00424910` folds the frames run so far
                        // into the base (`+0x540 = +0x544`) and `FUN_00424a10`
                        // restarts from there, so the new rate applies from now
                        // on and the frame never jumps backwards. That is the
                        // fold-and-rebase below.
                        //
                        // The audio is retimed with the picture, because the
                        // original retimes it: `FUN_00429500` hands the rate to
                        // `FUN_004433d0`, which sets it on the stream through
                        // `FUN_0041a050` and **mutes above 4.0** — so 1x, 2x
                        // and 4x are heard, resampled, and 12x and 24x are
                        // silent. `Mixer::set_rate` is both halves of that.
                        bar::Act::Speed(index) => {
                            if bar_state.set_speed(index) {
                                offset = clock(origin, now, offset, rate);
                                origin = now;
                                player.mixer.set_rate(bar_state.rate);
                            }
                        }
                        bar::Act::Seek(code) if code == bar::Seek::RESTART => {
                            offset = Frame::ZERO;
                            origin = now;
                        }
                        // Skip jumps to the choice this script raises, if it
                        // still has one ahead. `FUN_00425bf0`'s case 6 is the
                        // state `+0xfc(5)` selects, and it is the only seek
                        // that is not "this script is over" — see
                        // `Stage::skip_target` for the rule and the landing
                        // frame. With nothing to skip to here, the chase moves
                        // on to the scripts after this one.
                        bar::Act::Seek(code) if code == bar::Seek::SKIP => match skip_target {
                            Some(to) => {
                                log::info!("skipping from {at} to the choice at {to}");
                                offset = to;
                                origin = now;
                            }
                            None => return Ok(Outcome::SkipToChoice),
                        },
                        // Everything past a restart lands in the executable's
                        // state 4, which is the "this script is finished" path:
                        // it asks `_GetNextScriptFile@12` what follows and
                        // plays that.
                        bar::Act::Seek(_) => return Ok(Outcome::Finished),
                        // Host `+0x100(1)` leaves playback rather than moving
                        // along it, so this one goes back to the title.
                        bar::Act::Leave => return Ok(Outcome::Play),
                        // These are `setSystemInit`'s own codes: 4 opens the
                        // save/load module to save, 5 to load and 2 the Option
                        // screen. Code 3 has a case too — it selects the module
                        // object `DAT_1004ffc8` — but **which screen that is
                        // has not been recovered**, so the bar's third menu
                        // button is the one this engine cannot answer.
                        bar::Act::Menu(request) => {
                            let opened = match request.0 {
                                4 => Some((Mode::SAVELOAD, saveload::Kind::Save)),
                                5 => Some((Mode::SAVELOAD, saveload::Kind::Load)),
                                // The Option screen ignores the save/load job.
                                2 => Some((Mode::OPTION, saveload::Kind::Load)),
                                _ => None,
                            };
                            let Some((mode, kind)) = opened else {
                                log::info!(
                                    "the bar asked for menu {}, which is not recovered",
                                    request.0
                                );
                                continue;
                            };
                            // Stop the script clock so playback resumes where
                            // it was, open the menus, then put it back. The
                            // script's sound stops with the clock: host `+0xf8`
                            // (`FUN_0042a430`) calls `FUN_00424e20` before it
                            // switches the playback object into its menu layer,
                            // and `FUN_00425550` case 8 calls `FUN_00424eb0` on
                            // the way back. So the scene is held on the frame it
                            // was interrupted on rather than playing on under a
                            // screen the player is reading. The menus' own
                            // sounds are a channel of their own and keep
                            // sounding, at 1x, whatever speed the bar was at.
                            offset = clock(origin, now, offset, rate);
                            player.mixer.pause_script();
                            let outcome = run_menu(
                                player,
                                canvas,
                                creator,
                                events,
                                start_ini,
                                MenuEntry::OverPlayback(mode, kind),
                                progress.as_deref_mut(),
                            )?;
                            player.mixer.resume_script();
                            // The SOMCON tab may have left its test running,
                            // and the tick below is about to say what the
                            // script asks for anyway.
                            player.pads.stop();
                            // The Sound tab may have moved `MenVoice`, which
                            // the loop hands to the stage on its next tick.
                            // Re-reading the file lands on the same value at
                            // the same frame as the original does: the DLL
                            // writes the member `GetMenVoice` reads the moment
                            // the widget is pressed (`FUN_10008260`, widgets 10
                            // and 11), the close button flushes it, and the
                            // script clock is stopped for all of it.
                            config = Config::load(&player.game);
                            // The Option screen can change the display mode,
                            // which moves both the art set the bar draws from
                            // and the rate the window is presented at.
                            if let Err(err) =
                                control.set_resolution(player.vfs, &player.dll, player.resolution())
                            {
                                log::warn!(
                                    "no control bar art at {}: {err}",
                                    player.resolution().name()
                                );
                            }
                            // The menu may have moved the window to another
                            // display, so the grid is read again — and the
                            // clock re-bases onto it, not onto the wall clock,
                            // so the first frame back is on the beat.
                            cadence = Cadence::new(canvas);
                            origin = cadence.tick();
                            // Every cached texture belonged to the menu's
                            // renderer; drop them so playback rebuilds.
                            movie_texture = None;
                            choice_labels = None;
                            bar_texture = None;
                            still_texture = None;
                            text_texture = None;
                            // Saving no longer ends the menus: the screen
                            // writes the slot itself and stays up, which is
                            // what `FUN_10014c90` does.
                            //
                            // `Outcome::Play` means two different things
                            // depending on who said it. From the bar's own
                            // leave button (`+0x100(1)`) it means stop playing.
                            // From a menu the bar opened it is `+0x4c(0)`, the
                            // Close button, and `FUN_00425550` case 8 puts the
                            // engine back into state 1 — the playback tick. So
                            // here it means resume this script where it was
                            // paused, and returning it would hand the outer
                            // loop a finished script and land the player on the
                            // title, which is the bug this whole path is about.
                            match outcome {
                                Outcome::Play => {}
                                Outcome::Quit => return Ok(Outcome::Quit),
                                other => return Ok(other),
                            }
                        }
                        // Applied inside the bar, as `FUN_10026ed0` applies
                        // it: nothing out here has to act on it.
                        bar::Act::Transparency(level) => {
                            log::info!("the replay indicator is now at {level} of 10")
                        }
                        bar::Act::None => {}
                    }
                }
            }
            control.expire_latch(at.0);
            // A press can turn the widget the selection is on dead — skipping
            // to the choice takes the skip widget with it. A selection resting
            // on a dead widget looks like a control that has stopped working,
            // so it moves along to the next live one.
            if let Some(widget) = bar_focus {
                if !bar::enabled(widget, bar_state) {
                    bar_focus = step_bar(widget, true, bar_state);
                }
            }
        }

        // The choice box. It is raised and decided by the script clock, not by
        // the player: an ignored choice still resolves when its window runs out.
        match (&visual.select, &mut choice) {
            (Some(window), None) => {
                let pending = Choice::new(window.a, window.b, window.start, window.end);
                match Select::load(
                    player.vfs,
                    player.film,
                    pending.count(),
                    player.resolution(),
                ) {
                    Ok(map) => choice = Some((pending, map)),
                    Err(err) => log::warn!("the choice box is unavailable: {err}"),
                }
            }
            (None, Some(_)) => choice = None,
            _ => {}
        }
        if let Some((pending, map)) = &mut choice {
            // The four navigation slots are the original's own: host
            // `+0x148` carries eight buttons and `FUN_0044de50` reads six of
            // them — the pointer's pick and dismiss, and slots 4 to 7 for
            // previous, next, confirm and cancel. The pointer half was always
            // here; the other four are what a player without one presses.
            let input = Input {
                pointer: (
                    f64::from((pointer.0 - dst.x) / dst.w),
                    f64::from((pointer.1 - dst.y) / dst.h),
                ),
                pick: buttons.0,
                dismiss: buttons.1,
                ..answer
            };
            let event = pending.tick(at, map, input, bar_state.auto, &mut |n| {
                // The original seeds from `GetTickCount` and draws once; any
                // source of the same range does the same job.
                (Instant::now().elapsed().subsec_nanos() as usize ^ at.0 as usize) % n.max(1)
            });
            // `FUN_00431740` settles the answer before it picks the sound, and
            // two of the things it does there are the play-data list's.
            //
            // While host `+0x98` is up — the list started this playthrough to
            // follow a slot's own answers — a box **nobody answered** takes
            // `FUN_00428a80`, the answer that slot recorded at this script, and
            // then rings as whatever that answer is rather than as a cancel.
            // And the moment the player answers a box themselves the flag comes
            // down (`+0x94(0)`), which is why following stops the first time
            // they disagree with the recording. The flag is also what gates the
            // write: `FUN_00428a50` runs only with it clear, so a followed
            // playthrough does not overwrite the recording it is reading.
            let mut event = event;
            if let select::Event::Decided(index, _) = event {
                if player.following_record {
                    if index < 0 {
                        if let Some(answer) =
                            progress.as_deref().and_then(Progress::recorded_choice)
                        {
                            log::info!("the slot recorded {answer} here, so that is the answer");
                            event = select::Event::Decided(
                                answer,
                                if answer < 0 {
                                    SystemSe::Cancel
                                } else {
                                    SystemSe::Select
                                },
                            );
                        }
                    } else {
                        log::info!("the player answered, so the recording is no longer followed");
                        player.following_record = false;
                    }
                }
            }
            match event {
                select::Event::Raised(se) | select::Event::Decided(_, se) => {
                    player
                        .system_se
                        .play(se, player.vfs, &mut player.sounds, player.mixer);
                    // A choice is decided at 1x, whatever the player was
                    // fast-forwarding at, and the speed they had comes back
                    // when they answer. `FUN_00431740` calls
                    // `FUN_004250b0(engine, 0)` as the box goes up — which
                    // saves the live index into `+0x534` before overwriting
                    // `+0x530` — and `FUN_004316b0` calls `FUN_004251d0`,
                    // which puts `+0x534` back. Both are skipped while the
                    // auto flag (host `+0x134`) is set, because that is the
                    // mode that answers the box for you.
                    //
                    // Re-basing the clock is part of it: the position is
                    // `base + elapsed * 24 * rate` (`FUN_00422f70`), so
                    // changing the rate without moving the base would move the
                    // frame as well as the speed.
                    if !bar_state.auto {
                        let want = match event {
                            select::Event::Raised(_) => {
                                speed_before_choice = Some(bar_state.speed);
                                0
                            }
                            _ => speed_before_choice.take().unwrap_or(bar_state.speed),
                        };
                        if bar_state.set_speed(want) {
                            offset = clock(origin, now, offset, rate);
                            origin = now;
                            player.mixer.set_rate(bar_state.rate);
                        }
                    }
                    if let select::Event::Decided(index, _) = event {
                        // The engine credits the choice's deltas the moment
                        // the box settles, before the script has ended --
                        // `FUN_00431740` calls `_SetFeeling@8(host, 1)` right
                        // after storing the index.
                        log::info!("choice decided: {index}");
                        player.last_choice = index;
                        if let Some(p) = progress.as_deref_mut() {
                            p.decide(index, !player.following_record);
                        }
                    }
                }
                select::Event::Nothing => {}
            }
        }

        // A movie frame arrives at the size the window wants it, because the
        // decoder was asked for that size: libswscale folds the scale into the
        // colour conversion every frame goes through anyway, in hand-written
        // SIMD. See `daysengine::media::VideoDecoder::set_output_size`. What is
        // left here is the upload, and that happens once per *picture* — the
        // loop spins much faster than 24 fps to stay responsive to the pointer,
        // so it sees each frame several times over. `movie_id` is what says two
        // of those are the same picture.
        if let Some(frame) = visual.movie {
            let showing = visual
                .movie_id
                .map(|(clip, index)| (clip, index, window_px));
            let stale = movie_texture
                .as_ref()
                .is_none_or(|held| showing != Some((held.clip.as_str(), held.index, held.window)));
            if stale {
                let size = (frame.width, frame.height);
                if movie_texture.as_ref().is_none_or(|held| held.size != size) {
                    movie_texture = Some(MovieFrame {
                        // Nothing is in it yet; the upload below is what makes
                        // an identity true.
                        clip: String::new(),
                        index: 0,
                        window: (0, 0),
                        size,
                        texture: new_texture(creator, size.0, size.1, ScaleMode::Linear)?,
                    });
                }
                if let Some(held) = &mut movie_texture {
                    held.texture
                        .update(None, &frame.rgba, frame.width as usize * 4)
                        .context("uploading movie frame")?;
                    if let Some((clip, index, window)) = showing {
                        held.clip.clear();
                        held.clip.push_str(clip);
                        held.index = index;
                        held.window = window;
                    }
                }
            }
            if let Some(MovieFrame { texture, .. }) = &movie_texture {
                canvas
                    .copy(texture, None, dst)
                    .map_err(|e| anyhow::anyhow!("drawing movie: {e}"))?;
            }
        } else if let Some(still) = visual.still {
            // Scaled to the window here, unless whole-number scaling is on —
            // then it goes up at its own size for the blit to multiply. Rebuilt
            // when the background changes, when a mouth moves, or when the size
            // it was built for does.
            let at = if whole {
                (still.width, still.height)
            } else {
                window_px
            };
            let mouths: Vec<_> = visual
                .mouths
                .iter()
                .map(|(m, index)| (m.x, m.y, m.width, m.height, *index))
                .collect();
            let stale = still_texture.as_ref().is_none_or(|held| {
                held.path != still.path || held.size != at || held.mouths != mouths
            });
            if stale {
                // The mouths go into the background's own surface first, at the
                // background's own size. That is what the original does — a
                // straight `memcpy` into the surface in `FUN_00444b80`, before
                // anything composites or scales it — and doing it in this order
                // is what keeps the patch on the same pixel grid as the face
                // around it. Scaling the two separately put the patch on a grid
                // of its own, a fraction of a pixel out at 1:1 and a visible
                // step once the window was any bigger.
                let src = (still.width, still.height);
                let patched = compose_mouths(&still.rgba, src, &visual.mouths);
                let base = patched.as_deref().unwrap_or(&still.rgba);
                let scaled = scaler
                    .scale(base, src, at)
                    .context("scaling the background")?;
                let (w, h, rgba) = match &scaled {
                    Some(pixels) => (at.0, at.1, pixels.as_slice()),
                    None => (still.width, still.height, base),
                };
                let mut texture = new_texture(creator, w, h, art)?;
                texture.update(None, rgba, w as usize * 4)?;
                still_texture = Some(StillFrame {
                    path: still.path.clone(),
                    size: at,
                    mouths,
                    texture,
                });
            }
            if let Some(held) = &still_texture {
                canvas
                    .copy(&held.texture, None, dst)
                    .map_err(|e| anyhow::anyhow!("drawing background: {e}"))?;
            }
        }

        if let Some(([r, g, b], opacity)) = visual.fade {
            if opacity > 0.0 {
                canvas.set_blend_mode(BlendMode::Blend);
                canvas.set_draw_color(Color::RGBA(r, g, b, (opacity * 255.0) as u8));
                canvas
                    .fill_rect(dst)
                    .map_err(|e| anyhow::anyhow!("drawing fade: {e}"))?;
            }
        }

        if let Some((speaker, line)) = visual.text {
            // Broken into lines the way `FUN_0043f600` breaks them, then
            // stacked at the recovered `0x30` pitch. Cached on the joined
            // lines: laying one out costs a 48x48 glyph decode per character,
            // which is wasteful at 24 fps.
            // The speaker field is not drawn: the original never hands it to
            // the text layer, only to the backlog. See `playback::text`.
            let _ = speaker;
            let stale = text_texture
                .as_ref()
                .is_none_or(|cached| cached.source != line);
            if stale {
                let lines = text::wrap(line, english);
                let mut drawn = Vec::new();
                for one in &lines {
                    let image = text::render_line(player.font, one, [255, 255, 255], english);
                    // Glyphs, and never at a whole-number scale: the block is
                    // laid out at 0.75 of its own size before the window's
                    // scale is applied. See `playback::text`.
                    let mut texture = new_texture(
                        creator,
                        image.width as u32,
                        image.height as u32,
                        ScaleMode::Linear,
                    )?;
                    texture.set_blend_mode(BlendMode::Blend);
                    texture.update(None, &image.rgba, image.width * 4)?;
                    drawn.push(texture);
                }
                text_texture = Some(DialogueBlock {
                    source: line.to_string(),
                    lines,
                    drawn,
                });
            }
            if let Some(block_lines) = &text_texture {
                // Placed by `FUN_0044bf30`: centred on each line's own width
                // and anchored to the bottom, at the 800x450 scale of 0.75.
                // `scale` on top of that is only this window's letterbox.
                let geometry = text::Geometry::native(left_arrangement);
                let places = text::place(&block_lines.lines, english, geometry);
                for (texture, at) in block_lines.drawn.iter().zip(places) {
                    canvas
                        .copy(
                            texture,
                            None,
                            FRect::new(
                                dst.x + at.x * scale,
                                dst.y + at.y * scale,
                                at.width * scale,
                                at.height * scale,
                            ),
                        )
                        .map_err(|e| anyhow::anyhow!("drawing text: {e}"))?;
                }
            }
        }

        // The choice labels. The original places these with `FUN_0044ced0`, and
        // that formula is written down in `ui::select` but not used here yet:
        // each label is centred in the box the shipped hit map gives, which is
        // exact data and agrees with the hit testing.
        if let Some((pending, map)) = &choice {
            if pending.visible(at) {
                if let Some(((mw, mh), boxes)) = map.map_size().zip(map.bounds()) {
                    // What colour each label is this frame, and whether it is
                    // drawn at all: lit or plain while the box is live, and
                    // ramping to nothing once it has been answered.
                    let colours: Vec<Option<select::Rgba>> = (0..pending.labels.len())
                        .map(|index| pending.label_colour(index, at))
                        .collect();
                    // Rebuilt when the labels change, which is once, or when
                    // the colour under one of them changes — the pointer
                    // moving to another box, or the answer starting the fade.
                    // The alpha is a modulation on the finished texture, so a
                    // fade does not re-render a glyph 60 times a second.
                    let rgb: Vec<Option<[u8; 3]>> = colours
                        .iter()
                        .map(|c| c.map(|c| [c.red, c.green, c.blue]))
                        .collect();
                    let stale = choice_labels.as_ref().is_none_or(|cached| {
                        cached.colours != rgb || cached.labels != pending.labels
                    });
                    if stale {
                        let mut drawn = Vec::new();
                        for (index, label) in pending.labels.iter().enumerate() {
                            let colour = rgb[index].unwrap_or([0, 0, 0]);
                            let image = text::render_line(player.font, label, colour, english);
                            let mut texture = new_texture(
                                creator,
                                image.width as u32,
                                image.height as u32,
                                ScaleMode::Linear,
                            )?;
                            texture.set_blend_mode(BlendMode::Blend);
                            texture.update(None, &image.rgba, image.width * 4)?;
                            drawn.push((image.width as u32, image.height as u32, texture));
                        }
                        choice_labels = Some(ChoiceLabels {
                            labels: pending.labels.clone(),
                            colours: rgb,
                            drawn,
                        });
                    }
                    if let Some(cached) = &mut choice_labels {
                        // `FUN_0044ced0` gives a label the font's whole
                        // 48-pixel cell — unlike a dialogue line, which
                        // `_DAT_004d6770` squashes to 42 — so a rendered label
                        // goes into layout space at the geometry's own scale.
                        // `scale` on top of that is only this window's
                        // letterbox, exactly as for the dialogue above.
                        let geometry = text::Geometry::native(left_arrangement);
                        let text_scale = select::label_scale(geometry) * scale;
                        for (index, (w, h, texture)) in cached.drawn.iter_mut().enumerate() {
                            let Some(region) = boxes.get(index) else {
                                continue;
                            };
                            let Some(colour) = colours[index] else {
                                continue;
                            };
                            texture.set_alpha_mod(colour.alpha);
                            let tw = *w as f32 * text_scale;
                            let th = *h as f32 * text_scale;
                            let cx = (f32::from(region.x as u16) + region.width as f32 / 2.0)
                                / mw as f32;
                            let cy = (f32::from(region.y as u16) + region.height as f32 / 2.0)
                                / mh as f32;
                            canvas
                                .copy(
                                    texture,
                                    None,
                                    FRect::new(
                                        dst.x + dst.w * cx - tw / 2.0,
                                        dst.y + dst.h * cy - th / 2.0,
                                        tw,
                                        th,
                                    ),
                                )
                                .map_err(|e| anyhow::anyhow!("drawing a choice label: {e}"))?;
                        }
                    }
                }
            }
        }

        // The REPLAYMODE indicator is not part of the strip: it sits below it,
        // on the picture, and neither waits for the bar to drop down nor fades
        // with it. Drawn before the strip so a bar on its way in goes over it.
        if let Some(control) = control.as_ref() {
            if let Some(sign) = control.indicator(bar_state) {
                let (art_w, art_h) = (sign.art.width, sign.art.height);
                let (bw, bh) = control.strip();
                let strip = bar_strip(dst, (bw, bh));
                let scale = strip.w / bw.max(1) as f32;
                if indicator_texture
                    .as_ref()
                    .is_none_or(|(w, h, _)| (*w, *h) != (art_w, art_h))
                {
                    let mut texture = new_texture(creator, art_w, art_h, art)?;
                    texture.set_blend_mode(BlendMode::Blend);
                    texture.update(None, &sign.art.rgba, art_w as usize * 4)?;
                    indicator_texture = Some((art_w, art_h, texture));
                }
                if let Some((w, h, texture)) = &mut indicator_texture {
                    texture.set_alpha_mod(sign.alpha);
                    canvas
                        .copy(
                            &*texture,
                            None,
                            FRect::new(
                                strip.x + sign.dst.0 as f32 * scale,
                                strip.y + sign.dst.1 as f32 * scale,
                                *w as f32 * scale,
                                *h as f32 * scale,
                            ),
                        )
                        .map_err(|e| anyhow::anyhow!("drawing the replay indicator: {e}"))?;
                }
            }
        }

        // The control bar last, over everything, as its own layer — and only
        // while it is dropped down.
        // A raised gauge keeps its own alpha while the rest of the strip fades,
        // and the rate readout never had the bar's alpha at all, so the bar can
        // have something to draw after it is otherwise gone.
        if let Some(control) = control
            .as_ref()
            .filter(|c| c.fade().drawn() || c.pinned(bar_state))
        {
            let elapsed = auto_since.elapsed().as_millis().min(u128::from(u32::MAX)) as u32;
            // The cache key carries the gauge's leads as well as the record
            // list: the gauge's three pieces are sized from those and not from
            // any record, so a layer keyed on records alone would hold one
            // frame of the ramp still for the whole of it.
            // Two sprites out of the strip carry an alpha of their own once the
            // gauge is raised, and one texture cannot be modulated twice — so
            // that case composites the fade in and is keyed on both alphas.
            let pinned = control.pinned(bar_state);
            let alphas = pinned.then(|| (control.fade().alpha(), control.fade().gauge_alpha()));
            let records = (
                control.records(hovered, bar_state, elapsed),
                control.gauge_leads(),
                alphas,
            );
            let stale = bar_texture
                .as_ref()
                .is_none_or(|(cached, ..)| cached != &records);
            if stale {
                let image = if pinned {
                    control.compose_faded(hovered, bar_state, elapsed)
                } else {
                    control.compose(hovered, bar_state, elapsed)
                };
                let mut texture = new_texture(creator, image.width, image.height, art)?;
                texture.set_blend_mode(BlendMode::Blend);
                texture.update(None, &image.rgba, image.width as usize * 4)?;
                bar_texture = Some((records, image.width, image.height, texture));
            }
            if let Some((_, w, h, texture)) = &mut bar_texture {
                // The strip is cached on its record list and the fade applied
                // as an alpha modulation, so ramping does not recomposite it
                // 60 times a second. This is also how the original fades it:
                // one ARGB set on every sprite the bar owns.
                texture.set_alpha_mod(if pinned { 255 } else { control.fade().alpha() });
                canvas
                    .copy(&*texture, None, bar_strip(dst, (*w, *h)))
                    .map_err(|e| anyhow::anyhow!("drawing the control bar: {e}"))?;
            }
        }

        canvas.present();
        // A press lasts exactly one pass, whoever read it. The original
        // latches a button down in its window procedure and never handles the
        // matching button-up at all (`FUN_00466ce0`); `FUN_00467540` hands the
        // latch out and zeroes it in the same breath, and `FUN_0042b770` is
        // the one caller, taking the whole input snapshot once a frame. So
        // holding the button is not a press held down, and every consumer this
        // pass sees the same one press.
        buttons = (false, false);
        // The script clock is the authority; this only keeps the loop from
        // spinning a core between the frames the display can actually show.
        cadence.wait(now);
    }
}

/// The frame playback is at: the base frame plus the scaled time elapsed since
/// the clock was last re-based.
///
/// `now` is the instant to answer for, which the loops take from [`Cadence`]
/// rather than from the wall clock directly — see there for why.
///
/// `FUN_00422f70` is this function. `offset` is the executable's `+0x540`, the
/// frame the clock was last re-based to; `origin` stands for the `timeGetTime`
/// value it keeps at `+0x550`; and `rate` is the float at `+0x538` that host
/// slot `+0x8c` stores out of the speed table. Folding the elapsed frames back
/// into `offset` and taking a fresh `origin` is `FUN_00424910` followed by
/// `FUN_00424a10`, which is exactly what the original does on a rate change.
fn clock(origin: Instant, now: Instant, offset: Frame, rate: f32) -> Frame {
    let elapsed = now.saturating_duration_since(origin);
    Frame(offset.0 + Frame::from_duration_at(elapsed, rate).0)
}

/// Resolves a script name to its pack path, preferring the configured language.
fn find_script(vfs: &Vfs, wanted: &str, english: bool) -> Result<(String, String)> {
    let wanted_lower = wanted.to_ascii_lowercase();
    let mut candidates: Vec<&str> = vfs
        .paths()
        .filter(|p| p.starts_with("script/") && p.ends_with(".ors"))
        .filter(|p| {
            p.rsplit('/')
                .next()
                .is_some_and(|f| f.starts_with(&wanted_lower))
        })
        .collect();
    if candidates.is_empty() {
        bail!("no script named {wanted}");
    }
    // Prefer the language FILMENGINE.INI selects, but take whatever exists.
    candidates.sort_by_key(|p| {
        let is_english = p.contains("english") || p.contains(".eng.");
        (is_english != english) as u8
    });
    let path = candidates[0].to_string();
    Ok((wanted.to_uppercase(), path))
}

/// Finds the game by looking where this binary lives, then the working directory.
fn discover_game_dir() -> Result<PathBuf> {
    let candidates = [
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(Path::to_path_buf)),
        std::env::current_dir().ok(),
    ];
    for dir in candidates.into_iter().flatten() {
        if dir.join("Packs").is_dir() {
            return Ok(dir);
        }
    }
    bail!("no School Days HQ install found; put this next to the game or pass --game <dir>")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Whole-number scaling is the whole of pixel-perfect: the game's own
    /// 800x450 goes up by an integer, centred, and the rest is border. On a
    /// 1920x1200 panel that is exactly twice — not the 2.4 that fitting the
    /// window would give, and it is the 0.4 that cannot be drawn without
    /// inventing pixels.
    #[test]
    fn whole_number_scaling_multiplies_by_an_integer() {
        let content = (800, 450);
        let whole = fit((1920, 1200), content, true);
        assert_eq!((whole.w, whole.h), (1600.0, 900.0));
        assert_eq!((whole.x, whole.y), (160.0, 150.0), "centred, with a border");

        // Every pixel of the source is the same square block of the window.
        for (window, factor) in [
            ((1920, 1200), 2.0),
            ((2560, 1440), 3.0),
            ((3840, 2160), 4.0),
            ((1366, 768), 1.0),
        ] {
            let at = fit(window, content, true);
            assert_eq!(at.w, content.0 as f32 * factor, "{window:?} wide");
            assert_eq!(at.h, content.1 as f32 * factor, "{window:?} high");
            assert!(at.x >= 0.0 && at.y >= 0.0, "{window:?} overflows");
        }
    }

    /// Fitting the window is the other mode, and it fills what it can.
    #[test]
    fn fitting_the_window_uses_all_of_one_axis() {
        let at = fit((1920, 1200), (800, 450), false);
        assert_eq!((at.w, at.h), (1920.0, 1080.0));
        assert_eq!((at.x, at.y), (0.0, 60.0));
    }

    /// A window smaller than the game has no whole multiple to take, so it is
    /// fitted rather than bordered down to nothing.
    #[test]
    fn a_window_too_small_for_the_game_is_still_filled() {
        let small = fit((640, 360), (800, 450), true);
        assert_eq!((small.w, small.h), (640.0, 360.0));
    }

    /// The judder fix, which is the whole point of [`Cadence`].
    ///
    /// A 24 fps clip on a 60 Hz panel holds one source frame for three
    /// refreshes and the next for two; that ratio is 2.5 and there is no even
    /// way to spend it. What must not happen is the pattern changing with how
    /// long the last pass took — that is the difference between a beat and
    /// judder. So: run the same walk twice, once with a pass waking exactly on
    /// its refresh and once with it waking late by a jittery few milliseconds,
    /// and insist the two produce the same frame at every single refresh.
    #[test]
    fn which_frame_is_shown_does_not_depend_on_when_the_pass_woke() {
        let interval = Duration::from_secs_f64(1.0 / 59.88);
        // Milliseconds late, arbitrary, and deliberately wider than the margin
        // between a 24 fps boundary and a 60 Hz refresh — which is where a
        // frame flips sides.
        let jitter: [f64; 8] = [0.0, 2.9, 1.7, 0.4, 3.0, 1.1, 2.2, 0.8];
        let walk = |late: bool| {
            (0..600u32)
                .map(|tick| {
                    let mut wall = interval.mul_f64(f64::from(tick));
                    if late {
                        wall +=
                            Duration::from_secs_f64(jitter[tick as usize % jitter.len()] / 1000.0);
                    }
                    Frame::from_duration_at(snap(wall, interval), 1.0)
                })
                .collect::<Vec<_>>()
        };
        let on_time = walk(false);
        assert_eq!(
            walk(true),
            on_time,
            "the frame shown moved because a pass woke late"
        );

        // And what it settles into is the 3:2 beat: every frame held two or
        // three refreshes, both lengths present.
        let mut held = Vec::new();
        let mut run = 1usize;
        for pair in on_time.windows(2) {
            if pair[0] == pair[1] {
                run += 1;
            } else {
                held.push(run);
                run = 1;
            }
        }
        let held = &held[1..];
        assert!(
            held.iter().all(|n| (2..=3).contains(n)),
            "a frame was held for something other than two or three refreshes: {held:?}"
        );
        assert!(held.contains(&2) && held.contains(&3), "{held:?}");
    }

    /// Snapping is to the nearest refresh, and never runs backwards.
    #[test]
    fn the_grid_rounds_to_the_nearest_refresh() {
        let interval = Duration::from_millis(10);
        assert_eq!(snap(Duration::from_millis(0), interval), Duration::ZERO);
        assert_eq!(snap(Duration::from_millis(4), interval), Duration::ZERO);
        assert_eq!(
            snap(Duration::from_millis(6), interval),
            Duration::from_millis(10)
        );
        assert_eq!(
            snap(Duration::from_millis(94), interval),
            Duration::from_millis(90)
        );
    }

    /// The control bar covers the picture's full width and the same fraction of
    /// its height whichever art set is loaded, because every set is the same
    /// 800x75 layout scaled. Getting this from the stage's scale instead —
    /// 1280 wide art multiplied by a window-over-452 factor — is what drew the
    /// full-screen bar off the side of the window.
    #[test]
    fn the_bar_covers_the_picture_whichever_art_set_is_loaded() {
        let dst = FRect::new(0.0, -2.0, 1920.0, 1084.0);
        let windowed = bar_strip(dst, (800, 75));
        let full = bar_strip(dst, (1280, 120));
        assert_eq!(windowed.w, dst.w);
        assert_eq!(full.w, dst.w);
        assert_eq!(windowed.h, full.h);
        assert!((full.h - dst.w * 75.0 / 800.0).abs() < 0.01, "{full:?}");
        assert_eq!((windowed.x, windowed.y), (dst.x, dst.y));
    }

    /// A screen whose hit map failed to load leaves the strip zero-sized, and
    /// that must not divide by zero on the way to a rectangle.
    #[test]
    fn a_zero_sized_strip_still_gives_a_rectangle() {
        let dst = bar_strip(FRect::new(4.0, 8.0, 100.0, 50.0), (0, 10));
        assert_eq!(dst.w, 100.0);
        assert!(dst.h.is_finite(), "{dst:?}");
    }

    /// The dialog is magnified with the screen under it, and a click has to
    /// follow it there: the composite is the window's size, so the dialog is
    /// drawn at that same magnification and a window pixel divides back by it
    /// to reach the dialog's own space. Drawing and hit testing read the one
    /// `Placed` the drawing pass wrote, which is what keeps them together.
    #[test]
    fn a_click_lands_where_the_magnified_dialog_was_drawn() {
        // A 200x100 dialog at 3x, centred in a 1920x1080 composite shown
        // one-to-one in the window.
        let placed = Placed {
            at: (660, 390),
            size: (600, 300),
            scale: 3.0,
        };
        let dst = FRect::new(0.0, 0.0, 1920.0, 1080.0);
        let size = (1920, 1080);
        // The dialog's top-left three window pixels in, which is its own
        // pixel (1, 1) at this magnification.
        assert_eq!(in_dialog(dst, size, placed, 663.0, 393.0), Some((1, 1)));
        // The dialog's own centre, whatever it was magnified by.
        assert_eq!(in_dialog(dst, size, placed, 960.0, 540.0), Some((100, 50)));
        // Just outside each edge.
        assert_eq!(in_dialog(dst, size, placed, 659.0, 540.0), None);
        assert_eq!(in_dialog(dst, size, placed, 1260.0, 540.0), None);
        assert_eq!(in_dialog(dst, size, placed, 960.0, 389.0), None);
        assert_eq!(in_dialog(dst, size, placed, 960.0, 690.0), None);
    }

    /// The same click through a letterboxed window: the composite is scaled
    /// into `dst` first, and only then measured against where the dialog sits
    /// inside it.
    #[test]
    fn a_letterboxed_window_still_finds_the_dialog() {
        let placed = Placed {
            at: (660, 390),
            size: (600, 300),
            scale: 3.0,
        };
        // Half size, offset — the composite's centre is the rectangle's.
        let dst = FRect::new(100.0, 60.0, 960.0, 540.0);
        assert_eq!(
            in_dialog(dst, (1920, 1080), placed, 100.0 + 480.0, 60.0 + 270.0),
            Some((100, 50))
        );
    }

    /// The mode the player chose goes into the settings in the spelling the
    /// game's own reader expects, so that `Config.DAT` still means the same
    /// thing to `SCHOOLDAYS HQ.exe`: `FUN_0040cbb0` takes `DisplayType` 0 as
    /// 4:3, and `FUN_0040db00` takes `WindowMode` 1 as full screen.
    #[test]
    fn the_display_mode_is_written_the_way_the_game_reads_it() {
        let mut config = Config::default();
        for (wide, full, display_type, window_mode) in [
            (false, false, "0", "0"),
            (true, false, "1", "0"),
            (false, true, "0", "1"),
            (true, true, "1", "1"),
        ] {
            remember_display(
                &mut config,
                Display {
                    wide,
                    full_screen: full,
                },
            );
            assert_eq!(config.get("DisplayType"), Some(display_type));
            assert_eq!(config.get("WindowMode"), Some(window_mode));
        }
    }

    /// Writing the mode marks the settings dirty, because that is what the
    /// Option screen's close button flushes — a display change that left the
    /// config clean was written nowhere and lost on exit.
    #[test]
    fn changing_the_display_mode_asks_for_a_flush() {
        let mut config = Config::parse_text("[WindowMode]=\"0\"\n");
        assert!(!config.dirty(), "a freshly read file is not dirty");
        remember_display(
            &mut config,
            Display {
                wide: true,
                full_screen: true,
            },
        );
        assert!(config.dirty());
    }
}
