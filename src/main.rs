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
use daysengine::install::save::FlagStore;
use daysengine::install::vfs::Vfs;
use daysengine::media::AudioBuffer;
use daysengine::ui::bar::{self, Bar};
use daysengine::ui::ending;
use daysengine::ui::menu::{Action, Menu, Mode, SaveState, Session, SystemSe};
use daysengine::ui::options::{Dir, Display, Som};
use daysengine::ui::replay::Scenes;
use daysengine::ui::screen::Resolution;
use daysengine::ui::select::{self, Choice, Input, Select};
use daysengine::{install::ini::Ini, playback::text, Mixer, Stage};
use sdl3::audio::{AudioCallback, AudioFormat, AudioSpec, AudioStream};
use sdl3::event::Event;
use sdl3::keyboard::Keycode;
use sdl3::mouse::MouseButton;
use sdl3::pixels::{Color, PixelFormat};
use sdl3::render::{BlendMode, Canvas, FRect, Texture, TextureCreator};
use sdl3::video::{Window, WindowContext};
use sdl3::EventPump;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

/// Presentation size. Movies decode at 800x452 and still backgrounds are
/// 800x450; everything is drawn into this box and the box is letterboxed into
/// the window, so the two never disagree about aspect.
const STAGE_WIDTH: u32 = 800;
const STAGE_HEIGHT: u32 = 452;

/// The UI is authored at 800x450 and the shipped `.CMAP`s carry that same
/// layout pre-scaled to each of the four sizes. Presenting at the native one
/// keeps the menu in the same box as the stage and leaves the scaling to the
/// one letterbox both share.
const MENU_RESOLUTION: Resolution = Resolution::Wide;

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

    let mut player = Player {
        game: game.clone(),
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

    let mut next = wanted.clone();
    loop {
        if menus {
            match run_menu(&mut player, &mut canvas, &creator, &mut events, &start)? {
                Outcome::Quit => break,
                Outcome::Play => next = wanted.clone(),
                // A replay names its own script, which the DLL's table spells
                // as a path; `find_script` wants the trailing name.
                Outcome::Replay(script) => {
                    next = script.rsplit('/').next().unwrap_or(&script).to_string();
                    log::info!("replaying {next}");
                }
            }
        }
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
        let outcome = run_script(&mut player, &mut canvas, &creator, &mut events, script)?;
        canvas.window_mut().set_title("DaysEngine")?;
        // A script that ran out returns to the title, as the game does. Quitting
        // out of one ends the session either way.
        if outcome == Outcome::Quit || !menus {
            break;
        }
    }

    Ok(())
}

/// Why a loop gave up control.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Outcome {
    /// Start playing the script.
    Play,
    /// Play one replay scene's script, chosen on the replay screen.
    Replay(String),
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
fn build_session(player: &Player, start: &Ini) -> Session {
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
        // This engine draws its menus at one fixed size, so the two questions
        // the Def tab asks have one answer. `MENU_RESOLUTION` is widescreen and
        // the window is not full screen.
        display: Display {
            wide: true,
            full_screen: false,
        },
        som: Som::default(),
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
) -> Result<Outcome> {
    let session = build_session(player, start);
    apply_settings(&session, player.mixer);
    let mut menu = Menu::open(
        player.vfs,
        &player.dll,
        Mode::TITLE,
        session,
        MENU_RESOLUTION,
    )
    .context("opening the title screen")?;

    let backdrop = load_title_backdrop(player, start);

    play_menu_bgm(player, start.get("TitleBGM"));

    let mut texture: Option<Texture> = None;
    let mut size = (0, 0);
    loop {
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
                Action::Quit => return Ok(Outcome::Quit),
                // Each change is already in the settings; this is where the
                // engine picks the new volumes up.
                Action::SettingsChanged => {
                    apply_settings(menu.session(), player.mixer);
                    texture = None;
                }
                // The Option screen's close button. The original flushes here
                // and tells the host to leave the menus with a mode whose
                // meaning is not recovered, so this returns to the title.
                Action::SettingsSaved => {
                    apply_settings(menu.session(), player.mixer);
                    if menu.session().config.dirty() {
                        let mut config = menu.session().config.clone();
                        if let Err(err) = config.save(&player.game) {
                            log::warn!("could not write the settings: {err}");
                        }
                        menu.session_mut().config = config;
                    }
                    menu.advance(player.vfs, &player.dll, Mode::TITLE)?;
                    texture = None;
                }
                // The DLL only records the request and this engine draws its
                // menus at one size, so the request is logged rather than
                // silently dropped. See `daysengine::ui::options::DisplayRequest`.
                Action::Display(request) => {
                    log::info!("the Option screen asked for {request:?}; not applied");
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

        if menu.dirty() || texture.is_none() {
            // The backdrop is only the title's; every other screen draws its own
            // background or sits over black.
            let under = (menu.mode() == Mode::TITLE)
                .then_some(backdrop.as_ref())
                .flatten();
            let image = menu.compose(under);
            size = (image.width, image.height);
            let mut new = creator.create_texture_streaming(
                PixelFormat::try_from(sdl3::sys::pixels::SDL_PIXELFORMAT_RGBA32)?,
                image.width,
                image.height,
            )?;
            new.update(None, &image.rgba, image.width as usize * 4)?;
            texture = Some(new);
        }

        canvas.set_draw_color(Color::BLACK);
        canvas.clear();
        if let Some(texture) = &texture {
            canvas
                .copy(texture, None, letterbox(canvas, size.0, size.1))
                .map_err(|e| anyhow::anyhow!("drawing the menu: {e}"))?;
        }
        canvas.present();
        std::thread::sleep(std::time::Duration::from_millis(8));
    }
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
) -> Result<Outcome> {
    player.mixer.stop_all();

    let mut movie_texture = creator
        .create_texture_streaming(
            PixelFormat::try_from(sdl3::sys::pixels::SDL_PIXELFORMAT_RGBA32)?,
            STAGE_WIDTH,
            STAGE_HEIGHT,
        )
        .context("creating movie texture")?;
    let mut still_texture: Option<(String, Texture)> = None;
    let mut text_texture: Option<(String, u32, u32, Texture)> = None;
    // One patch texture, rebuilt only when a mouth of a different size shows
    // up. Mouths are tens of pixels across and change three times a second, so
    // allocating per frame would be pure churn.
    let mut mouth_texture: Option<(usize, usize, Texture)> = None;

    let mut stage = Stage::new(script);
    let config = Config::load(&player.game);
    // Male voice lines are dropped when the player has turned `MenVoice` off.
    stage.set_men_voice(config.flag(Flag::MenVoice));

    // The control bar and the choice box both come out of the install, and a
    // missing one has to leave playback alone: this is the UI over a movie, not
    // the movie. So both are loaded with a warning and the loop checks for them.
    let mut control = match Bar::load(player.vfs, &player.dll, MENU_RESOLUTION) {
        Ok(bar) => Some(bar),
        Err(err) => {
            log::warn!("the control bar is unavailable: {err}");
            None
        }
    };
    let mut bar_state = bar::State::from_config(&config);
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

    loop {
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
                        offset = clock(origin, offset);
                    }
                    paused = !paused;
                }
                Event::KeyDown {
                    keycode: Some(key @ (Keycode::Right | Keycode::Left)),
                    ..
                } => {
                    let now = clock(origin, offset);
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
            clock(origin, offset)
        };
        if stage.finished(at) {
            log::info!("script finished");
            return Ok(Outcome::Play);
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
        bar_state.rate = bar::SPEEDS[bar_state.speed.min(bar::SPEEDS.len() - 1)];
        if let Some(control) = &mut control {
            let (bw, bh) = control.screen().size();
            let strip = FRect::new(dst.x, dst.y, bw as f32 * scale, bh as f32 * scale);
            let over = |p: (f32, f32)| {
                let sx = (p.0 - strip.x) / scale;
                let sy = (p.1 - strip.y) / scale;
                (sx >= 0.0 && sy >= 0.0 && sx < bw as f32 && sy < bh as f32)
                    .then_some((sx as u32, sy as u32))
            };
            hovered = over(pointer).and_then(|(x, y)| control.hit(x, y));

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
                                offset = clock(origin, offset);
                            }
                            paused = !paused;
                        }
                        // Host `+0x8c` sets a member and the bar redraws from
                        // it, which is what happens here. **It does not yet
                        // make playback faster**: the clock is wall-clock and
                        // the decoders run at their own rate, so scaling only
                        // the timeline would run it ahead of the audio. The
                        // rate is carried, the fast-forward is not implemented.
                        bar::Act::Speed(index) => bar_state.speed = index,
                        bar::Act::Seek(code) if code == bar::Seek::RESTART => {
                            offset = Frame::ZERO;
                            origin = Instant::now();
                        }
                        // Everything past a restart lands in the executable's
                        // state 4, which chains to the next script — the route
                        // system, which this engine does not have. Leaving
                        // playback is the closest honest answer, and the two
                        // menu requests have no screen to open over playback
                        // yet, so they are logged rather than half-done.
                        bar::Act::Seek(_) | bar::Act::Leave => return Ok(Outcome::Play),
                        bar::Act::Menu(request) => {
                            log::info!("the bar asked for menu {} over playback", request.0)
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
                match Select::load(player.vfs, player.film, pending.count(), MENU_RESOLUTION) {
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
                        log::info!("choice decided: {index}");
                    }
                }
                select::Event::Nothing => {}
            }
            if event != select::Event::Nothing {
                buttons.0 = false;
            }
        }

        if let Some(frame) = visual.movie {
            movie_texture
                .update(None, &frame.rgba, frame.width as usize * 4)
                .context("uploading movie frame")?;
            canvas
                .copy(&movie_texture, None, dst)
                .map_err(|e| anyhow::anyhow!("drawing movie: {e}"))?;
        } else if let Some(still) = visual.still {
            // Rebuild the texture only when the background actually changes.
            let stale = still_texture
                .as_ref()
                .is_none_or(|(path, _)| path != &still.path);
            if stale {
                let mut texture = creator.create_texture_streaming(
                    PixelFormat::try_from(sdl3::sys::pixels::SDL_PIXELFORMAT_RGBA32)?,
                    still.width,
                    still.height,
                )?;
                texture.update(None, &still.rgba, still.width as usize * 4)?;
                still_texture = Some((still.path.clone(), texture));
            }
            if let Some((_, texture)) = &still_texture {
                canvas
                    .copy(texture, None, dst)
                    .map_err(|e| anyhow::anyhow!("drawing background: {e}"))?;
            }
        }

        // Mouth patches, over the background and under the fade, matching the
        // order the engine composites them in.
        for (mouth, index) in &visual.mouths {
            let stale = mouth_texture
                .as_ref()
                .is_none_or(|(w, h, _)| (*w, *h) != (mouth.width, mouth.height));
            if stale {
                let mut texture = creator.create_texture_streaming(
                    PixelFormat::try_from(sdl3::sys::pixels::SDL_PIXELFORMAT_RGBA32)?,
                    mouth.width as u32,
                    mouth.height as u32,
                )?;
                texture.set_blend_mode(BlendMode::Blend);
                mouth_texture = Some((mouth.width, mouth.height, texture));
            }
            let Some((_, _, texture)) = &mut mouth_texture else {
                continue;
            };
            texture.update(None, mouth.image(*index), mouth.width * 4)?;
            let patch = FRect::new(
                dst.x + mouth.x as f32 * scale,
                dst.y + mouth.y as f32 * scale,
                mouth.width as f32 * scale,
                mouth.height as f32 * scale,
            );
            canvas
                .copy(&*texture, None, patch)
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
            // Cache on the rendered string: laying out a line costs 48x48 of
            // glyph decode per character, which is wasteful at 24 fps.
            let display = if speaker.is_empty() {
                line.to_string()
            } else {
                format!("{speaker}: {line}")
            };
            let stale = text_texture
                .as_ref()
                .is_none_or(|(cached, ..)| cached != &display);
            if stale {
                let image = text::render_line(player.font, &display, [255, 255, 255]);
                let mut texture = creator.create_texture_streaming(
                    PixelFormat::try_from(sdl3::sys::pixels::SDL_PIXELFORMAT_RGBA32)?,
                    image.width as u32,
                    image.height as u32,
                )?;
                texture.set_blend_mode(BlendMode::Blend);
                texture.update(None, &image.rgba, image.width * 4)?;
                text_texture = Some((display, image.width as u32, image.height as u32, texture));
            }
            if let Some((_, w, h, texture)) = &text_texture {
                // Dialogue sits along the bottom of the stage. The original's
                // exact box is in the UI layer, which is not built yet.
                let text_scale = scale * 0.5;
                let tw = *w as f32 * text_scale;
                let th = *h as f32 * text_scale;
                canvas
                    .copy(
                        texture,
                        None,
                        FRect::new(
                            dst.x + 16.0 * scale,
                            dst.y + dst.h - th - 16.0 * scale,
                            tw,
                            th,
                        ),
                    )
                    .map_err(|e| anyhow::anyhow!("drawing text: {e}"))?;
            }
        }

        // The choice labels. The original draws these through its own text
        // pipeline from an anchor whose transform is not recovered; what *is*
        // recovered is the box each choice occupies, because that is the
        // shipped hit map. So each label is centred in its own box.
        if let Some((pending, map)) = &choice {
            if pending.visible(at) {
                if let Some(((mw, mh), boxes)) = map.map_size().zip(map.bounds()) {
                    for (index, label) in pending.labels.iter().enumerate() {
                        let Some(region) = boxes.get(index) else {
                            continue;
                        };
                        let lit = pending.highlight == Some(index);
                        let colour = if lit {
                            [255, 236, 160]
                        } else {
                            [255, 255, 255]
                        };
                        let image = text::render_line(player.font, label, colour);
                        let mut texture = creator.create_texture_streaming(
                            PixelFormat::try_from(sdl3::sys::pixels::SDL_PIXELFORMAT_RGBA32)?,
                            image.width as u32,
                            image.height as u32,
                        )?;
                        texture.set_blend_mode(BlendMode::Blend);
                        texture.update(None, &image.rgba, image.width * 4)?;
                        let text_scale = scale * 0.5;
                        let tw = image.width as f32 * text_scale;
                        let th = image.height as f32 * text_scale;
                        let cx =
                            (f32::from(region.x as u16) + region.width as f32 / 2.0) / mw as f32;
                        let cy =
                            (f32::from(region.y as u16) + region.height as f32 / 2.0) / mh as f32;
                        canvas
                            .copy(
                                &texture,
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

        // The control bar last, over everything, as its own layer.
        if let Some(control) = &control {
            let elapsed = auto_since.elapsed().as_millis().min(u128::from(u32::MAX)) as u32;
            let records = control.records(hovered, bar_state, elapsed);
            let stale = bar_texture
                .as_ref()
                .is_none_or(|(cached, ..)| cached != &records);
            if stale {
                let image = control.compose(None, &control.states(hovered, bar_state, elapsed));
                let mut texture = creator.create_texture_streaming(
                    PixelFormat::try_from(sdl3::sys::pixels::SDL_PIXELFORMAT_RGBA32)?,
                    image.width,
                    image.height,
                )?;
                texture.set_blend_mode(BlendMode::Blend);
                texture.update(None, &image.rgba, image.width as usize * 4)?;
                bar_texture = Some((records, image.width, image.height, texture));
            }
            if let Some((_, w, h, texture)) = &bar_texture {
                canvas
                    .copy(
                        texture,
                        None,
                        FRect::new(dst.x, dst.y, *w as f32 * scale, *h as f32 * scale),
                    )
                    .map_err(|e| anyhow::anyhow!("drawing the control bar: {e}"))?;
            }
        }

        canvas.present();
        // The script clock is the authority; sleeping a little keeps us from
        // spinning a core between frames.
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
}

/// Current script frame from wall-clock elapsed time plus the seek offset.
fn clock(origin: Instant, offset: Frame) -> Frame {
    Frame(offset.0 + Frame::from_duration(origin.elapsed()).0)
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
