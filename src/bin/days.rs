//! `days` — offline inspection tools for a School Days HQ install.
//!
//! Nothing here is needed to play; it exists so the archive and script formats
//! can be checked against real data rather than against our assumptions.

#![forbid(unsafe_code)]

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use days_gpk::{Archive, Key};
use std::path::{Path, PathBuf};

#[derive(Parser)]
#[command(name = "days", about = "Inspect a School Days HQ installation")]
struct Cli {
    /// Game directory (the one containing "SCHOOLDAYS HQ.exe").
    ///
    /// Defaults to wherever this binary lives, so dropping it into the game
    /// folder and running it works with no arguments.
    #[arg(long, short = 'g', env = "DAYS_GAME_DIR")]
    game: Option<PathBuf>,

    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Print the archive key recovered from the game executable.
    Key,
    /// List entries in one pack, or across all packs with no argument.
    List {
        /// Pack name, e.g. "System" or "Ini". Omit to list every pack.
        pack: Option<String>,
        /// Only show entries whose path contains this substring (case-insensitive).
        #[arg(long)]
        filter: Option<String>,
    },
    /// Extract entries to a directory.
    Extract {
        pack: String,
        #[arg(long, short = 'o')]
        out: PathBuf,
        /// Only extract entries whose path contains this substring.
        #[arg(long)]
        filter: Option<String>,
    },
    /// Read every entry of every pack and verify it decodes. Slow; ~12 GB of I/O.
    Verify {
        /// Restrict to one pack.
        pack: Option<String>,
    },
    /// Parse every .ORS script and report anything the parser cannot handle.
    Scripts {
        /// Also print a per-command histogram.
        #[arg(long)]
        stats: bool,
    },
    /// Parse one script and print its timeline.
    Script {
        /// Script name, e.g. "00-00-A00".
        name: String,
    },
    /// Check that every asset path referenced by every script resolves.
    Assets,
    /// Decode one media asset and report what came out.
    Media {
        /// Logical path, e.g. "Movie00/00-00/00-00-A00/00-00-A00-000".
        path: String,
        /// Write the first decoded video frame here as a PPM.
        #[arg(long)]
        dump_frame: Option<PathBuf>,
    },
    /// Composite frames of a script to PNG, without a display.
    ///
    /// This is the regression check for playback: timing, fades and text layout
    /// all show up as an image rather than as "it looked wrong when I ran it".
    Render {
        /// Script name, e.g. "00-00-A00".
        name: String,
        /// Timecodes to render, as MM:SS:FF. Repeatable.
        #[arg(long = "at", required = true)]
        at: Vec<String>,
        /// Directory to write PNGs into.
        #[arg(long, short = 'o', default_value = ".")]
        out: PathBuf,
    },
    /// Composite a UI screen to PNG, without a display.
    ///
    /// The screen is drawn exactly as the game draws it: base art from the
    /// packs, widget sprites from the `_CHIP` sheet, positioned by the table in
    /// the user's own SysMenuSDHQ.dll. `--active` selects widgets by 1-based
    /// region ID, matching the `.CMAP`.
    Ui(UiArgs),
    /// Drive the menu state machine without a display.
    ///
    /// Replays a script of menu events against the real screens and reports
    /// where each one lands, optionally writing the final frame. This is how
    /// the menus are checked: the SDL player draws through the GPU and cannot
    /// be inspected from a test, but every decision the menu makes happens
    /// here, on the user's own widget tables.
    Menu(MenuArgs),
    /// Decode the glyph store and render characters as ASCII art.
    Font {
        /// Characters to render. Omit to just report coverage.
        text: Option<String>,
        /// Show the alpha (outline) plane instead of the luminance plane.
        #[arg(long)]
        alpha: bool,
        /// Decode every defined glyph and report failures.
        #[arg(long)]
        verify: bool,
    },
    /// Decode the save data and print what the player has unlocked.
    Save {
        /// Print every flag, not just the ones the menus read.
        #[arg(long)]
        all: bool,
        /// Only print flags whose name contains this, implies --all.
        #[arg(long)]
        grep: Option<String>,
    },
    /// Decode every movie referenced by a script, checking frame counts against
    /// the timeline the script declares.
    Timing {
        /// Script name, e.g. "00-00-A00".
        name: String,
    },
}

#[derive(clap::Args)]
struct UiArgs {
    /// Screen path stem as the DLL spells it, e.g. "System/Title/Title".
    screen: String,
    /// Resolution: standard, wide, note or full.
    #[arg(long, short = 'r', default_value = "wide")]
    resolution: String,
    /// Region IDs to draw in their active (hover/selected) state.
    #[arg(long = "active")]
    active: Vec<usize>,
    /// Alternate-state records to draw, by index into the trailing run.
    #[arg(long = "extra")]
    extra: Vec<usize>,
    /// Base art, when the screen does not name it after the stem —
    /// Exit/Popup and SaveLoad/SaveLoad pick theirs by context.
    #[arg(long)]
    base: Option<String>,
    /// Image to draw behind the screen, e.g. the title's
    /// STARTSCRIPT.INI [BaseFile], "System/Title/TitleBase.png".
    #[arg(long)]
    backdrop: Option<String>,
    /// Report the recovered widget table instead of drawing.
    #[arg(long)]
    table: bool,
    /// PNG to write.
    #[arg(long, short = 'o')]
    out: Option<PathBuf>,
}

#[derive(clap::Args)]
struct MenuArgs {
    /// Events to replay, comma separated: `down`, `up`, `enter`, `esc`,
    /// `at:X,Y` to point at a pixel, and `click:X,Y` to point and confirm.
    #[arg(long, short = 'e', default_value = "")]
    events: String,
    /// Resolution: standard, wide, note or full.
    #[arg(long, short = 'r', default_value = "wide")]
    resolution: String,
    /// Ignore the player's save data and start from a fresh install.
    #[arg(long)]
    fresh: bool,
    /// Force the save to all-clear: the title becomes Title_AC.
    #[arg(long)]
    all_clear: bool,
    /// Force the first route cleared: the title becomes Title_Clear.
    #[arg(long)]
    cleared: bool,
    /// Force REPLAY unlocked, which is greyed out on a fresh save.
    #[arg(long)]
    replay: bool,
    /// Image to draw behind the title, normally STARTSCRIPT.INI [BaseFile].
    #[arg(long)]
    backdrop: Option<String>,
    /// PNG to write the final frame to.
    #[arg(long, short = 'o')]
    out: Option<PathBuf>,
    /// Try to open every mode and report which ones this engine can draw.
    #[arg(long)]
    check_all: bool,
}

fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let cli = Cli::parse();

    let game = match cli.game {
        Some(dir) => dir,
        None => discover_game_dir()?,
    };
    let exe = find_executable(&game)?;
    let key = Key::from_executable(&exe)
        .with_context(|| format!("recovering archive key from {}", exe.display()))?;

    match cli.cmd {
        Cmd::Key => {
            println!("{}", exe.display());
            println!("key recovered ({} packs readable)", packs(&game)?.len());
        }
        Cmd::List { pack, filter } => {
            let packs = select_packs(&game, pack.as_deref())?;
            let mut total = 0usize;
            for p in packs {
                let ar = Archive::open(&p, &key)?;
                let name = pack_name(&p);
                for e in ar.entries() {
                    if !matches(&e.name, filter.as_deref()) {
                        continue;
                    }
                    total += 1;
                    println!(
                        "{:<12} {:>12} {:>12}  {}",
                        name,
                        e.size,
                        e.decoded_len(),
                        e.name
                    );
                }
            }
            eprintln!("{total} entries");
        }
        Cmd::Extract { pack, out, filter } => {
            let p = resolve_pack(&game, &pack)?;
            let ar = Archive::open(&p, &key)?;
            let entries: Vec<_> = ar
                .entries()
                .iter()
                .filter(|e| matches(&e.name, filter.as_deref()))
                .cloned()
                .collect();
            let root = out.join(pack_name(&p));
            for e in &entries {
                let data = ar.read(e)?;
                let dest = root.join(&e.name);
                if let Some(parent) = dest.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::write(&dest, &data)
                    .with_context(|| format!("writing {}", dest.display()))?;
            }
            eprintln!("extracted {} entries to {}", entries.len(), root.display());
        }
        Cmd::Scripts { stats } => cmd_scripts(&game, stats)?,
        Cmd::Script { name } => cmd_script(&game, &name)?,
        Cmd::Assets => cmd_assets(&game)?,
        Cmd::Media { path, dump_frame } => cmd_media(&game, &path, dump_frame.as_deref())?,
        Cmd::Timing { name } => cmd_timing(&game, &name)?,
        Cmd::Font {
            text,
            alpha,
            verify,
        } => cmd_font(&game, text.as_deref(), alpha, verify)?,
        Cmd::Render { name, at, out } => cmd_render(&game, &name, &at, &out)?,
        Cmd::Ui(args) => cmd_ui(&game, &args)?,
        Cmd::Menu(args) => cmd_menu(&game, &args)?,
        Cmd::Save { all, grep } => cmd_save(&game, all, grep.as_deref())?,
        Cmd::Verify { pack } => {
            let packs = select_packs(&game, pack.as_deref())?;
            let (mut ok, mut bad) = (0usize, 0usize);
            for p in packs {
                let ar = Archive::open(&p, &key)?;
                let entries: Vec<_> = ar.entries().to_vec();
                for e in &entries {
                    match ar.read(e) {
                        Ok(_) => ok += 1,
                        Err(err) => {
                            bad += 1;
                            eprintln!("FAIL {}/{}: {err}", pack_name(&p), e.name);
                        }
                    }
                }
                eprintln!("{:<12} verified", pack_name(&p));
            }
            println!("{ok} ok, {bad} failed");
            if bad > 0 {
                bail!("{bad} entries failed to decode");
            }
        }
    }
    Ok(())
}

/// Every `.ORS` in the Script pack, as (script name, logical path).
///
/// Pack paths look like `english/00/00-00-A00.ENG.ORS`; the route layer knows
/// the script as `00-00-A00`, so strip the language directory and the `.ENG`
/// infix.
fn script_paths(vfs: &daysengine::vfs::Vfs) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = vfs
        .paths()
        .filter(|p| p.starts_with("script/") && p.ends_with(".ors"))
        .map(|p| {
            let stem = p.rsplit('/').next().unwrap_or(p);
            let name = stem
                .trim_end_matches(".ors")
                .trim_end_matches(".eng")
                .trim_end_matches(".jpn")
                .to_uppercase();
            (name, p.to_string())
        })
        .collect();
    out.sort();
    out
}

fn cmd_scripts(game: &Path, stats: bool) -> Result<()> {
    let vfs = daysengine::vfs::Vfs::mount(game)?;
    let mut histogram: std::collections::BTreeMap<&'static str, usize> = Default::default();
    let (mut ok, mut failed) = (0usize, 0usize);

    for (name, path) in script_paths(&vfs) {
        let bytes = vfs.read_path(&path)?;
        match days_script::Script::parse(&name, &bytes) {
            Ok(script) => {
                ok += 1;
                for e in &script.events {
                    *histogram.entry(command_name(&e.command)).or_default() += 1;
                }
            }
            Err(err) => {
                failed += 1;
                eprintln!("FAIL {name}: {err}");
            }
        }
    }

    if stats {
        let mut rows: Vec<_> = histogram.iter().collect();
        rows.sort_by_key(|(_, n)| std::cmp::Reverse(**n));
        for (cmd, n) in rows {
            println!("{n:>8}  {cmd}");
        }
    }
    println!("{ok} scripts parsed, {failed} failed");
    if failed > 0 {
        bail!("{failed} scripts failed to parse");
    }
    Ok(())
}

fn cmd_script(game: &Path, name: &str) -> Result<()> {
    let vfs = daysengine::vfs::Vfs::mount(game)?;
    let wanted = name.to_uppercase();
    let (_, path) = script_paths(&vfs)
        .into_iter()
        .find(|(n, _)| *n == wanted)
        .with_context(|| format!("no script named {name}"))?;
    let script = days_script::Script::parse(&wanted, &vfs.read_path(&path)?)?;

    println!("{} — length {}", script.name, script.length);
    for e in &script.events {
        println!("  {} -> {}  {:?}", e.start, e.end, e.command);
    }
    Ok(())
}

fn cmd_assets(game: &Path) -> Result<()> {
    let vfs = daysengine::vfs::Vfs::mount(game)?;
    let mut missing: std::collections::BTreeMap<String, usize> = Default::default();
    let mut checked = 0usize;

    for (name, path) in script_paths(&vfs) {
        let script = days_script::Script::parse(&name, &vfs.read_path(&path)?)?;
        for e in &script.events {
            let (asset, ext) = match &e.command {
                days_script::Command::PlayVoice { path, .. } => (path, "ogg"),
                days_script::Command::PlaySe { path, .. } => (path, "ogg"),
                days_script::Command::PlayBgm { path } => (path, "ogg"),
                days_script::Command::EndBgm { path } => (path, "ogg"),
                days_script::Command::CreateBg { path, .. } => (path, "png"),
                days_script::Command::PlayMovie { path, .. } => (path, "wmv"),
                days_script::Command::EndRoll { path } => (path, "wmv"),
                _ => continue,
            };
            checked += 1;
            if vfs.resolve_as(asset, ext).is_none() {
                *missing.entry(format!("{asset}.{ext}")).or_default() += 1;
            }
        }
    }

    for (path, n) in &missing {
        println!("MISSING x{n:<4} {path}");
    }
    println!(
        "{checked} references checked, {} distinct missing",
        missing.len()
    );
    Ok(())
}

fn cmd_media(game: &Path, path: &str, dump_frame: Option<&Path>) -> Result<()> {
    let vfs = daysengine::vfs::Vfs::mount(game)?;
    println!("ffmpeg {}", daysengine::media::ffmpeg_version());

    let handle = vfs
        .resolve(path)
        .with_context(|| format!("no asset at {path}"))?;
    let entry = vfs.entry(handle);
    let name = entry.name.clone();
    let bytes = vfs.read(handle)?;
    println!("{} ({} bytes)", name, bytes.len());

    if name.to_ascii_lowercase().ends_with(".wmv") {
        let mut decoder = daysengine::media::VideoDecoder::open(bytes)?;
        println!("video {}x{}", decoder.width(), decoder.height());
        let mut frames = 0usize;
        let mut last = 0.0;
        let mut first: Option<daysengine::media::VideoFrame> = None;
        while let Some(frame) = decoder.next_frame()? {
            last = frame.timestamp;
            if first.is_none() {
                first = Some(frame);
            }
            frames += 1;
        }
        println!(
            "{frames} frames, last pts {last:.3}s ({:.2} fps average)",
            if last > 0.0 {
                (frames - 1) as f64 / last
            } else {
                0.0
            }
        );
        if let (Some(path), Some(frame)) = (dump_frame, first) {
            write_ppm(path, &frame)?;
            println!("wrote first frame to {}", path.display());
        }
    } else {
        let audio = daysengine::media::decode_audio(bytes)?;
        println!(
            "audio {} frames, {:.3}s, peak {:.3}",
            audio.frames(),
            audio.duration_seconds(),
            audio.samples.iter().fold(0f32, |m, s| m.max(s.abs()))
        );
    }
    Ok(())
}

/// Writes an RGBA frame as a binary PPM, dropping alpha. Enough to eyeball a
/// decode without pulling in an image encoder.
fn write_ppm(path: &Path, frame: &daysengine::media::VideoFrame) -> Result<()> {
    let mut out = format!("P6\n{} {}\n255\n", frame.width, frame.height).into_bytes();
    out.extend(
        frame
            .rgba
            .as_chunks::<4>()
            .0
            .iter()
            .flat_map(|p| [p[0], p[1], p[2]]),
    );
    std::fs::write(path, out)?;
    Ok(())
}

/// Cross-checks decoded movie lengths against the durations the script declares.
///
/// A mismatch means either the timecode interpretation is wrong or the engine is
/// expected to cut a movie short, and we would rather find out here than by
/// watching playback drift.
fn cmd_timing(game: &Path, name: &str) -> Result<()> {
    let vfs = daysengine::vfs::Vfs::mount(game)?;
    let wanted = name.to_uppercase();
    let (_, script_path) = script_paths(&vfs)
        .into_iter()
        .find(|(n, _)| *n == wanted)
        .with_context(|| format!("no script named {name}"))?;
    let script = days_script::Script::parse(&wanted, &vfs.read_path(&script_path)?)?;

    println!(
        "{:<46} {:>9} {:>9} {:>8}",
        "movie", "script", "decoded", "delta"
    );
    let mut worst = 0.0f64;
    for event in &script.events {
        let days_script::Command::PlayMovie { path, .. } = &event.command else {
            continue;
        };
        let Some(handle) = vfs.resolve_as(path, "wmv") else {
            println!("{path:<46} MISSING");
            continue;
        };
        let mut decoder = daysengine::media::VideoDecoder::open(vfs.read(handle)?)?;
        let mut frames = 0usize;
        while decoder.next_frame()?.is_some() {
            frames += 1;
        }
        let decoded = frames as f64 / f64::from(days_script::FPS);
        let declared = event.duration().as_seconds();
        let delta = decoded - declared;
        worst = worst.max(delta.abs());
        println!(
            "{:<46} {declared:>8.3}s {decoded:>8.3}s {delta:>+7.3}s",
            path.rsplit('/').next().unwrap_or(path)
        );
    }
    println!("worst absolute difference: {worst:.3}s");
    Ok(())
}

fn cmd_render(game: &Path, name: &str, at: &[String], out: &Path) -> Result<()> {
    use days_script::Frame;

    let vfs = daysengine::vfs::Vfs::mount(game)?;
    let wanted = name.to_uppercase();
    let (_, script_path) = script_paths(&vfs)
        .into_iter()
        .find(|(n, _)| *n == wanted)
        .with_context(|| format!("no script named {name}"))?;
    let script = days_script::Script::parse(&wanted, &vfs.read_path(&script_path)?)?;
    let font = days_font::Font::parse(
        vfs.read_path("System/System/FONTDATA_ENG.DAT")
            .or_else(|_| vfs.read_path("System/System/FONTDATA.DAT"))?,
    )?;

    // Targets are rendered in order and the stage is only ever advanced forward,
    // so the timeline is exercised the same way playback exercises it.
    let mut targets: Vec<Frame> = at
        .iter()
        .map(|s| Frame::parse(s).map_err(anyhow::Error::from))
        .collect::<Result<_>>()?;
    targets.sort();

    let mixer = daysengine::Mixer::new();
    let mut stage = daysengine::Stage::new(script);
    std::fs::create_dir_all(out)?;

    const W: usize = 800;
    const H: usize = 452;

    for target in targets {
        stage.seek_to(target, &vfs, &mixer)?;
        let visual = stage.visual_at(target);
        let described = format!(
            "movie={} still={} text={:?} fade={:?}",
            visual.movie.is_some(),
            visual.still.map(|s| s.path.as_str()).unwrap_or("-"),
            visual.text.map(|(s, t)| format!("{s}: {t}")),
            visual.fade,
        );
        let rgba = daysengine::compose::frame_rgba(&visual, &font, W, H);

        let path = out.join(format!(
            "{wanted}-{}.png",
            target.to_string().replace(':', "-")
        ));
        write_png(&path, &rgba, W as u32, H as u32)?;
        println!("{} {}  {described}", target, path.display());
    }
    Ok(())
}

/// Reads the system-menu DLL, which is where the widget-to-sprite table lives.
///
/// Like the archive key, this comes out of the user's own install at runtime;
/// none of it is embedded here.
fn system_menu_dll(game: &Path) -> Result<Vec<u8>> {
    let path = game.join("SysMenuSDHQ.dll");
    std::fs::read(&path).with_context(|| {
        format!(
            "reading {} — the UI widget tables live in it",
            path.display()
        )
    })
}

fn cmd_ui(game: &Path, args: &UiArgs) -> Result<()> {
    use daysengine::screen::{Resolution, Screen, WidgetState};

    let vfs = daysengine::vfs::Vfs::mount(game)?;
    let dll = system_menu_dll(game)?;
    let resolution = Resolution::from_name(&args.resolution)
        .with_context(|| format!("unknown resolution {}", args.resolution))?;
    let screen =
        Screen::load_with_base(&vfs, &dll, &args.screen, args.base.as_deref(), resolution)?;

    let (w, h) = screen.size();
    println!(
        "{} at {} — {w}x{h}, scale {:.2}, letterbox {}px, {} widgets \
         ({}/{} boxes matched the table)",
        screen.path,
        resolution.name(),
        screen.scale(),
        screen.letterbox(),
        screen.widget_count(),
        screen.atlas().matched,
        screen.widget_count(),
    );

    if args.table {
        let atlas = screen.atlas();
        println!("widget table at DLL offset {:#x}", atlas.offset);
        for (i, wgt) in atlas.widgets.iter().enumerate() {
            println!(
                "  id {:3}  dst ({:4},{:4}) {:4}x{:<3}  src ({:4},{:4})",
                i + 1,
                wgt.dst.x,
                wgt.dst.y,
                wgt.dst.width,
                wgt.dst.height,
                wgt.src_x,
                wgt.src_y
            );
        }
        for (i, wgt) in atlas.extras.iter().enumerate() {
            println!(
                "  alt {:3}  dst ({:4},{:4}) {:4}x{:<3}  src ({:4},{:4})",
                i, wgt.dst.x, wgt.dst.y, wgt.dst.width, wgt.dst.height, wgt.src_x, wgt.src_y
            );
        }
        if args.out.is_none() {
            return Ok(());
        }
    }

    let mut states = vec![WidgetState::Resting; screen.widget_count()];
    for id in &args.active {
        let Some(slot) = id.checked_sub(1).and_then(|i| states.get_mut(i)) else {
            bail!(
                "region {id} is not one of this screen's {} widgets",
                states.len()
            );
        };
        *slot = WidgetState::Active;
    }
    // Alternate-state records are not per-widget, so they are appended as their
    // own entries rather than replacing a widget's state.
    for n in &args.extra {
        states.push(WidgetState::Extra(*n));
    }

    let backdrop = args
        .backdrop
        .as_deref()
        .map(|p| {
            let bytes = vfs
                .read_path(p)
                .with_context(|| format!("reading backdrop {p}"))?;
            days_ui::Image::decode_png(&bytes).map_err(anyhow::Error::from)
        })
        .transpose()?;
    // Only write when asked to. Defaulting to a file in the working directory
    // litters whatever the caller happened to be standing in, and the summary
    // above is the useful part when you are just checking that a screen loads.
    if let Some(path) = &args.out {
        let image = screen.compose_over(backdrop.as_ref(), &states);
        write_png(path, &image.rgba, image.width, image.height)?;
        println!("wrote {}", path.display());
    }
    Ok(())
}

fn write_png(path: &Path, rgba: &[u8], width: u32, height: u32) -> Result<()> {
    let file =
        std::fs::File::create(path).with_context(|| format!("creating {}", path.display()))?;
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header()?.write_image_data(rgba)?;
    Ok(())
}

fn cmd_font(game: &Path, text: Option<&str>, alpha: bool, verify: bool) -> Result<()> {
    let vfs = daysengine::vfs::Vfs::mount(game)?;
    // The English build ships both; FONTDATA_ENG is the one the localised
    // executable selects.
    let bytes = vfs
        .read_path("System/System/FONTDATA_ENG.DAT")
        .or_else(|_| vfs.read_path("System/System/FONTDATA.DAT"))?;
    let font = days_font::Font::parse(bytes)?;
    println!("{} glyphs defined", font.glyph_count());

    if verify {
        let (mut ok, mut failed) = (0usize, 0usize);
        for cp in 0..=0xffffu32 {
            let Some(c) = char::from_u32(cp) else {
                continue;
            };
            match font.glyph(c) {
                Ok(Some(_)) => ok += 1,
                Ok(None) => {}
                Err(err) => {
                    failed += 1;
                    eprintln!("FAIL U+{cp:04X}: {err}");
                }
            }
        }
        println!("{ok} glyphs decoded, {failed} failed");
        if failed > 0 {
            bail!("{failed} glyphs failed to decode");
        }
    }

    let ramp: Vec<char> = " .:-=+*#%@".chars().collect();
    for c in text.unwrap_or("").chars() {
        let Some(glyph) = font.glyph(c)? else {
            println!("\nU+{:04X} {c:?}: no glyph", u32::from(c));
            continue;
        };
        let plane = if alpha {
            &glyph.alpha
        } else {
            &glyph.luminance
        };
        println!(
            "\nU+{:04X} {c:?}  ink bounds {:?}  ({} plane)",
            u32::from(c),
            glyph.ink_bounds(),
            if alpha { "alpha" } else { "luminance" }
        );
        for row in plane.chunks(days_font::CELL) {
            let line: String = row
                .iter()
                .map(|&v| ramp[(usize::from(v) * (ramp.len() - 1)) / 255])
                .collect();
            if line.trim().is_empty() {
                continue;
            }
            println!("  {line}");
        }
    }
    Ok(())
}

fn command_name(c: &days_script::Command) -> &'static str {
    use days_script::Command as C;
    match c {
        C::PrintText { .. } => "PrintText",
        C::PlayVoice { .. } => "PlayVoice",
        C::CreateBg { .. } => "CreateBG",
        C::PlaySe { .. } => "PlaySe",
        C::PlayMovie { .. } => "PlayMovie",
        C::PlayBgm { .. } => "PlayBgm",
        C::EndBgm { .. } => "EndBGM",
        C::EndRoll { .. } => "EndRoll",
        C::BlackFade(_) => "BlackFade",
        C::WhiteFade(_) => "WhiteFade",
        C::SetSelect { .. } => "SetSELECT",
        C::MoveSom { .. } => "MoveSom",
        C::SkipFrame => "SkipFRAME",
        C::Next => "Next",
    }
}

/// Finds the game without being told where it is.
///
/// The intended deployment is dropping our binaries straight into the user's
/// existing game folder, so the directory holding this executable is the first
/// guess. The working directory is the fallback, which covers running from a
/// build tree with `cwd` set to the install.
fn discover_game_dir() -> Result<PathBuf> {
    let mut tried = Vec::new();
    let candidates = [
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(Path::to_path_buf)),
        std::env::current_dir().ok(),
    ];
    for dir in candidates.into_iter().flatten() {
        if is_game_dir(&dir) {
            log::debug!("using game directory {}", dir.display());
            return Ok(dir);
        }
        tried.push(dir);
    }
    bail!(
        "no School Days HQ install found (looked in: {}).\n\
         Put this binary in the game folder, or pass --game <dir>.",
        tried
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join(", ")
    )
}

/// A directory is the game if it has a Packs/ directory with at least one GPK.
/// Checking for the executable by name would break on localised and repackaged
/// installs, which rename it.
fn is_game_dir(dir: &Path) -> bool {
    packs(dir).is_ok_and(|p| !p.is_empty())
}

fn matches(name: &str, filter: Option<&str>) -> bool {
    match filter {
        None => true,
        Some(f) => name.to_ascii_lowercase().contains(&f.to_ascii_lowercase()),
    }
}

fn pack_name(p: &Path) -> String {
    p.file_stem().unwrap_or_default().to_string_lossy().into()
}

/// Locates the game executable. The retail name has a space in it, and localised
/// or repackaged installs vary, so fall back to any `.exe` carrying the key.
fn find_executable(dir: &Path) -> Result<PathBuf> {
    let preferred = dir.join("SCHOOLDAYS HQ.exe");
    if preferred.is_file() {
        return Ok(preferred);
    }
    for entry in std::fs::read_dir(dir)
        .with_context(|| format!("reading game directory {}", dir.display()))?
    {
        let path = entry?.path();
        if path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("exe"))
            && Key::from_executable(&path).is_ok()
        {
            return Ok(path);
        }
    }
    bail!(
        "no game executable with a CIPHERCODE resource found in {}",
        dir.display()
    )
}

fn packs(dir: &Path) -> Result<Vec<PathBuf>> {
    let packs_dir = dir.join("Packs");
    let mut out = Vec::new();
    for entry in
        std::fs::read_dir(&packs_dir).with_context(|| format!("reading {}", packs_dir.display()))?
    {
        let path = entry?.path();
        if path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("gpk"))
        {
            out.push(path);
        }
    }
    out.sort();
    Ok(out)
}

fn resolve_pack(dir: &Path, name: &str) -> Result<PathBuf> {
    packs(dir)?
        .into_iter()
        .find(|p| pack_name(p).eq_ignore_ascii_case(name))
        .with_context(|| format!("no pack named {name}"))
}

fn select_packs(dir: &Path, name: Option<&str>) -> Result<Vec<PathBuf>> {
    match name {
        Some(n) => Ok(vec![resolve_pack(dir, n)?]),
        None => packs(dir),
    }
}

/// Reads the player's global flag store, through the path their INI names.
///
/// Shared by `menu` and `save` so both see the install the same way. A missing
/// or unreadable file reads as a fresh install, which is what it means.
fn load_flags(game: &Path, vfs: &daysengine::vfs::Vfs) -> daysengine::save::FlagStore {
    let film = match vfs.read_path("Ini/FILMENGINE.INI") {
        Ok(bytes) => daysengine::Ini::parse_bytes(&bytes),
        Err(err) => {
            log::warn!("reading Ini/FILMENGINE.INI: {err}");
            daysengine::Ini::parse("")
        }
    };
    daysengine::save::load_flags(game, &film)
}

/// Prints what the save data says the player has unlocked.
fn cmd_save(game: &Path, all: bool, grep: Option<&str>) -> Result<()> {
    use daysengine::save::Value;
    use daysengine::SaveState;

    let vfs = daysengine::vfs::Vfs::mount(game)?;
    let flags = load_flags(game, &vfs);
    let save = SaveState::from_flags(&flags);

    println!("{} flags", flags.len());
    println!();
    println!("what the title screen reads:");
    println!("  AllClear          {}", flags.flag("AllClear"));
    println!("  EndClear          {}", flags.flag("EndClear"));
    match flags.get("EndNo").and_then(Value::as_int) {
        Some(n) => println!("  EndNo             {n} endings seen"),
        None => println!("  EndNo             (not set)"),
    }
    println!();
    println!(
        "so the title screen shows {}, REPLAY {}",
        save.title_variant(),
        if save.replay_unlocked() {
            "unlocked"
        } else {
            "locked"
        }
    );

    if all || grep.is_some() {
        println!();
        for (name, value) in flags.iter() {
            if grep.is_some_and(|g| !name.to_lowercase().contains(&g.to_lowercase())) {
                continue;
            }
            match value {
                Value::Bool(b) => println!("  {name:<40} {b}"),
                Value::Int(n) => println!("  {name:<40} {n}"),
                Value::Float(f) => println!("  {name:<40} {f}"),
                Value::Str(s) => println!("  {name:<40} {s:?}"),
            }
        }
    }
    Ok(())
}

/// Drives the menu state machine and reports where each event lands.
fn cmd_menu(game: &Path, args: &MenuArgs) -> Result<()> {
    use days_ui::Image;
    use daysengine::menu::{Action, Menu, Mode, SaveState};
    use daysengine::screen::Resolution;

    let vfs = daysengine::vfs::Vfs::mount(game)?;
    let dll = system_menu_dll(game)?;
    let resolution = Resolution::from_name(&args.resolution)
        .with_context(|| format!("unknown resolution {}", args.resolution))?;
    // The player's real save decides this; the flags below only force things
    // on, so a fresh install can still be driven through every screen.
    let mut save = if args.fresh {
        SaveState::default()
    } else {
        SaveState::from_flags(&load_flags(game, &vfs))
    };
    save.all_clear |= args.all_clear;
    save.cleared_first |= args.cleared;
    save.cleared_replay |= args.replay;

    if args.check_all {
        // Every mode SystemInit can dispatch to, opened at its own default
        // variant. A refusal here is a screen this engine cannot draw yet, not
        // a guess that happened to miss.
        let modes = [
            ("title", Mode::TITLE),
            ("saveload", Mode::SAVELOAD),
            ("option", Mode::OPTION),
            ("replay", Mode::REPLAY),
            ("routemap", Mode::ROUTEMAP),
            ("som config", Mode::SOM_CONFIG),
            ("replay popup", Mode::REPLAY_POPUP),
            ("confirm popup", Mode::CONFIRM),
        ];
        for (name, mode) in modes {
            let variant = if mode == Mode::TITLE {
                save.title_variant()
            } else {
                mode.default_variant()
            };
            let stem = mode.stem(variant).unwrap_or_default();
            match Menu::open(&vfs, &dll, mode, save, resolution) {
                Ok(menu) => println!(
                    "  mode {:>2}  {name:<14} {stem:<34} ok, {} widgets",
                    mode.0,
                    menu.screen().widget_count()
                ),
                Err(err) => println!("  mode {:>2}  {name:<14} {stem:<34} {err}", mode.0),
            }
        }
        return Ok(());
    }

    let mut menu = Menu::open(&vfs, &dll, Mode::TITLE, save, resolution)
        .context("opening the title screen")?;
    println!(
        "mode {} ({}) — {} widgets",
        menu.mode().0,
        menu.variant(),
        menu.screen().widget_count()
    );

    for event in args
        .events
        .split(',')
        .map(str::trim)
        .filter(|e| !e.is_empty())
    {
        let (action, label) = match event {
            "down" => (menu.navigate(1), "down".to_string()),
            "up" => (menu.navigate(-1), "up".to_string()),
            "enter" => (menu.confirm(&vfs, &dll)?, "enter".to_string()),
            "esc" => (menu.cancel(&vfs, &dll)?, "esc".to_string()),
            "yes" => (menu.confirm_popup(&vfs, &dll)?, "yes".to_string()),
            other => {
                // Points are written `at:X:Y` rather than `at:X,Y` so the comma
                // stays free as the separator between events.
                let (kind, point) = other
                    .split_once(':')
                    .filter(|(k, _)| *k == "at" || *k == "click")
                    .with_context(|| {
                        format!(
                            "unknown menu event {other:?}; expected down, up, enter, \
                             esc, yes, at:X:Y or click:X:Y"
                        )
                    })?;
                let (x, y) = point
                    .split_once(':')
                    .with_context(|| format!("{other:?} needs both coordinates, as {kind}:X:Y"))?;
                let x: u32 = x.parse().with_context(|| format!("bad x in {other:?}"))?;
                let y: u32 = y.parse().with_context(|| format!("bad y in {other:?}"))?;
                let pointed = menu.point_at(x, y);
                if kind == "at" {
                    (pointed, format!("at {x},{y}"))
                } else {
                    (menu.confirm(&vfs, &dll)?, format!("click {x},{y}"))
                }
            }
        };
        println!(
            "  {label:<12} -> {action:?}  mode {} selection {:?}",
            menu.mode().0,
            menu.selection()
        );
        match action {
            Action::Play => {
                println!("  (would start the script)");
                break;
            }
            Action::Quit => {
                println!("  (would quit)");
                break;
            }
            _ => {}
        }
    }

    if let Some(out) = &args.out {
        let backdrop = match &args.backdrop {
            Some(path) => Some(Image::decode_png(&vfs.read_path(path)?)?),
            None => None,
        };
        let image = menu.compose(backdrop.as_ref());
        write_png(out, &image.rgba, image.width, image.height)?;
        println!("wrote {}", out.display());
    }
    Ok(())
}
