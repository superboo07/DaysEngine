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
use daysengine::install::config::{Channel, Config, Flag};
use daysengine::install::progress::Progress;
use daysengine::install::save::FlagStore;
use daysengine::install::vfs::Vfs;
use daysengine::media::AudioBuffer;
use daysengine::ui::bar::{self, Bar};
use daysengine::ui::comment;
use daysengine::ui::ending;
use daysengine::ui::menu::{Action, Menu, Mode, SaveState, Session, SystemSe};
use daysengine::ui::options::{self, Dir, Display, Som};
use daysengine::ui::replay::Scenes;
use daysengine::ui::saveload::{self, Slots};
use daysengine::ui::screen::Resolution;
use daysengine::ui::select::{self, Choice, Input, Select};
use daysengine::{install::ini::Ini, playback::scale, playback::text, Mixer, Stage};
use sdl3::audio::{AudioCallback, AudioFormat, AudioSpec, AudioStream};
use sdl3::event::Event;
use sdl3::keyboard::Keycode;
use sdl3::mouse::MouseButton;
use sdl3::pixels::{Color, PixelFormat};
use sdl3::render::{BlendMode, Canvas, FRect, ScaleMode, Texture, TextureCreator};
use sdl3::video::{Window, WindowContext};
use sdl3::EventPump;
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

    /// Plays one, on the mixer slot the menus own.
    ///
    /// Scripts address SE slots 1..=5 and the menus are never up while a script
    /// is running, so borrowing the last slot costs nothing.
    fn play(&self, se: SystemSe, vfs: &Vfs, sounds: &mut Sounds, mixer: &Mixer) {
        let Some(path) = self.paths.get(se.key()) else {
            return;
        };
        if let Some(buffer) = sounds.get(vfs, path) {
            mixer.play_se(5, buffer);
        }
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
    highlight: Option<usize>,
    drawn: Vec<(u32, u32, Texture<'r>)>,
}

/// One mouth patch's texture, and which patch and image it holds.
///
/// A mouth flaps at its voice's envelope, a few times a second; without this
/// the patch went up to the GPU on every pass round the loop instead.
struct MouthPatch<'r> {
    /// Patch rectangle and which of the three images — the whole of what
    /// decides the pixels.
    key: (usize, usize, usize, usize, usize),
    size: (usize, usize),
    texture: Texture<'r>,
}

/// Everything the two loops both need.
struct Player<'a> {
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
}

impl Player<'_> {
    /// The art set to load for the mode in force. See
    /// [`Resolution::for_display`].
    fn resolution(&self) -> Resolution {
        Resolution::for_display(self.display.wide, self.display.full_screen, self.mini_note)
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
    // [`present_interval`] paces the loop for the case where it is refused.
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
    // three keys the original reads: `DisplayType` is the aspect, `WindowMode`
    // is windowed versus full screen, and `TypeMiniNote` picks the 1024x576 art
    // over the 1280x720 art. The engine starts windowed whatever `WindowMode`
    // says, because a window is the safe thing to open on somebody else's
    // desktop; the Option screen is one click away.
    let boot_config = Config::load(&game);
    let mut player = Player {
        game: game.clone(),
        display: Display {
            wide: boot_config
                .get("DisplayType")
                .is_none_or(|v| v.trim() != "0"),
            full_screen: false,
        },
        mini_note: boot_config
            .get("TypeMiniNote")
            .is_some_and(|v| v.trim() != "0"),
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
    let mut replaying: Option<(Vec<String>, usize)> = None;
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
                Outcome::Play | Outcome::Finished => next = wanted.clone(),
                // A replay names its own script, which the DLL's table spells
                // as a path; `find_script` wants the trailing name.
                // A scene is a sequence, not one script. `FUN_1001f270` starts
                // it at step 0 and `FUN_1001f0d0` is asked for the next one
                // each time a script ends, so the whole list is carried and
                // walked here rather than the first of it being played alone.
                Outcome::Replay(scripts) => {
                    let Some(first) = scripts.first() else {
                        continue;
                    };
                    next = first.rsplit('/').next().unwrap_or(first).to_string();
                    log::info!("replaying {next}, {} script(s) in the scene", scripts.len());
                    replaying = Some((scripts, 0));
                }
                // Loading from the title puts the player wherever the slot
                // says, so the position comes from the slot and not from a
                // fresh `searchRoot` on the name.
                Outcome::LoadSlot(slot) => match progress
                    .as_mut()
                    .and_then(|p| p.load_from(&game, &film, slot))
                {
                    Some(script) => {
                        next = script.rsplit('/').next().unwrap_or(&script).to_string();
                        chained = true;
                        continue;
                    }
                    None => {
                        log::warn!("slot {slot} would not load");
                        continue;
                    }
                },
                // There is nothing to save from the title — the player has no
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
        // A script that reached its end hands over to the branch graph, which
        // is what the executable's state 4 does. When the graph names nothing
        // — the route ended, or the script was not in it — the session goes
        // back to the title, as the game does. Quitting ends it either way.
        if outcome == Outcome::Finished {
            // A replay walks the scene's own list. `FUN_1001ee20`'s `default`
            // arm is `step + 1`, which is every scene without a branch table —
            // see `daysengine::ui::replay` for which twelve have one and what
            // is still missing to follow them.
            if let Some((scripts, step)) = &mut replaying {
                *step += 1;
                if let Some(script) = scripts.get(*step) {
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
                continue;
            }
        }
        // Anything else leaving playback ends the replay too.
        if outcome != Outcome::Finished {
            replaying = None;
        }
        if outcome == Outcome::Quit || !menus {
            break;
        }
    }

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

/// Why a loop gave up control.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Outcome {
    /// Start playing the script.
    Play,
    /// The script reached its end, so the branch graph decides what follows.
    Finished,
    /// Play a replay scene: the sequence of scripts it runs through.
    Replay(Vec<String>),
    /// Load a save slot and play what it names.
    LoadSlot(u32),
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

/// Runs the menus until they start a script or the game is closed.
///
/// The menu is a still image that only changes when the selection does, so this
/// recomposites on demand rather than per frame: a frame is a 800x450 software
/// composite and there is nothing animating between clicks.
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
fn build_session(player: &Player, start: &Ini, english: bool) -> Session {
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
        config: Config::load(&player.game),
        scenes,
        // The Def tab shows which value is in force and greys the other, so
        // this has to be the engine's real mode rather than a fixed answer.
        // The original asks the same two questions through host `+0xb8` and
        // `+0xbc`.
        display: player.display,
        som: Som::default(),
        english,
        text_input: player.film.get_bool("TextInput").unwrap_or(false),
        // The screen reports a slot as present when its file opens, and takes
        // the line it shows from the global store.
        slots: Slots::read(&player.game, player.film, &player.flags, english),
    }
}

/// Pushes the settings' volumes into the mixer.
///
/// The original hands a decibel figure per channel to DirectSound; this engine
/// has one master gain, so until the mixer grows per-channel gain the quietest
/// of the three is what it can honestly apply. Muting is exact either way.
fn apply_settings(session: &Session, mixer: &Mixer) {
    let gain = if session.config.flag(Flag::Mute) {
        0.0
    } else {
        Channel::ALL
            .iter()
            .map(|c| session.config.gain(*c))
            .fold(f32::INFINITY, f32::min)
    };
    mixer.set_master_volume(gain);
}

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
    let session = build_session(player, start, english);
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

    play_menu_bgm(player, start.get("TitleBGM"));

    let mut texture: Option<Texture> = None;
    // Cached resampling weights, rebuilt when the window changes size.
    let mut scaler = scale::Scaler::default();
    // The slot the player picked on the save screen, waiting for the
    // comment dialog to confirm or abandon it.
    let mut naming: Option<u32> = None;
    let mut size = (0, 0);
    // Paced to the display, not to how fast the loop can spin.
    let mut interval = present_interval(canvas);
    loop {
        let pass = Instant::now();
        for event in events.poll_iter() {
            let action = match event {
                Event::Quit { .. } => return Ok(Outcome::Quit),
                Event::KeyDown {
                    keycode: Some(Keycode::Escape),
                    ..
                } => menu.cancel(player.vfs, &player.dll)?,
                Event::KeyDown {
                    keycode: Some(Keycode::Up),
                    ..
                } => menu.navigate(Dir::Up),
                Event::KeyDown {
                    keycode: Some(Keycode::Down),
                    ..
                } => menu.navigate(Dir::Down),
                Event::KeyDown {
                    keycode: Some(Keycode::Left),
                    ..
                } => menu.navigate(Dir::Left),
                Event::KeyDown {
                    keycode: Some(Keycode::Right),
                    ..
                } => menu.navigate(Dir::Right),
                Event::KeyDown {
                    keycode: Some(Keycode::Return | Keycode::KpEnter | Keycode::Space),
                    ..
                } => confirm(&mut menu, player)?,
                Event::MouseMotion { x, y, .. } => match to_screen(canvas, &menu, x, y) {
                    Some((sx, sy)) => menu.point_at(sx, sy),
                    None => menu.point_away(),
                },
                Event::MouseButtonDown {
                    mouse_btn: MouseButton::Left,
                    x,
                    y,
                    ..
                } => match to_screen(canvas, &menu, x, y) {
                    Some((sx, sy)) => {
                        // Clicking is pointing and then confirming: the original
                        // acts on whatever the cursor is over, not on whatever
                        // the keyboard last selected.
                        menu.point_at(sx, sy);
                        confirm(&mut menu, player)?
                    }
                    None => Action::Stay,
                },
                Event::MouseButtonDown {
                    mouse_btn: MouseButton::Right,
                    ..
                } => menu.cancel(player.vfs, &player.dll)?,
                _ => Action::Stay,
            };

            match action {
                Action::Play => return Ok(Outcome::Play),
                Action::PlayReplay(script) => return Ok(Outcome::Replay(script)),
                Action::Load(slot) => return Ok(Outcome::LoadSlot(slot)),
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
                // Still to build: the Option screen really does ask for a
                // window size and a full-screen toggle, and the player should
                // get them. The DLL only records the request — who acts on it
                // is **not recovered** — so what the engine does with it is the
                // engine's to decide, and right now it decides nothing. Logged
                // so the gap is visible rather than silent. See
                // `daysengine::ui::options::DisplayRequest`.
                // `FUN_004279e0` and `FUN_00427a90` are the original's two
                // appliers: each polls the flag the Def tab raised, changes the
                // mode once and clears it. This does the same on arrival, which
                // from the player's side is the same thing.
                Action::Display(request) => {
                    apply_display(player, canvas, request)?;
                    // Every screen's art is chosen by the mode, so whatever is
                    // showing has to be reloaded at the new size.
                    menu.set_resolution(player.vfs, &player.dll, player.resolution())?;
                    menu.session_mut().display = player.display;
                    under = backdrop.as_ref().map(|b| menu.screen().to_display(b));
                    // Full screen can land the window on a panel that refreshes
                    // at another rate.
                    interval = present_interval(canvas);
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

        // The screen's own size, and the rectangle it lands in. The art is
        // resampled to that rectangle rather than stretched onto it by the
        // driver — see `daysengine::playback::scale`.
        let (screen_w, screen_h) = menu.screen().size();
        let dst = letterbox(canvas, screen_w, screen_h);
        let at = (dst.w.round().max(1.0) as u32, dst.h.round().max(1.0) as u32);
        if menu.dirty() || texture.is_none() || at != size {
            // The backdrop is only the title's; every other screen draws its own
            // background or sits over black.
            let image = menu.compose(
                (menu.mode() == Mode::TITLE)
                    .then_some(under.as_ref())
                    .flatten(),
            );
            let src = (image.width as usize, image.height as usize);
            let want = (at.0 as usize, at.1 as usize);
            let (w, h, rgba) = match scaler.resample(&image.rgba, src, want) {
                Some(scaled) => (at.0, at.1, scaled),
                None => (image.width, image.height, image.rgba.as_slice()),
            };
            size = at;
            let mut new = new_texture(creator, w, h)?;
            new.update(None, rgba, w as usize * 4)?;
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
        pace(pass, interval);
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
    let mut dirty = true;

    // Paced to the display, not to how fast the loop can spin.
    let interval = present_interval(canvas);
    loop {
        let pass = Instant::now();
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
                    match to_dialog(canvas, size, dialog, base, x, y) {
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
            let over = dialog.compose_image(player.font, base);
            // `WM_INITDIALOG` centres the dialog on the game window, so this
            // does too.
            let (w, h) = (over.width, over.height);
            let at = (
                (image.width as i64 - w as i64) / 2,
                (image.height as i64 - h as i64) / 2,
            );
            image.blit_scaled(&over, (0, 0, w, h), (at.0, at.1, w, h));
            size = (image.width, image.height);
            let mut new = new_texture(creator, image.width, image.height)?;
            new.update(None, &image.rgba, image.width as usize * 4)?;
            texture = Some(new);
            dirty = false;
        }

        canvas.set_draw_color(Color::BLACK);
        canvas.clear();
        if let Some(texture) = &texture {
            canvas
                .copy(texture, None, letterbox(canvas, size.0, size.1))
                .map_err(|e| anyhow::anyhow!("drawing the comment dialog: {e}"))?;
        }
        canvas.present();
        pace(pass, interval);
    }
}

/// Maps a window pixel to the dialog's own space, or `None` outside it.
fn to_dialog(
    canvas: &Canvas<Window>,
    size: (u32, u32),
    dialog: &comment::Comment,
    base: (i32, i32),
    x: f32,
    y: f32,
) -> Option<(i32, i32)> {
    if size.0 == 0 || size.1 == 0 {
        return None;
    }
    let dst = letterbox(canvas, size.0, size.1);
    let sx = (x - dst.x) / dst.w * size.0 as f32;
    let sy = (y - dst.y) / dst.h * size.1 as f32;
    let (w, h) = dialog.size(base);
    let ox = (size.0 as f32 - w as f32) / 2.0;
    let oy = (size.1 as f32 - h as f32) / 2.0;
    let (dx, dy) = (sx - ox, sy - oy);
    (dx >= 0.0 && dy >= 0.0 && dx < w as f32 && dy < h as f32).then_some((dx as i32, dy as i32))
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

/// How long one pass round a drawing loop should take.
///
/// The loops have to run faster than the 24 fps a script is clocked at: they
/// poll the pointer, the bar ramps in over 300ms and the auto indicator spins.
/// They do not have to run faster than the display can show, and before this
/// they ran as fast as a 2ms sleep allowed — some hundreds of presents a second
/// on a panel doing 60. Vsync should already be blocking in `present`; this is
/// what paces the loop when the driver refused the hint, and it is why the
/// sleep is measured from the top of the pass rather than added to the end.
///
/// A display that will not say what it does, or claims something absurd, gets
/// 60: wrong and smooth beats wrong and spinning.
fn present_interval(canvas: &Canvas<Window>) -> Duration {
    let hz = canvas
        .window()
        .get_display()
        .and_then(|display| display.get_mode())
        .map(|mode| mode.refresh_rate)
        .unwrap_or(0.0);
    let hz = if hz.is_finite() && (20.0..=1000.0).contains(&hz) {
        hz
    } else {
        60.0
    };
    Duration::from_secs_f32(1.0 / hz)
}

/// Sleeps out the rest of a pass that began at `started`.
///
/// Measured from the top of the pass, not added to the end, so a pass that took
/// most of its budget — a 4K frame being resampled — does not then sleep a
/// whole interval on top and halve the rate.
fn pace(started: Instant, interval: Duration) {
    if let Some(left) = interval.checked_sub(started.elapsed()) {
        std::thread::sleep(left);
    }
}

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
/// is filtered on its way onto the screen. Saying so here rather than leaving
/// it to SDL's default is the difference between a recovered choice and an
/// inherited one.
fn new_texture<'a>(
    creator: &'a TextureCreator<WindowContext>,
    width: u32,
    height: u32,
) -> Result<Texture<'a>> {
    let mut texture = creator.create_texture_streaming(
        PixelFormat::try_from(sdl3::sys::pixels::SDL_PIXELFORMAT_RGBA32)?,
        width,
        height,
    )?;
    texture.set_scale_mode(ScaleMode::Linear);
    Ok(texture)
}

/// The rectangle a `width` x `height` image is drawn into, centred and scaled
/// to fit the window without distorting it.
fn letterbox(canvas: &Canvas<Window>, width: u32, height: u32) -> FRect {
    let (win_w, win_h) = canvas.output_size().unwrap_or((width, height));
    let scale = (win_w as f32 / width as f32).min(win_h as f32 / height as f32);
    let draw_w = width as f32 * scale;
    let draw_h = height as f32 * scale;
    FRect::new(
        (win_w as f32 - draw_w) / 2.0,
        (win_h as f32 - draw_h) / 2.0,
        draw_w,
        draw_h,
    )
}

/// Maps a window pixel to the menu screen's own coordinate space.
///
/// Returns `None` outside the letterboxed image, where there is nothing to hit.
fn to_screen(canvas: &Canvas<Window>, menu: &Menu, x: f32, y: f32) -> Option<(u32, u32)> {
    let (w, h) = menu.screen().size();
    let dst = letterbox(canvas, w, h);
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

    let mut movie_texture =
        new_texture(creator, STAGE_WIDTH, STAGE_HEIGHT).context("creating movie texture")?;
    let mut still_texture: Option<(String, (u32, u32), Texture)> = None;
    // The movie frame resampled to the window, and the weights that did it.
    let mut movie_scaled: Option<(u32, u32, Texture)> = None;
    // Which picture the movie textures are holding: clip, frame index, the
    // window size it was scaled for, and whether it went into `movie_scaled`
    // or straight into `movie_texture`.
    let mut movie_shown: Option<(String, u64, (u32, u32), bool)> = None;
    let mut scaler = scale::Scaler::default();
    // The wrapped lines and a texture each, cached on the script's own line.
    let mut text_texture: Option<DialogueBlock<'_>> = None;
    // One patch per mouth on screen at once, which is all but always exactly
    // one. Mouths are tens of pixels across and change three times a second, so
    // rebuilding the texture per pass would be pure churn.
    let mut mouth_patches: Vec<MouthPatch<'_>> = Vec::new();
    let mut choice_labels: Option<ChoiceLabels<'_>> = None;

    let mut stage = Stage::new(script);
    let config = Config::load(&player.game);
    // `[UseEnglish]` decides the dialogue pitch and whether it wraps at all.
    let english = player.film.get_bool("UseEnglish").unwrap_or(false);
    // `[LeftArrangement]` picks per-line centring or a left-aligned block.
    let left_arrangement = player.film.get_bool("LeftArrangement").unwrap_or(false);
    // Male voice lines are dropped when the player has turned `MenVoice` off.
    stage.set_men_voice(config.flag(Flag::MenVoice));

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
    let mut bar_texture: Option<(Vec<usize>, u32, u32, Texture)> = None;
    let mut choice: Option<(Choice, Select)> = None;

    // The clock is wall-clock based with an offset, so pausing and seeking are
    // both just adjustments to the offset rather than separate state machines.
    let mut origin = Instant::now();
    let mut offset = Frame::ZERO;
    let mut paused = false;
    let mut pointer = (0.0f32, 0.0f32);
    let mut buttons = (false, false);
    // A wall clock for the bar's fade, which ramps in milliseconds of real time
    // and so cannot hang off the script clock: the script clock stops when
    // playback is paused, and the bar still has to fade.
    let start = Instant::now();

    // Paced to the display, not to how fast the loop can spin.
    let mut interval = present_interval(canvas);
    loop {
        let pass = Instant::now();
        // The rate the clock runs at, from the speed widget that is lit. Read
        // once per frame because every use of the clock in this iteration has
        // to agree about it.
        let rate = bar::SPEEDS[bar_state.speed.min(bar::SPEEDS.len() - 1)];

        for event in events.poll_iter() {
            match event {
                Event::Quit { .. }
                | Event::KeyDown {
                    keycode: Some(Keycode::Escape),
                    ..
                } => return Ok(Outcome::Quit),
                Event::KeyDown {
                    keycode: Some(Keycode::Space),
                    ..
                } => {
                    if paused {
                        origin = Instant::now();
                    } else {
                        offset = clock(origin, offset, rate);
                    }
                    paused = !paused;
                }
                Event::KeyDown {
                    keycode: Some(key @ (Keycode::Right | Keycode::Left)),
                    ..
                } => {
                    let now = clock(origin, offset, rate);
                    let delta = 5 * FPS;
                    offset = if key == Keycode::Right {
                        Frame(now.0 + delta)
                    } else {
                        Frame(now.0.saturating_sub(delta))
                    };
                    origin = Instant::now();
                }
                Event::KeyDown {
                    keycode: Some(Keycode::R),
                    ..
                } => {
                    offset = Frame::ZERO;
                    origin = Instant::now();
                }
                Event::MouseMotion { x, y, .. } => pointer = (x, y),
                Event::MouseButtonDown {
                    mouse_btn, x, y, ..
                } => {
                    pointer = (x, y);
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
        }

        let at = if paused {
            offset
        } else {
            clock(origin, offset, rate)
        };
        if stage.finished(at) {
            log::info!("script finished");
            return Ok(Outcome::Finished);
        }

        stage.seek_to(at, player.vfs, player.mixer)?;
        let visual = stage.visual_at(at);

        canvas.set_draw_color(Color::BLACK);
        canvas.clear();

        let dst = letterbox(canvas, STAGE_WIDTH, STAGE_HEIGHT);
        let scale = dst.h / STAGE_HEIGHT as f32;

        // The control bar's own space is 800x75 with its origin at the strip's
        // top-left corner; the engine places that strip, and the DLL's
        // `FUN_10021c20` gives the base sprite the same half-pixel inset every
        // other sprite gets, so the origin is the only placement there is.
        bar_state.paused = paused;
        // The gauge draws the two counters the branch system keeps, and shows
        // over a faded bar only while a delta has raised it.
        if let Some(p) = progress.as_deref() {
            let (values, raised) = p.gauge();
            bar_state.gauge = Some(values);
            bar_state.gauge_raised = raised;
        }
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
            let sx = (pointer.0 - strip.x) / bar_scale;
            let sy = (pointer.1 - strip.y) / bar_scale;
            let over = (sx >= 0.0 && sy >= 0.0 && sx < bw as f32 && sy < bh as f32)
                .then_some((sx as u32, sy as u32));
            let now_ms = start.elapsed().as_millis().min(u128::from(u32::MAX)) as u32;
            hovered = control.point_at(over, now_ms);

            // A click only reaches the bar while the bar is actually on screen.
            if buttons.0 && !control.fade().drawn() {
                buttons.0 = false;
            }
            if buttons.0 {
                if let Some(widget) = hovered {
                    // Consume the press, so holding the button does not
                    // re-dispatch every frame.
                    buttons.0 = false;
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
                        bar::Act::TogglePause => {
                            if paused {
                                origin = Instant::now();
                            } else {
                                offset = clock(origin, offset, rate);
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
                                offset = clock(origin, offset, rate);
                                origin = Instant::now();
                                player.mixer.set_rate(bar_state.rate);
                            }
                        }
                        bar::Act::Seek(code) if code == bar::Seek::RESTART => {
                            offset = Frame::ZERO;
                            origin = Instant::now();
                        }
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
                            // menus play their own BGM through the same mixer
                            // and are not on the rate-adjusted stream, so the
                            // rate comes off for the duration and goes back on
                            // return — otherwise a menu opened at 24x would be
                            // silent.
                            offset = clock(origin, offset, rate);
                            player.mixer.set_rate(1.0);
                            let outcome = run_menu(
                                player,
                                canvas,
                                creator,
                                events,
                                start_ini,
                                MenuEntry::OverPlayback(mode, kind),
                                progress.as_deref_mut(),
                            )?;
                            origin = Instant::now();
                            player.mixer.set_rate(bar_state.rate);
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
                            interval = present_interval(canvas);
                            // Every cached texture belonged to the menu's
                            // renderer; drop them so playback rebuilds.
                            movie_shown = None;
                            mouth_patches.clear();
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
                        bar::Act::Step(step) => {
                            log::info!("the bar asked for step {step}")
                        }
                        bar::Act::None => {}
                    }
                }
            }
            control.expire_latch(at.0);
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
            let input = Input {
                pointer: (
                    f64::from((pointer.0 - dst.x) / dst.w),
                    f64::from((pointer.1 - dst.y) / dst.h),
                ),
                pick: buttons.0,
                dismiss: buttons.1,
                ..Input::default()
            };
            let event = pending.tick(at, map, input, bar_state.auto, &mut |n| {
                // The original seeds from `GetTickCount` and draws once; any
                // source of the same range does the same job.
                (Instant::now().elapsed().subsec_nanos() as usize ^ at.0 as usize) % n.max(1)
            });
            match event {
                select::Event::Raised(se) | select::Event::Decided(_, se) => {
                    player
                        .system_se
                        .play(se, player.vfs, &mut player.sounds, player.mixer);
                    if let select::Event::Decided(index, _) = event {
                        // The engine credits the choice's deltas the moment
                        // the box settles, before the script has ended --
                        // `FUN_00431740` calls `_SetFeeling@8(host, 1)` right
                        // after storing the index.
                        log::info!("choice decided: {index}");
                        if let Some(p) = progress.as_deref_mut() {
                            p.decide(index);
                        }
                    }
                }
                select::Event::Nothing => {}
            }
            if event != select::Event::Nothing {
                buttons.0 = false;
            }
        }

        // The frame the window actually shows, resampled to the letterbox
        // rather than stretched onto it by the driver. See
        // `daysengine::playback::scale`.
        //
        // Resampled once per *picture*, not once per pass round this loop. The
        // loop has to spin much faster than 24 fps to stay responsive to the
        // pointer, so it sees each movie frame several times over, and scaling
        // one is the most expensive thing it does. `movie_id` is what says two
        // of those are the same picture; the window size joins it because a
        // resize changes the answer.
        let window_px = (dst.w.round().max(1.0) as u32, dst.h.round().max(1.0) as u32);
        if let Some(frame) = visual.movie {
            let showing = visual
                .movie_id
                .map(|(clip, index)| (clip, index, window_px));
            let stale = movie_shown.as_ref().is_none_or(|(clip, index, size, _)| {
                showing != Some((clip.as_str(), *index, *size))
            });
            if stale {
                let src = (frame.width as usize, frame.height as usize);
                let want = (window_px.0 as usize, window_px.1 as usize);
                let scaled = match scaler.resample(&frame.rgba, src, want) {
                    Some(scaled) => {
                        if movie_scaled
                            .as_ref()
                            .is_none_or(|(w, h, _)| (*w, *h) != window_px)
                        {
                            let texture = new_texture(creator, window_px.0, window_px.1)?;
                            movie_scaled = Some((window_px.0, window_px.1, texture));
                        }
                        if let Some((w, _, texture)) = &mut movie_scaled {
                            texture
                                .update(None, scaled, *w as usize * 4)
                                .context("uploading movie frame")?;
                        }
                        true
                    }
                    // The window is already the frame's own size, so there is
                    // nothing to scale and the frame goes up as it stands.
                    None => {
                        movie_texture
                            .update(None, &frame.rgba, frame.width as usize * 4)
                            .context("uploading movie frame")?;
                        false
                    }
                };
                movie_shown =
                    showing.map(|(clip, index, size)| (clip.to_string(), index, size, scaled));
            }
            let texture = match movie_shown {
                Some((_, _, _, true)) => movie_scaled.as_ref().map(|(_, _, t)| t),
                _ => Some(&movie_texture),
            };
            if let Some(texture) = texture {
                canvas
                    .copy(texture, None, dst)
                    .map_err(|e| anyhow::anyhow!("drawing movie: {e}"))?;
            }
        } else if let Some(still) = visual.still {
            // Rebuild the texture when the background changes or the window
            // does, since the upload is now at the window's size.
            let stale = still_texture
                .as_ref()
                .is_none_or(|(path, size, _)| path != &still.path || *size != window_px);
            if stale {
                let src = (still.width as usize, still.height as usize);
                let (w, h, rgba) = match scaler.resample(
                    &still.rgba,
                    src,
                    (window_px.0 as usize, window_px.1 as usize),
                ) {
                    Some(scaled) => (window_px.0, window_px.1, scaled),
                    None => (still.width, still.height, still.rgba.as_slice()),
                };
                let mut texture = new_texture(creator, w, h)?;
                texture.update(None, rgba, w as usize * 4)?;
                still_texture = Some((still.path.clone(), window_px, texture));
            }
            if let Some((_, _, texture)) = &still_texture {
                canvas
                    .copy(texture, None, dst)
                    .map_err(|e| anyhow::anyhow!("drawing background: {e}"))?;
            }
        }

        // Mouth patches, over the background and under the fade, matching the
        // order the engine composites them in. One texture per patch, kept
        // until the patch or the image it shows changes.
        mouth_patches.truncate(visual.mouths.len());
        for (slot, (mouth, index)) in visual.mouths.iter().enumerate() {
            let key = (mouth.x, mouth.y, mouth.width, mouth.height, *index);
            let size = (mouth.width, mouth.height);
            if mouth_patches.get(slot).is_none_or(|p| p.size != size) {
                let mut texture = new_texture(creator, mouth.width as u32, mouth.height as u32)?;
                texture.set_blend_mode(BlendMode::Blend);
                let patch = MouthPatch {
                    // Not `key`: the upload below is what makes it true.
                    key: (0, 0, 0, 0, usize::MAX),
                    size,
                    texture,
                };
                match mouth_patches.get_mut(slot) {
                    Some(slot) => *slot = patch,
                    None => mouth_patches.push(patch),
                }
            }
            let Some(patch) = mouth_patches.get_mut(slot) else {
                continue;
            };
            if patch.key != key {
                patch
                    .texture
                    .update(None, mouth.image(*index), mouth.width * 4)?;
                patch.key = key;
            }
            let at = FRect::new(
                dst.x + mouth.x as f32 * scale,
                dst.y + mouth.y as f32 * scale,
                mouth.width as f32 * scale,
                mouth.height as f32 * scale,
            );
            canvas
                .copy(&patch.texture, None, at)
                .map_err(|e| anyhow::anyhow!("drawing mouth: {e}"))?;
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
                    let mut texture =
                        new_texture(creator, image.width as u32, image.height as u32)?;
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
                    // Rebuilt when the labels change, which is once, or when the
                    // pointer moves to another of them, which lights it.
                    let stale = choice_labels.as_ref().is_none_or(|cached| {
                        cached.highlight != pending.highlight || cached.labels != pending.labels
                    });
                    if stale {
                        let mut drawn = Vec::new();
                        for (index, label) in pending.labels.iter().enumerate() {
                            let colour = if pending.highlight == Some(index) {
                                [255, 236, 160]
                            } else {
                                [255, 255, 255]
                            };
                            let image = text::render_line(player.font, label, colour, english);
                            let mut texture =
                                new_texture(creator, image.width as u32, image.height as u32)?;
                            texture.set_blend_mode(BlendMode::Blend);
                            texture.update(None, &image.rgba, image.width * 4)?;
                            drawn.push((image.width as u32, image.height as u32, texture));
                        }
                        choice_labels = Some(ChoiceLabels {
                            labels: pending.labels.clone(),
                            highlight: pending.highlight,
                            drawn,
                        });
                    }
                    if let Some(cached) = &choice_labels {
                        let text_scale = scale * 0.5;
                        for (index, (w, h, texture)) in cached.drawn.iter().enumerate() {
                            let Some(region) = boxes.get(index) else {
                                continue;
                            };
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

        // The control bar last, over everything, as its own layer — and only
        // while it is dropped down.
        if let Some(control) = control.as_ref().filter(|c| c.fade().drawn()) {
            let elapsed = auto_since.elapsed().as_millis().min(u128::from(u32::MAX)) as u32;
            let records = control.records(hovered, bar_state, elapsed);
            let stale = bar_texture
                .as_ref()
                .is_none_or(|(cached, ..)| cached != &records);
            if stale {
                let image = control.compose(&control.states(hovered, bar_state, elapsed));
                let mut texture = new_texture(creator, image.width, image.height)?;
                texture.set_blend_mode(BlendMode::Blend);
                texture.update(None, &image.rgba, image.width as usize * 4)?;
                bar_texture = Some((records, image.width, image.height, texture));
            }
            if let Some((_, w, h, texture)) = &mut bar_texture {
                // The strip is cached on its record list and the fade applied
                // as an alpha modulation, so ramping does not recomposite it
                // 60 times a second. This is also how the original fades it:
                // one ARGB set on every sprite the bar owns.
                texture.set_alpha_mod(control.fade().alpha());
                canvas
                    .copy(&*texture, None, bar_strip(dst, (*w, *h)))
                    .map_err(|e| anyhow::anyhow!("drawing the control bar: {e}"))?;
            }
        }

        canvas.present();
        // The script clock is the authority; this only keeps the loop from
        // spinning a core between the frames the display can actually show.
        pace(pass, interval);
    }
}

/// Current script frame from wall-clock elapsed time plus the seek offset.
/// The frame playback is at: the base frame plus the scaled elapsed wall time.
///
/// `FUN_00422f70` is this function. `offset` is the executable's `+0x540`, the
/// frame the clock was last re-based to; `origin` stands for the `timeGetTime`
/// value it keeps at `+0x550`; and `rate` is the float at `+0x538` that host
/// slot `+0x8c` stores out of the speed table. Folding the elapsed frames back
/// into `offset` and taking a fresh `origin` is `FUN_00424910` followed by
/// `FUN_00424a10`, which is exactly what the original does on a rate change.
fn clock(origin: Instant, offset: Frame, rate: f32) -> Frame {
    Frame(offset.0 + Frame::from_duration_at(origin.elapsed(), rate).0)
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
}
