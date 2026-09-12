//! `daysengine` — plays a School Days HQ script.
//!
//! Drop this next to `SCHOOLDAYS HQ.exe` and run it. It reads the start script
//! out of the game's own `STARTSCRIPT.INI`, so with no arguments it plays what
//! the game would play.

#![forbid(unsafe_code)]

use anyhow::{bail, Context, Result};
use days_engine::{ini::Ini, text, Mixer, Stage};
use days_font::Font;
use days_script::{Frame, Script, FPS};
use days_vfs::Vfs;
use sdl3::audio::{AudioCallback, AudioFormat, AudioSpec, AudioStream};
use sdl3::event::Event;
use sdl3::keyboard::Keycode;
use sdl3::pixels::{Color, PixelFormat};

use sdl3::render::{BlendMode, FRect, Texture};
use std::path::{Path, PathBuf};
use std::time::Instant;

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
                println!("With no script name, plays the one STARTSCRIPT.INI names.");
                println!("Keys: Space pause, Left/Right seek 5s, R restart, Esc quit.");
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
    log::info!("ffmpeg {}", days_media::ffmpeg_version());

    let start = Ini::parse_bytes(&vfs.read_path("Ini/STARTSCRIPT.INI")?);
    let film = Ini::parse_bytes(&vfs.read_path("Ini/FILMENGINE.INI")?);
    let english = film.get_bool("UseEnglish").unwrap_or(false);

    // STARTSCRIPT.INI gives a path like "00/00-00-A00"; the route layer and our
    // CLI both use the bare name.
    let wanted = script_name.unwrap_or_else(|| {
        start
            .get("StartScript")
            .unwrap_or("00/00-00-A00")
            .rsplit('/')
            .next()
            .unwrap_or("00-00-A00")
            .to_string()
    });
    let (name, path) = find_script(&vfs, &wanted, english)?;
    log::info!("playing {name} from {path}");
    let script = Script::parse(&name, &vfs.read_path(&path)?)?;
    log::info!(
        "{} events, length {} ({:.1}s)",
        script.events.len(),
        script.length,
        script.length.as_seconds()
    );

    let font = Font::parse(
        vfs.read_path("System/System/FONTDATA_ENG.DAT")
            .or_else(|_| vfs.read_path("System/System/FONTDATA.DAT"))?,
    )?;

    let sdl = sdl3::init().map_err(|e| anyhow::anyhow!("SDL init: {e}"))?;
    let video = sdl.video().map_err(|e| anyhow::anyhow!("SDL video: {e}"))?;
    let window = video
        .window(&format!("DaysEngine — {name}"), STAGE_WIDTH, STAGE_HEIGHT)
        .position_centered()
        .resizable()
        .build()
        .context("creating window")?;
    let mut canvas = window.into_canvas();
    let creator = canvas.texture_creator();

    let mut movie_texture = creator
        .create_texture_streaming(
            PixelFormat::try_from(sdl3::sys::pixels::SDL_PIXELFORMAT_RGBA32)?,
            STAGE_WIDTH,
            STAGE_HEIGHT,
        )
        .context("creating movie texture")?;
    let mut still_texture: Option<(String, Texture)> = None;
    let mut text_texture: Option<(String, u32, u32, Texture)> = None;

    let audio = sdl.audio().map_err(|e| anyhow::anyhow!("SDL audio: {e}"))?;
    let mixer = Mixer::new();
    let spec = AudioSpec {
        freq: Some(days_media::SAMPLE_RATE as i32),
        channels: Some(days_media::CHANNELS as i32),
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

    let mut stage = Stage::new(script);
    let mut events = sdl
        .event_pump()
        .map_err(|e| anyhow::anyhow!("SDL event pump: {e}"))?;

    // The clock is wall-clock based with an offset, so pausing and seeking are
    // both just adjustments to the offset rather than separate state machines.
    let mut origin = Instant::now();
    let mut offset = Frame::ZERO;
    let mut paused = false;

    'running: loop {
        for event in events.poll_iter() {
            match event {
                Event::Quit { .. }
                | Event::KeyDown {
                    keycode: Some(Keycode::Escape),
                    ..
                } => break 'running,
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
            break;
        }

        stage.seek_to(at, &vfs, &mixer)?;
        let visual = stage.visual_at(at);

        canvas.set_draw_color(Color::BLACK);
        canvas.clear();

        // Letterbox the stage into whatever size the window is now.
        let (win_w, win_h) = canvas.output_size().unwrap_or((STAGE_WIDTH, STAGE_HEIGHT));
        let scale = (win_w as f32 / STAGE_WIDTH as f32).min(win_h as f32 / STAGE_HEIGHT as f32);
        let draw_w = STAGE_WIDTH as f32 * scale;
        let draw_h = STAGE_HEIGHT as f32 * scale;
        let dst = FRect::new(
            (win_w as f32 - draw_w) / 2.0,
            (win_h as f32 - draw_h) / 2.0,
            draw_w,
            draw_h,
        );

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
                let image = text::render_line(&font, &display, [255, 255, 255]);
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
                            dst.y + draw_h - th - 16.0 * scale,
                            tw,
                            th,
                        ),
                    )
                    .map_err(|e| anyhow::anyhow!("drawing text: {e}"))?;
            }
        }

        canvas.present();
        // The script clock is the authority; sleeping a little keeps us from
        // spinning a core between frames.
        std::thread::sleep(std::time::Duration::from_millis(2));
    }

    Ok(())
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
