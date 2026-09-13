//! `days` — offline inspection tools for a School Days HQ install.
//!
//! Nothing here is needed to play; it exists so the archive and script formats
//! can be checked against real data rather than against our assumptions.

#![forbid(unsafe_code)]

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use days_gpk::{Archive, Key};
use daysengine::ui::ending;
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
        /// Decode video at this window size, e.g. "1920x1085", the way the
        /// player does. Reports how long a frame costs, which is what says
        /// whether a machine can hold 24 fps at that size.
        #[arg(long, value_name = "WxH")]
        at_size: Option<String>,
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
        /// Also drop the in-game control bar over the frame, with the pointer
        /// inside the strip. Without this the bar is off, which is what the
        /// engine shows while the pointer is anywhere else.
        #[arg(long)]
        bar: bool,
        /// Answer host `+0x98` true, so the REPLAYMODE indicator is on the
        /// picture — what a row of the replay screen's play-data list starts.
        #[arg(long)]
        following_record: bool,
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
        /// Decode a save slot and print what it holds.
        #[arg(long, value_name = "N")]
        slot: Option<u32>,
        /// Read every save file, write it back, and check the bytes match.
        #[arg(long)]
        roundtrip: bool,
    },
    /// Print DaysEngine's own settings, and a template for the file they
    /// come from.
    ///
    /// These are the engine's choices, not the game's: which filter scales a
    /// movie frame, which scales the UI art. The game's own settings are
    /// `days config`.
    Settings {
        /// Print a commented file of the defaults, to redirect into place.
        #[arg(long)]
        template: bool,
    },
    /// Print the player's settings, as the Option screen reads them.
    ///
    /// `Config.DAT` is a deflated `.INI` next to the executable. This shows the
    /// ten settings the Option screen loads, with the volume levels worked
    /// through the DLL's own attenuation formula, and every other key the file
    /// carries.
    Config,
    /// Print the replay scene table recovered from the user's SysMenuSDHQ.dll.
    ///
    /// The forty-one scenes, which page each sits on, the save flag that
    /// unlocks it and the script a click starts. With no save data to hand it
    /// still lists the table; with save data it marks what is unlocked.
    Replay {
        /// Only show scenes the save data has unlocked.
        #[arg(long)]
        unlocked: bool,
    },
    /// Print a dialog template out of the user's own executable.
    ///
    /// The save-comment box is a Win32 dialog from the executable's resources,
    /// not something the menu DLL draws. This reads the template the engine
    /// lays out from.
    Dialog {
        /// Resource id, default the comment dialog for the install's language.
        #[arg(long)]
        id: Option<String>,
        /// Draw the dialog the way the engine does, to a PNG.
        #[arg(short, long)]
        out: Option<PathBuf>,
        /// Text to show in the edit field.
        #[arg(long, default_value = "")]
        text: String,
    },
    /// Print the branch graph recovered from the user's RouteProcSDHQ.dll.
    ///
    /// The 55 routes and their script-name tables, and the two affection
    /// tables that go with them. With a script named, says where it sits in
    /// the graph, what it credits and what it is gated on.
    Route {
        /// A script to locate, e.g. "00-00-A04" or "00/00-00-A04".
        name: Option<String>,
        /// List every scene of every route, not just a summary line each.
        #[arg(long)]
        scenes: bool,
        /// Print the recovered transition for every scene, and check the
        /// graph against the name tables.
        #[arg(long)]
        edges: bool,
        /// Walk the graph from a script, the way playback does: chain from
        /// scene to scene, answering each choice box in turn.
        #[arg(long, value_name = "SCRIPT")]
        play: Option<String>,
        /// The choices to answer with while walking, e.g. "0,1,0". Runs out
        /// to -1, which is what a choice box that times out reports.
        #[arg(long, value_name = "LIST", default_value = "")]
        choices: String,
        /// How many scripts to play before stopping.
        #[arg(long, default_value_t = 40)]
        steps: usize,
    },
    /// Decode every movie referenced by a script, checking frame counts against
    /// the timeline the script declares.
    Timing {
        /// Script name, e.g. "00-00-A00".
        name: String,
    },
    /// Report the in-game control bar: what each widget is and what it does.
    ///
    /// The bar's geometry comes from the user's own DLL like any other screen,
    /// but its behaviour is a dispatch rather than a table, so this prints the
    /// decision for every widget under whatever engine state is asked for.
    Bar(BarArgs),
    /// Report a choice box: which hit map it uses and where a click lands.
    ///
    /// `System/Select` ships hit maps and nothing else, and only for two of the
    /// four UI sizes, so this is the way to see which map a resolution really
    /// gets and what the engine falls back to when there is none.
    Select(SelectArgs),
}

#[derive(clap::Args)]
struct BarArgs {
    /// Resolution: standard, wide, note or full.
    #[arg(long, short = 'r', default_value = "wide")]
    resolution: String,
    /// Widget to treat as hovered, 0-based, as the dispatch numbers them.
    #[arg(long)]
    hover: Option<usize>,
    /// The auto flag widget 0 toggles is set.
    #[arg(long)]
    auto: bool,
    /// Playback is paused.
    #[arg(long)]
    paused: bool,
    /// Playback was started from the replay menu.
    #[arg(long)]
    replay: bool,
    /// The host's message flag is set.
    #[arg(long)]
    message: bool,
    /// Answer host `+0x88` false: no script is loaded, so the path at
    /// `engine + 0x188` does not resolve and `_GetSkipFlag@0` is clear.
    ///
    /// This is the only way that slot answers false. Over a playing script it
    /// is true whatever the Skip setting says, so the speed row is live — the
    /// default here.
    #[arg(long)]
    no_script: bool,
    /// Answer host `+0x98` true: playback is following a save's recorded
    /// answers, which is what the replay screen's play-data list starts. It is
    /// what lights the transparency slider on the right of the strip, makes its
    /// ten cells pressable, and puts the REPLAYMODE indicator on the picture.
    #[arg(long)]
    following_record: bool,
    /// How solid the REPLAYMODE indicator is drawn, 0 to 10. The bar starts at
    /// 10 and the ten cells set it; level 1 is unreachable.
    #[arg(long)]
    transparency: Option<usize>,
    /// Playback rate index, 0 to 4.
    #[arg(long, default_value_t = 0)]
    speed: usize,
    /// Milliseconds since the auto flag was set, for its animation frame.
    #[arg(long, default_value_t = 0)]
    elapsed: u32,
    /// Where the pointer is, in the strip's own pixels: `X,Y`, or `off` for
    /// anywhere else on the screen. The bar is a drop-down, so `off` is what
    /// fades it away.
    #[arg(long, default_value = "0,0")]
    pointer: String,
    /// Milliseconds the pointer has been where `--pointer` says, so a ramp can
    /// be seen part way through. Defaults to long enough to have settled.
    #[arg(long)]
    after: Option<u32>,
    /// The two affection counters the gauge draws, as `001,002`. Defaults to
    /// whatever the player's own save holds.
    #[arg(long)]
    feeling: Option<String>,
    /// Raise the gauge, which a delta to either counter does: it then draws
    /// over a bar that is otherwise faded away.
    #[arg(long)]
    gauge: bool,
    /// PNG to write the composited bar to. It has an alpha channel: the strip
    /// is a layer the engine draws over the frame, not a picture with a black
    /// bar in it.
    #[arg(long, short = 'o')]
    out: Option<PathBuf>,
}

#[derive(clap::Args)]
struct SelectArgs {
    /// First choice label.
    label: String,
    /// Second choice label. Omit, or pass `null`, for a one-choice box.
    label2: Option<String>,
    /// Resolution: standard, wide, note or full.
    #[arg(long, short = 'r', default_value = "full")]
    resolution: String,
    /// Hit-test a normalised point, as `X,Y` in 0.0..1.0.
    #[arg(long = "at")]
    at: Vec<String>,
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
    /// STARTSCRIPT.INI `[BaseFile]`, "System/Title/TitleBase.png".
    #[arg(long)]
    backdrop: Option<String>,
    /// Report the recovered widget table instead of drawing.
    #[arg(long)]
    table: bool,
    /// Composite at this window size, e.g. "1920x1080", the way the player's
    /// window does rather than at the hit map's own size. This is the pass a
    /// change of `[UI] Scaler` shows up in.
    #[arg(long, value_name = "WxH")]
    at_size: Option<String>,
    /// PNG to write.
    #[arg(long, short = 'o')]
    out: Option<PathBuf>,
}

#[derive(clap::Args)]
struct MenuArgs {
    /// Composite and hit-test at this window size, e.g. "1920x1080", the way
    /// the player's window does rather than at the hit map's own size.
    #[arg(long, value_name = "WxH")]
    at_size: Option<String>,
    /// Events to replay, comma separated: `down`, `up`, `left`, `right`,
    /// `enter`, `esc`, `at:X:Y` to point at a pixel, and `click:X:Y` to point
    /// and confirm.
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
    /// Image to draw behind the title, overriding the one the save chooses.
    /// A `.wmv` ending card is accepted and shows its first frame.
    #[arg(long)]
    backdrop: Option<String>,
    /// PNG to write the final frame to.
    #[arg(long, short = 'o')]
    out: Option<PathBuf>,
    /// Try to open every mode and report which ones this engine can draw.
    #[arg(long)]
    check_all: bool,
    /// Open the screen the way the in-game control bar opens it, over live
    /// playback, instead of starting at the title. Takes `setSystemInit`'s own
    /// code, which is what the bar passes host `+0xf8`: 4 for the save screen,
    /// 5 for the load screen, 2 for the Option screen.
    ///
    /// This is the entry that decides where Close goes, so it is the only way
    /// to check that from here. With it, `-e click:X:Y` on Close reports
    /// `Play` — leave the menus and resume — where a title-rooted run reports
    /// `Opened(Mode(2))`.
    #[arg(long, value_name = "CODE")]
    from_bar: Option<u32>,
}

fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let cli = Cli::parse();

    // The engine's own settings are the one thing here that is not about the
    // player's install, so this answers without needing to find one.
    if let Cmd::Settings { template } = cli.cmd {
        cmd_settings(template);
        return Ok(());
    }

    // These tools are the verification path for what the engine draws, so they
    // have to draw it the same way: same UI kernel, out of the same file.
    daysengine::playback::scale::set_kernel(
        daysengine::install::engine::Settings::load()
            .ui_scaler
            .kernel(),
    );

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
        // Handled before the install is found.
        Cmd::Settings { .. } => {}
        Cmd::Media {
            path,
            dump_frame,
            at_size,
        } => cmd_media(&game, &path, dump_frame.as_deref(), at_size.as_deref())?,
        Cmd::Timing { name } => cmd_timing(&game, &name)?,
        Cmd::Font {
            text,
            alpha,
            verify,
        } => cmd_font(&game, text.as_deref(), alpha, verify)?,
        Cmd::Render {
            name,
            at,
            out,
            bar,
            following_record,
        } => cmd_render(&game, &name, &at, &out, bar, following_record)?,
        Cmd::Ui(args) => cmd_ui(&game, &args)?,
        Cmd::Menu(args) => cmd_menu(&game, &args)?,
        Cmd::Save {
            all,
            grep,
            slot,
            roundtrip,
        } => {
            if roundtrip {
                cmd_save_roundtrip(&game)?
            } else if let Some(n) = slot {
                cmd_save_slot(&game, n)?
            } else {
                cmd_save(&game, all, grep.as_deref())?
            }
        }
        Cmd::Config => cmd_config(&game)?,
        Cmd::Dialog { id, out, text } => cmd_dialog(&game, id.as_deref(), out.as_deref(), &text)?,
        Cmd::Replay { unlocked } => cmd_replay(&game, unlocked)?,
        Cmd::Route {
            name,
            scenes,
            edges,
            play,
            choices,
            steps,
        } => match play {
            Some(from) => cmd_route_play(&game, &from, &choices, steps)?,
            None => cmd_route(&game, name.as_deref(), scenes, edges)?,
        },
        Cmd::Bar(args) => cmd_bar(&game, &args)?,
        Cmd::Select(args) => cmd_select(&game, &args)?,
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
fn script_paths(vfs: &daysengine::install::vfs::Vfs) -> Vec<(String, String)> {
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
    let vfs = daysengine::install::vfs::Vfs::mount(game)?;
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
    let vfs = daysengine::install::vfs::Vfs::mount(game)?;
    let wanted = name.to_uppercase();
    let (_, path) = script_paths(&vfs)
        .into_iter()
        .find(|(n, _)| *n == wanted)
        .with_context(|| format!("no script named {name}"))?;
    let script = days_script::Script::parse(&wanted, &vfs.read_path(&path)?)?;

    // The two boundaries are different frames whenever the script raises a
    // choice: `length` is `[Next]`, `skip_to` is `[SkipFRAME]`, which is where
    // the control bar's skip button jumps to.
    print!("{} — length {}", script.name, script.length);
    if script.skip_to < script.length {
        let landing = days_script::Frame(script.skip_to.0.saturating_sub(days_script::FPS));
        println!(
            ", skip to {} (landing one second earlier at {landing})",
            script.skip_to
        );
    } else {
        println!(" — no choice to skip to");
    }
    for e in &script.events {
        println!("  {} -> {}  {:?}", e.start, e.end, e.command);
    }
    Ok(())
}

fn cmd_assets(game: &Path) -> Result<()> {
    let vfs = daysengine::install::vfs::Vfs::mount(game)?;
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

/// Reports the engine's own settings, or prints a file to start from.
fn cmd_settings(template: bool) {
    use daysengine::install::engine::{self, Settings};
    if template {
        print!("{}", engine::template());
        return;
    }
    // Loaded first: a first run has no file, and loading is what writes one.
    let settings = Settings::load();
    match Settings::path() {
        Some(path) if path.is_file() => println!("{}", path.display()),
        Some(path) => println!("{} (absent; these are the defaults)", path.display()),
        None => println!("(cannot find this binary's own directory)"),
    }
    println!("  [Video] Scaler       = {:?}", settings.video_scaler);
    println!("  [UI]    PixelPerfect = {}", settings.pixel_perfect());
    if settings.pixel_perfect() {
        println!("          Scaler         (unused: nothing is resampled)");
    } else {
        println!("          Scaler       = {:?}", settings.ui_scaler);
    }
    println!(
        "\nA file of the defaults is written beside the binary on its first run. \
         Print one with: days settings --template > {}",
        engine::FILE
    );
}

fn cmd_media(
    game: &Path,
    path: &str,
    dump_frame: Option<&Path>,
    at_size: Option<&str>,
) -> Result<()> {
    let vfs = daysengine::install::vfs::Vfs::mount(game)?;
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
        if let Some(size) = at_size {
            // The engine's own settings, so this measures what the player gets
            // rather than what the defaults would give — the filter chains and
            // the grain included, since they are most of what a frame costs
            // now and this is where that is measured.
            let settings = daysengine::install::engine::Settings::load();
            decoder.set_scaler(settings.video_scaler)?;
            decoder.set_filters(&settings.video_filters);
            decoder.set_post_filters(&settings.video_filters_after);
            decoder.set_grain(settings.video_grain);
            let (w, h) = parse_size(size)?;
            decoder.set_output_size(w, h)?;
            println!("decoding at {w}x{h} with {:?}", settings.video_scaler);
            let (before, after) = decoder.filters();
            println!("  before the scale: {before:?}");
            println!(
                "  after the scale:  {after:?}, grain {}",
                settings.video_grain
            );
        }
        let mut frames = 0usize;
        let mut last = 0.0;
        let mut first: Option<daysengine::media::VideoFrame> = None;
        let started = std::time::Instant::now();
        while let Some(frame) = decoder.next_frame()? {
            last = frame.timestamp;
            if first.is_none() {
                first = Some(frame);
            }
            frames += 1;
        }
        let took = started.elapsed();
        println!(
            "{frames} frames, last pts {last:.3}s ({:.2} fps average)",
            if last > 0.0 {
                (frames - 1) as f64 / last
            } else {
                0.0
            }
        );
        if frames > 0 {
            // What the player has to fit into one 24 fps frame, which is 41ms.
            let each = took / frames as u32;
            println!(
                "decoded and scaled in {took:.2?}, {each:.2?} a frame ({:.0} fps)",
                frames as f64 / took.as_secs_f64()
            );
        }
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

/// Parses a `WxH` size.
fn parse_size(text: &str) -> Result<(u32, u32)> {
    let (w, h) = text
        .split_once(['x', 'X'])
        .with_context(|| format!("{text} is not a WxH size"))?;
    Ok((w.trim().parse()?, h.trim().parse()?))
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
    let vfs = daysengine::install::vfs::Vfs::mount(game)?;
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

/// Blends an RGBA layer over an opaque RGBA frame, both the same width.
///
/// The layer may be shorter than the frame — the control bar is an 800x75 strip
/// over an 800x452 picture — in which case the rest of the frame is untouched.
fn blend_over(frame: &mut [u8], width: usize, height: usize, layer: &days_ui::Image) {
    blend_at(frame, width, height, layer, (0, 0), 255);
}

/// As [`blend_over`], at an offset and through an alpha of its own.
///
/// The control bar's REPLAYMODE indicator needs both: it goes at y = 80, below
/// the strip, and it carries the transparency the bar's ten cells set rather
/// than the bar's fade.
fn blend_at(
    frame: &mut [u8],
    width: usize,
    height: usize,
    layer: &days_ui::Image,
    at: (i64, i64),
    alpha: u8,
) {
    for y in 0..layer.height as usize {
        let Ok(dy) = usize::try_from(at.1 + y as i64) else {
            continue;
        };
        if dy >= height {
            continue;
        }
        for x in 0..layer.width as usize {
            let Ok(dx) = usize::try_from(at.0 + x as i64) else {
                continue;
            };
            if dx >= width {
                continue;
            }
            let s = (y * layer.width as usize + x) * 4;
            let d = (dy * width + dx) * 4;
            let a = u32::from(layer.rgba[s + 3]) * u32::from(alpha) / 255;
            if a == 0 {
                continue;
            }
            for c in 0..3 {
                let src = u32::from(layer.rgba[s + c]);
                let under = u32::from(frame[d + c]);
                frame[d + c] = ((src * a + under * (255 - a)) / 255) as u8;
            }
        }
    }
}

fn cmd_render(
    game: &Path,
    name: &str,
    at: &[String],
    out: &Path,
    bar: bool,
    following_record: bool,
) -> Result<()> {
    use days_script::Frame;

    let vfs = daysengine::install::vfs::Vfs::mount(game)?;
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

    // Two answers only the install can give: the choice box's axis, and
    // whether dialogue wraps and at what pitch.
    let film = film_ini(&vfs);
    let stacked =
        daysengine::ui::select::Layout::from_ini(&film) == daysengine::ui::select::Layout::Stacked;
    let english = film.get_bool("UseEnglish").unwrap_or(false);
    let left_arrangement = film.get_bool("LeftArrangement").unwrap_or(false);

    // The control bar, with the pointer parked inside the strip and the ramp
    // settled, so `--bar` shows the dropped-down state.
    let control = if bar {
        let dll = system_menu_dll(game)?;
        match daysengine::ui::bar::Bar::load(&vfs, &dll, daysengine::ui::screen::Resolution::Wide) {
            Ok(mut strip) => {
                strip.point_at(Some((0, 0)), 0, false);
                strip.point_at(Some((0, 0)), daysengine::ui::bar::FADE_IN_MS + 1, false);
                Some(strip)
            }
            Err(err) => {
                eprintln!("the control bar is unavailable: {err}");
                None
            }
        }
    } else {
        None
    };
    // The gauge reads the player's own save, as playback does: a bar drawn with
    // no counters has nothing to put in the channel.
    let bar_state = daysengine::ui::bar::State {
        following_record,
        gauge: slot_feeling(game)
            .map(|(_, _, _, store)| daysengine::install::feeling::gauge(&store)),
        ..daysengine::ui::bar::State::from_config(&daysengine::install::config::Config::load(game))
    };

    const W: usize = 800;
    const H: usize = 452;

    for target in targets {
        stage.seek_to(target, &vfs, &mixer)?;
        let visual = stage.visual_at(target);
        let described = format!(
            "movie={} still={} text={:?} fade={:?} select={:?}",
            visual
                .movie_id
                .map_or("-".to_string(), |(clip, index)| format!("{clip}#{index}")),
            visual.still.map(|s| s.path.as_str()).unwrap_or("-"),
            visual.text.map(|(s, t)| format!("{s}: {t}")),
            visual.fade,
            visual
                .select
                .map(|w| format!("{:?}/{:?} {}..{}", w.a, w.b, w.start, w.end)),
        );
        let mut rgba = daysengine::playback::compose::frame_rgba_with(
            &visual,
            &font,
            W,
            H,
            stacked,
            english,
            left_arrangement,
        );

        if let Some(strip) = &control {
            // Blended over the frame, which is the check that matters: the
            // strip is RGBA and the engine draws it over the picture. Had the
            // layer been flattened onto black first, this is where a black band
            // would show up.
            let layer = strip.compose_faded(None, bar_state, 0);
            blend_over(&mut rgba, W, H, &layer);
            // Not part of the strip: it sits below it, on the picture.
            if let Some(sign) = strip.indicator(bar_state) {
                blend_at(
                    &mut rgba,
                    W,
                    H,
                    &sign.art,
                    (sign.dst.0, sign.dst.1),
                    sign.alpha,
                );
            }
        }

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

/// Reads `FILMENGINE.INI` out of the packs, or an empty one with a warning.
fn film_ini(vfs: &daysengine::install::vfs::Vfs) -> daysengine::Ini {
    match vfs.read_path("Ini/FILMENGINE.INI") {
        Ok(bytes) => daysengine::Ini::parse_bytes(&bytes),
        Err(err) => {
            log::warn!("reading Ini/FILMENGINE.INI: {err}");
            daysengine::Ini::parse("")
        }
    }
}

fn cmd_bar(game: &Path, args: &BarArgs) -> Result<()> {
    use daysengine::ui::bar::{self, Act, Bar, State};
    use daysengine::ui::screen::Resolution;

    let vfs = daysengine::install::vfs::Vfs::mount(game)?;
    let dll = system_menu_dll(game)?;
    let resolution = Resolution::from_name(&args.resolution)
        .with_context(|| format!("unknown resolution {}", args.resolution))?;
    let bar = Bar::load(&vfs, &dll, resolution)?;

    let config = daysengine::install::config::Config::load(game);
    let feeling = match &args.feeling {
        Some(pair) => {
            let (a, b) = pair
                .split_once(',')
                .with_context(|| format!("--feeling wants 001,002, got {pair}"))?;
            Some((a.trim().parse()?, b.trim().parse()?))
        }
        None => {
            slot_feeling(game).map(|(_, _, _, store)| daysengine::install::feeling::gauge(&store))
        }
    };
    let state = State {
        auto: args.auto,
        gauge: feeling,
        gauge_raised: args.gauge,
        paused: args.paused,
        replay: args.replay,
        message: args.message,
        skippable: !args.no_script && State::from_config(&config).skippable,
        following_record: args.following_record,
        ..State::default()
    };
    // The two move together in the engine, and the rate is what decides whether
    // the readout is drawn, so a `--speed` that left it behind would hide it.
    let mut state = state;
    state.set_speed(args.speed.min(bar::SPEEDS.len() - 1));

    // The level is the bar's own, and the only thing that moves it is a press
    // on one of the ten cells -- so that is how it is set here too.
    let mut bar = bar;
    if let Some(want) = args.transparency {
        match (bar::indicator::FIRST_WIDGET..)
            .take(bar::indicator::CELLS)
            .find(|w| bar::indicator::level_for(*w) == Some(want))
        {
            Some(widget) => {
                if bar.press(widget, state, 0) == Act::None {
                    anyhow::bail!(
                        "the cells are dead without --following-record, so the press \
                         that sets the transparency is swallowed"
                    );
                }
            }
            None => anyhow::bail!("no cell sets a transparency of {want}; level 1 is unreachable"),
        }
    }

    let (w, h) = bar.screen().size();
    println!(
        "{} at {} — {w}x{h}, scale {:.2}, {} widgets ({}/{} boxes matched the table)",
        bar::PATH,
        resolution.name(),
        bar.screen().scale(),
        bar::WIDGETS,
        bar.screen().atlas().matched,
        bar::WIDGETS,
    );
    println!(
        "state: auto {} paused {} replay {} message {} skippable {} \
         following-record {} speed x{}",
        state.auto,
        state.paused,
        state.replay,
        state.message,
        state.skippable,
        state.following_record,
        bar::SPEEDS[state.speed],
    );
    // The rate retimes the audio as well as the picture, and past the
    // threshold the streams keep running unheard. Worth printing, because it
    // is the difference between "fast-forward is silent" as a bug and as the
    // behaviour `FUN_004433d0` actually has.
    println!(
        "audio: resampled x{} and {}",
        bar::SPEEDS[state.speed],
        if bar::SPEEDS[state.speed] > daysengine::playback::mixer::MUTE_ABOVE {
            "muted, above the 4.0 threshold"
        } else {
            "heard"
        }
    );
    println!("fade in {}ms, out {}ms", bar::FADE_IN_MS, bar::FADE_OUT_MS);

    // The right-hand box: a ten-cell slider whose knob is the other sprite the
    // bar sizes itself, and the indicator it sets the transparency of, which is
    // not on the strip at all.
    {
        let level = bar.transparency();
        println!(
            "replay indicator: {}, transparency {level} of 10 (alpha {})",
            if state.following_record {
                "on the picture, slider live"
            } else {
                "off, slider dead"
            },
            bar::indicator::alpha(level),
        );
        match bar::indicator::knob(level) {
            Some((src, dst)) => println!(
                "  knob   sheet ({:.0},{:.0}) {:.0}x{:.0} -> strip ({:.1},{:.1}) {:.0}x{:.0}",
                src.x, src.y, src.w, src.h, dst.x, dst.y, dst.w, dst.h,
            ),
            None => println!("  knob   level {level} has no case in FUN_10027030"),
        }
    }

    // The gauge is the one thing on the strip that is not a chip record: its
    // three pieces are cut from the sheet at sizes worked out from the two
    // counters, so print the cut rather than a record number.
    match state.gauge {
        None => println!("gauge: no save to read the counters from"),
        Some((first, second)) => {
            let (lead, _) = bar::gauge::leads(first, second);
            println!(
                "gauge: {} {first}, {} {second} — lead {lead:+}px to the {}, {}",
                daysengine::install::feeling::FIRST,
                daysengine::install::feeling::SECOND,
                if lead >= 0.0 { "first" } else { "second" },
                if state.gauge_raised {
                    "raised, so it draws even with the bar faded out"
                } else {
                    "drawn with the bar"
                },
            );
            let p = bar::gauge::pieces(first, second);
            for (name, piece) in [("second", p.second), ("first", p.first), ("level", p.level)] {
                match piece {
                    None => println!("  {name:6} down"),
                    Some(c) => println!(
                        "  {name:6} sheet ({:.1},{:.1}) {:.0}x{:.0} -> strip ({:.1},{:.1}) {:.0}x{:.0}",
                        c.src.x, c.src.y, c.src.w, c.src.h, c.dst.x, c.dst.y, c.dst.w, c.dst.h,
                    ),
                }
            }
        }
    }

    println!("  wgt  region  dst                live   caption  action");
    for widget in 0..bar::WIDGETS {
        let rect = bar.screen().atlas().widgets[widget].dst;
        let act = bar::action(widget, state, false);
        let shown = match act {
            Act::None => "-".to_string(),
            Act::ToggleAuto => "toggle the auto flag".to_string(),
            Act::TogglePause => "toggle pause".to_string(),
            Act::Seek(code) => match code {
                bar::Seek::RESTART => "seek: restart this script".to_string(),
                bar::Seek::END_OF_SCRIPT => "seek: end of script".to_string(),
                bar::Seek::SKIP => "seek: to this script's choice, else its end".to_string(),
                other => format!("seek code {}", other.0),
            },
            Act::Speed(i) => format!("speed x{}", bar::SPEEDS[i]),
            Act::Menu(m) => format!("open menu {}", m.0),
            Act::Leave => "leave playback".to_string(),
            Act::Transparency(n) => format!("set the replay indicator to {n} of 10"),
        };
        println!(
            "  {widget:3}  {:6}  ({:4},{:3}) {:3}x{:<3}  {:5}  {:>7}  {shown}",
            widget + 1,
            rect.x,
            rect.y,
            rect.width,
            rect.height,
            bar::enabled(widget, state),
            bar::caption(widget)
                .map(|c| c.to_string())
                .unwrap_or_else(|| "-".to_string()),
        );
    }
    if state.auto {
        println!(
            "widget 0 is on animation record {} after {}ms",
            bar::auto_frame(args.elapsed, state.speed),
            args.elapsed
        );
    }
    // The bar is a drop-down: it is on screen only while the pointer is inside
    // the strip, and it ramps in over 300ms and out over 1000ms. Drive that
    // here so what gets drawn is what the engine would draw.
    let at = if args.pointer.eq_ignore_ascii_case("off") {
        None
    } else {
        let (x, y) = args
            .pointer
            .split_once(',')
            .with_context(|| format!("--pointer wants X,Y or `off`, got {}", args.pointer))?;
        Some((x.trim().parse::<u32>()?, y.trim().parse::<u32>()?))
    };
    // Settling takes one update to start the ramp and one past its end to
    // finish it, exactly as the original's two calls per frame do.
    let settled = args
        .after
        .unwrap_or(bar::FADE_IN_MS.max(bar::FADE_OUT_MS) + 1);
    bar.point_at(at, 0, state.gauge_raised);
    let hovered = bar.point_at(at, settled, state.gauge_raised);
    println!(
        "pointer {} — the bar {}, alpha {}",
        match at {
            Some((x, y)) => format!("at ({x}, {y}) in the strip"),
            None => "off the strip".to_string(),
        },
        if bar.fade().drawn() {
            "is drawn"
        } else {
            "is not drawn"
        },
        bar.fade().alpha(),
    );
    let hovered = args.hover.or(hovered);
    match hovered {
        Some(widget) => println!("hovering widget {widget}"),
        None => println!("no widget hovered"),
    }

    if let Some(out) = &args.out {
        let mut image = bar.compose_faded(hovered, state, args.elapsed);
        // The REPLAYMODE indicator is below the strip, so the PNG grows to hold
        // it rather than clipping the thing the slider controls.
        if let Some(sign) = bar.indicator(state) {
            let height = (sign.dst.1.max(0) as u32 + sign.dst.3).max(image.height);
            let mut taller = days_ui::Image::empty(image.width, height);
            taller.blit_scaled(
                &image,
                (0, 0, image.width, image.height),
                (0, 0, image.width, image.height),
            );
            // Through the indicator's own alpha, onto transparency, so the PNG
            // stays a layer rather than gaining a black band under the strip.
            let mut art = sign.art;
            for px in art.rgba.as_chunks_mut::<4>().0 {
                px[3] = (u32::from(px[3]) * u32::from(sign.alpha) / 255) as u8;
            }
            taller.blit_scaled(
                &art,
                (0, 0, art.width, art.height),
                (sign.dst.0, sign.dst.1, sign.dst.2, sign.dst.3),
            );
            image = taller;
        }
        write_png(out, &image.rgba, image.width, image.height)?;
        println!("wrote {}", out.display());
    }
    Ok(())
}

fn cmd_select(game: &Path, args: &SelectArgs) -> Result<()> {
    use days_script::Frame;
    use daysengine::ui::screen::Resolution;
    use daysengine::ui::select::{Choice, Metrics, Select};

    let vfs = daysengine::install::vfs::Vfs::mount(game)?;
    let film = film_ini(&vfs);
    let resolution = Resolution::from_name(&args.resolution)
        .with_context(|| format!("unknown resolution {}", args.resolution))?;

    let choice = Choice::new(&args.label, args.label2.as_deref(), Frame(0), Frame(1));
    let select = Select::load(&vfs, &film, choice.count(), resolution)?;
    let metrics = Metrics::from_ini(&film, choice.count());

    println!(
        "{} choice(s) at {} — {:?} layout, map {}",
        choice.count(),
        resolution.name(),
        select.layout,
        match select.map_size() {
            Some((w, h)) => format!("{} ({w}x{h})", select.path),
            None => format!("{} is absent; splitting the screen", select.path),
        }
    );
    if let Some(bounds) = select.bounds() {
        for (i, r) in bounds.iter().enumerate() {
            println!(
                "  box {i}  ({:4},{:4}) {:4}x{:<4}",
                r.x, r.y, r.width, r.height
            );
        }
    }
    println!(
        "labels wrap at {} characters, advance {}, word wrap {}",
        metrics.wrap, metrics.advance, metrics.word_wrap
    );
    for (i, label) in choice.labels.iter().enumerate() {
        for line in metrics.lines(label) {
            println!("  label {i}: {line:?}");
        }
    }

    for point in &args.at {
        let (x, y) = point
            .split_once(',')
            .with_context(|| format!("--at wants X,Y, got {point}"))?;
        let x: f64 = x.trim().parse().context("bad X")?;
        let y: f64 = y.trim().parse().context("bad Y")?;
        match select.hit(x, y) {
            Some(index) => println!("({x}, {y}) hits box {index}: {}", choice.labels[index]),
            None => println!("({x}, {y}) hits nothing"),
        }
    }
    Ok(())
}

fn cmd_ui(game: &Path, args: &UiArgs) -> Result<()> {
    use daysengine::ui::screen::{Resolution, Screen, WidgetState};

    let vfs = daysengine::install::vfs::Vfs::mount(game)?;
    let dll = system_menu_dll(game)?;
    let resolution = Resolution::from_name(&args.resolution)
        .with_context(|| format!("unknown resolution {}", args.resolution))?;
    let mut screen =
        Screen::load_with_base(&vfs, &dll, &args.screen, args.base.as_deref(), resolution)?;
    // Composite at a window's size rather than the hit map's, the way the
    // player's does. This is the pass a change of `[UI] Scaler` shows up in.
    if let Some(size) = &args.at_size {
        let (w, h) = parse_size(size)?;
        screen.fit_to(w, h);
    }

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
        let backdrop = backdrop.as_ref().map(|b| screen.to_display(b));
        let image = screen.compose_over(backdrop.as_ref(), &states);
        write_png(path, &image.rgba, image.width, image.height)?;
        println!(
            "wrote {} at {}x{}",
            path.display(),
            image.width,
            image.height
        );
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
    let vfs = daysengine::install::vfs::Vfs::mount(game)?;
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
fn load_flags(
    game: &Path,
    vfs: &daysengine::install::vfs::Vfs,
) -> daysengine::install::save::FlagStore {
    let film = match vfs.read_path("Ini/FILMENGINE.INI") {
        Ok(bytes) => daysengine::Ini::parse_bytes(&bytes),
        Err(err) => {
            log::warn!("reading Ini/FILMENGINE.INI: {err}");
            daysengine::Ini::parse("")
        }
    };
    daysengine::install::save::load_flags(game, &film)
}

/// The backdrop the title screen would be drawn over.
///
/// Shared by `menu` and `save`, and the reason `[BaseFile]` is read here rather
/// than passed in: the fresh-install picture is one of the four answers.
fn chosen_backdrop(
    vfs: &daysengine::install::vfs::Vfs,
    list: &ending::EndingList,
    flags: &daysengine::install::save::FlagStore,
) -> ending::Backdrop {
    let start = start_script_ini(vfs);
    ending::title_backdrop(list, flags, start.get("BaseFile").unwrap_or_default())
}

/// `STARTSCRIPT.INI`, which both the title art and the backdrop are read from.
fn start_script_ini(vfs: &daysengine::install::vfs::Vfs) -> daysengine::Ini {
    match vfs.read_path("Ini/STARTSCRIPT.INI") {
        Ok(bytes) => daysengine::Ini::parse_bytes(&bytes),
        Err(err) => {
            log::warn!("reading Ini/STARTSCRIPT.INI: {err}");
            daysengine::Ini::parse("")
        }
    }
}

/// Prints what the save data says the player has unlocked.
/// Reads every save file the install has, writes it back, and compares bytes.
///
/// The strongest check there is on the encoders: the game's files are the
/// specification, so reproducing them exactly means the writer agrees with
/// `FUN_004350b0` and `FUN_00433340` on every varint length, every string
/// cipher index and every record order — not just on what the reader happens
/// to accept.
fn cmd_save_roundtrip(game: &Path) -> Result<()> {
    use daysengine::install::save::{flag_path, slot_path, FlagStore, Slot};

    let vfs = daysengine::install::vfs::Vfs::mount(game)?;
    let film = film_ini(&vfs);
    let mut checked = 0;
    let mut differed = 0;

    let path = flag_path(game, &film);
    match std::fs::read(&path) {
        Ok(bytes) => {
            let store = FlagStore::parse(&bytes)?;
            let back = store.to_bytes();
            checked += 1;
            if back == bytes {
                println!(
                    "{:<28} {:>7} bytes, {} flags  ok",
                    "GlobalFlag.DAT",
                    bytes.len(),
                    store.len()
                );
            } else {
                differed += 1;
                println!(
                    "{:<28} DIFFERS: {}",
                    "GlobalFlag.DAT",
                    first_difference(&bytes, &back)
                );
            }
        }
        Err(err) => println!("{}: {err}", path.display()),
    }

    for n in 0..100 {
        let path = slot_path(game, &film, n);
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        let name = path
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        checked += 1;
        match Slot::parse(&bytes) {
            Ok(slot) => {
                let back = slot.to_bytes();
                if back == bytes {
                    println!(
                        "{name:<28} {:>7} bytes, {} story points, {} choices  ok",
                        bytes.len(),
                        slot.marks.len(),
                        slot.choices.len()
                    );
                } else {
                    differed += 1;
                    println!("{name:<28} DIFFERS: {}", first_difference(&bytes, &back));
                }
            }
            Err(err) => {
                differed += 1;
                println!("{name:<28} will not decode: {err}");
            }
        }
    }
    println!();
    println!("{checked} files, {differed} that do not come back identical");

    // The same again, but through the engine's own model of a slot rather
    // than through the decoder alone: a slot the player saves has been round
    // tripped through `Progress`, and the original game still has to read it.
    let mut through = 0;
    let mut lost = 0;
    if let Ok(dll) = std::fs::read(game.join("RouteProcSDHQ.dll")) {
        let global = load_flags(game, &vfs);
        if let Ok(mut progress) = daysengine::install::progress::Progress::load(&vfs, &dll, global)
        {
            for n in 0..100 {
                let Ok(bytes) = std::fs::read(slot_path(game, &film, n)) else {
                    continue;
                };
                let Ok(slot) = Slot::parse(&bytes) else {
                    continue;
                };
                through += 1;
                progress.from_slot(slot);
                if progress.to_slot().to_bytes() != bytes {
                    lost += 1;
                    log::warn!("slot {n} does not survive a pass through the engine");
                }
            }
        }
    }
    println!("{through} slots read into the engine and written back, {lost} that changed");
    Ok(())
}

/// Where two encodings first diverge, for a round-trip that failed.
fn first_difference(a: &[u8], b: &[u8]) -> String {
    match a.iter().zip(b).position(|(x, y)| x != y) {
        Some(at) => format!(
            "byte {at:#x}: the file has {:02x?}, we write {:02x?}",
            &a[at..(at + 8).min(a.len())],
            &b[at..(at + 8).min(b.len())]
        ),
        None => format!("the file is {} bytes, we write {}", a.len(), b.len()),
    }
}

/// Prints what one save slot holds.
fn cmd_save_slot(game: &Path, n: u32) -> Result<()> {
    use daysengine::install::save::{load_slot, slot_keys, slot_path, Value};

    let vfs = daysengine::install::vfs::Vfs::mount(game)?;
    let film = film_ini(&vfs);
    let flags = load_flags(game, &vfs);
    let path = slot_path(game, &film, n);

    let Some(slot) = load_slot(game, &film, n) else {
        println!("slot {n} ({}) is empty", path.display());
        return Ok(());
    };
    let (head, sub) = slot_keys(&film, n);
    println!("slot {n}  {}", path.display());
    println!(
        "  shown as   {}",
        flags
            .get(&head)
            .and_then(Value::as_str)
            .unwrap_or("(no line in the global store)")
    );
    println!(
        "  comment    {}",
        flags.get(&sub).and_then(Value::as_str).unwrap_or("")
    );
    println!(
        "  script     {}  (engine version {})",
        slot.script, slot.version
    );
    let int = |name: &str| slot.store.get(name).and_then(Value::as_int);
    match (int("ROUTE"), int("SCENE")) {
        (Some(r), Some(s)) => println!("  position   ROUTE {r} SCENE {s}"),
        _ => println!("  position   ROUTE/SCENE are not in the store"),
    }
    println!("  store      {} entries", slot.store.len());
    for name in ["001", "002", "000", "003", "004"] {
        if let Some(v) = int(name) {
            println!("      {name} = {v}");
        }
    }
    println!(
        "  {} story points, in the order they were reached:",
        slot.marks.len()
    );
    for mark in slot.in_order() {
        println!(
            "      {:>3}  {:<14} {:<6} {} entries",
            mark.order,
            mark.script,
            mark.story,
            mark.store.len()
        );
    }
    println!("  {} recorded choices:", slot.choices.len());
    for (script, choice) in &slot.choices {
        println!("      {script:<14} {choice}");
    }
    Ok(())
}

fn cmd_save(game: &Path, all: bool, grep: Option<&str>) -> Result<()> {
    use daysengine::install::save::Value;
    use daysengine::SaveState;

    let vfs = daysengine::install::vfs::Vfs::mount(game)?;
    let flags = load_flags(game, &vfs);
    let start = start_script_ini(&vfs);
    let save = SaveState::from_flags(&flags, &start);

    println!("{} flags", flags.len());
    println!();
    println!("what the title screen reads:");
    println!("  AllClear          {}", flags.flag("AllClear"));
    println!("  EndClear          {}", flags.flag("EndClear"));
    match flags.get("EndNo").and_then(Value::as_int) {
        Some(n) => println!("  EndNo             most recent ending is #{n}"),
        None => println!("  EndNo             (not set)"),
    }
    let list = ending::load_list(&vfs, &start);
    println!(
        "  [EndNN] flags     {} of {} endings seen",
        list.seen(&flags),
        list.max()
    );
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
    let chosen = chosen_backdrop(&vfs, &list, &flags);
    println!("and draws it over {} ({:?})", chosen.path, chosen.reason);
    if chosen.sets_all_clear {
        println!("  (the original would store the AllClear flag here)");
    }

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

/// Prints the player's settings the way the Option screen reads them.
fn cmd_config(game: &Path) -> Result<()> {
    use daysengine::install::config::{Channel, Config, Flag};

    let path = Config::path(game);
    let config = Config::load(game);
    println!("{}", path.display());
    println!();
    println!("volumes (level, then the attenuation the DLL's formula gives):");
    for channel in Channel::ALL {
        println!(
            "  {:<12} {:>2}/10   {:>6.1} dB   gain {:.3}",
            channel.key(),
            config.volume(channel),
            config.attenuation_db(channel),
            config.gain(channel),
        );
    }
    println!("  {:<12} {:>6.3}", "MasterVolume", config.master_volume());
    println!();
    println!("settings:");
    for flag in Flag::ALL {
        println!("  {:<12} {}", flag.key(), config.flag(flag));
    }
    println!();
    // The three the engine reads at startup, which is where the game reopens
    // in whatever the player left it in. `FUN_0040cbb0` reads all three;
    // `DisplayType` 0 is the 4:3 back buffer and 1 the wide one, `WindowMode`
    // 1 is the full-screen window style `FUN_0040db00` sets.
    println!("display:");
    let show = |key: &str, meaning: &str| {
        println!(
            "  {:<12} {:<6} {meaning}",
            key,
            config.get(key).unwrap_or("(absent)")
        );
    };
    let wide = config.get("DisplayType").is_none_or(|v| v.trim() != "0");
    show("DisplayType", if wide { "wide" } else { "4:3" });
    let full = config.get("WindowMode").is_some_and(|v| v.trim() == "1");
    show("WindowMode", if full { "full screen" } else { "windowed" });
    let note = config.get("TypeMiniNote").is_some_and(|v| v.trim() != "0");
    show(
        "TypeMiniNote",
        if note { "1024x576 art" } else { "1280x720 art" },
    );
    Ok(())
}

/// Prints the replay scene table recovered from the user's own menu DLL.
fn cmd_replay(game: &Path, only_unlocked: bool) -> Result<()> {
    use daysengine::ui::replay::{Scenes, HSCENE_PER_PAGE};

    let vfs = daysengine::install::vfs::Vfs::mount(game)?;
    let dll = system_menu_dll(game)?;
    let flags = load_flags(game, &vfs);
    let scenes = Scenes::recover(&dll)?;

    println!(
        "{} scenes over {} pages of {HSCENE_PER_PAGE}",
        scenes.len(),
        scenes.pages()
    );
    println!();
    let mut open = 0usize;
    for (index, scene) in scenes.iter().enumerate() {
        let unlocked = scenes.unlocked(index, &flags);
        if unlocked {
            open += 1;
        }
        if only_unlocked && !unlocked {
            continue;
        }
        println!(
            "  {index:>2}  page {} slot {:>2}  {}  {:<14} {}",
            index / HSCENE_PER_PAGE,
            index % HSCENE_PER_PAGE,
            if unlocked { "unlocked" } else { "locked  " },
            scene.flag,
            scene.first_script().unwrap_or("(no script recovered)"),
        );
        // The steps after the first, which is what a scene chains through when
        // it is played. See `daysengine::ui::replay` for the walk.
        if scene.scripts.len() > 1 {
            println!(
                "        then {}",
                scene.scripts[1..]
                    .iter()
                    .map(String::as_str)
                    .collect::<Vec<_>>()
                    .join(" -> ")
            );
        }
        print_branch("        ", scene.branch.as_ref(), &scene.scripts);
        for choice in &scene.choices {
            println!(
                "        version {:<14} {}  {}",
                choice.flag,
                if flags.flag(&choice.flag) {
                    "seen  "
                } else {
                    "unseen"
                },
                choice.scripts.first().map_or("(not recovered)", |s| s),
            );
            if choice.scripts.len() > 1 {
                println!(
                    "                                       then {}",
                    choice.scripts[1..]
                        .iter()
                        .map(String::as_str)
                        .collect::<Vec<_>>()
                        .join(" -> ")
                );
            }
            print_branch(
                "                                       ",
                choice.branch.as_ref(),
                &choice.scripts,
            );
        }
    }
    println!();
    println!("{open} of {} unlocked by this save", scenes.len());
    Ok(())
}

/// Prints the branch table a scene or one of its versions walks.
///
/// A row is the three next steps a step can lead to — the first for a player
/// who has answered no choice box, the other two for the two answers — and a
/// step past the end of the list is where the scene stops.
fn print_branch(indent: &str, branch: Option<&daysengine::ui::replay::Branch>, scripts: &[String]) {
    let Some(table) = branch else {
        return;
    };
    let name = |step: i32| match usize::try_from(step)
        .ok()
        .and_then(|step| scripts.get(step))
    {
        Some(script) => script.rsplit('/').next().unwrap_or(script).to_string(),
        None => "end".to_string(),
    };
    println!("{indent}branch  none / first / second");
    for (step, row) in table.iter().enumerate() {
        println!(
            "{indent}  {:<14} {}",
            name(step as i32),
            row.iter()
                .map(|next| name(*next))
                .collect::<Vec<_>>()
                .join(" / ")
        );
    }
}

/// Drives the menu state machine and reports where each event lands.
fn cmd_menu(game: &Path, args: &MenuArgs) -> Result<()> {
    use daysengine::install::config::Config;
    use daysengine::ui::menu::{Action, Menu, Mode, SaveState, Session};
    use daysengine::ui::options::{Dir, Display, Som};
    use daysengine::ui::replay::Scenes;
    use daysengine::ui::saveload::Kind;
    use daysengine::ui::screen::Resolution;

    let vfs = daysengine::install::vfs::Vfs::mount(game)?;
    let dll = system_menu_dll(game)?;
    let resolution = Resolution::from_name(&args.resolution)
        .with_context(|| format!("unknown resolution {}", args.resolution))?;
    // The player's real save decides this; the flags below only force things
    // on, so a fresh install can still be driven through every screen.
    // A forced-fresh run reads an empty store, so the backdrop below follows
    // the same pretence the widget tables do.
    let flags = if args.fresh {
        daysengine::install::save::FlagStore::default()
    } else {
        load_flags(game, &vfs)
    };
    let film = film_ini(&vfs);
    let english = film.get_bool("UseEnglish").unwrap_or(false);
    let mut save = SaveState::from_flags(&flags, &start_script_ini(&vfs));
    save.all_clear |= args.all_clear;
    save.cleared_first |= args.cleared;
    save.cleared_replay |= args.replay;

    // The settings are read but never written here: an inspection tool has no
    // business rewriting the player's own `Config.DAT`.
    let session = || Session {
        save,
        flags: flags.clone(),
        config: Config::load(game),
        scenes: Scenes::recover(&dll).unwrap_or_else(|err| {
            log::warn!("no replay scene table: {err}");
            Scenes::from_scenes(Vec::new())
        }),
        display: Display {
            wide: resolution != Resolution::Standard,
            full_screen: false,
        },
        som: Som::default(),
        slots: daysengine::ui::saveload::Slots::read(game, &film, &flags, english),
        english,
        text_input: film.get_bool("TextInput").unwrap_or(false),
    };

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
            match Menu::open(&vfs, &dll, mode, session(), resolution) {
                Ok(menu) => println!(
                    "  mode {:>2}  {name:<14} {stem:<34} ok, {} widgets",
                    mode.0,
                    menu.screen().widget_count()
                ),
                Err(err) => println!("  mode {:>2}  {name:<14} {stem:<34} {err}", mode.0),
            }
        }
        // The same screens again, opened the way the control bar opens them.
        // These are a separate entry, not a separate screen: the art is the
        // same and what differs is where Close goes, so both have to be walked.
        println!("opened over playback, as host +0xf8 does:");
        for (code, name, mode, kind) in [
            (4u32, "save", Mode::SAVELOAD, Kind::Save),
            (5, "load", Mode::SAVELOAD, Kind::Load),
            (2, "option", Mode::OPTION, Kind::Load),
        ] {
            match Menu::open_over_playback(&vfs, &dll, mode, kind, session(), resolution) {
                Ok(mut menu) => {
                    let leaving = menu.leave(&vfs, &dll);
                    println!(
                        "  code {code}  {name:<14} mode {:>2}  ok, {} widgets, leaving -> {:?}",
                        mode.0,
                        menu.screen().widget_count(),
                        leaving
                    );
                }
                Err(err) => println!("  code {code}  {name:<14} mode {:>2}  {err}", mode.0),
            }
        }
        return Ok(());
    }

    // `--from-bar` is the whole point of this branch: a screen opened over
    // playback is a different entry, and the entry is what decides where Close
    // goes. Without it every run here is title-rooted, which is exactly the
    // blind spot that let a bar-opened Close land on the title.
    let mut menu = match args.from_bar {
        None => Menu::open(&vfs, &dll, Mode::TITLE, session(), resolution)
            .context("opening the title screen")?,
        Some(code) => {
            let (mode, kind) = match code {
                4 => (Mode::SAVELOAD, Kind::Save),
                5 => (Mode::SAVELOAD, Kind::Load),
                // The Option screen has no save/load job to set.
                2 => (Mode::OPTION, Kind::Load),
                other => bail!(
                    "--from-bar {other} is not a menu the control bar can open;                      it passes host +0xf8 code 4 (save), 5 (load) or 2 (option).                      Code 3 has a case in setSystemInit, selecting DAT_1004ffc8,                      but which screen that is has not been recovered."
                ),
            };
            Menu::open_over_playback(&vfs, &dll, mode, kind, session(), resolution)
                .with_context(|| format!("opening menu mode {} over playback", mode.0))?
        }
    };
    // Composite and hit-test at a window's size rather than the hit map's, the
    // way the player's does. `at:X:Y` is then in that same space.
    if let Some(size) = &args.at_size {
        let (w, h) = parse_size(size)?;
        menu.set_output_size(w, h);
        let (w, h) = menu.screen().size();
        println!("driving at {w}x{h}");
    }
    println!(
        "mode {} ({}) — {} widgets, entry {:?}",
        menu.mode().0,
        menu.variant(),
        menu.screen().widget_count(),
        menu.entry()
    );
    // The title's backdrop comes from the same save data, so report it here:
    // the widget table and the picture are the two halves of "which title".
    // A bar-opened run has no title under it, so there is nothing to report.
    let chosen = chosen_backdrop(
        &vfs,
        &ending::load_list(&vfs, &start_script_ini(&vfs)),
        &flags,
    );
    if args.from_bar.is_none() {
        println!("backdrop {} ({:?})", chosen.path, chosen.reason);
    }

    for event in args
        .events
        .split(',')
        .map(str::trim)
        .filter(|e| !e.is_empty())
    {
        let (action, label) = match event {
            "down" => (menu.navigate(Dir::Down), "down".to_string()),
            "up" => (menu.navigate(Dir::Up), "up".to_string()),
            "left" => (menu.navigate(Dir::Left), "left".to_string()),
            "right" => (menu.navigate(Dir::Right), "right".to_string()),
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
                             esc, yes, left, right, at:X:Y or click:X:Y"
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
            Action::PlayReplay(run) => {
                println!(
                    "  (would replay {} script(s): {}{})",
                    run.scripts.len(),
                    run.scripts.join(", "),
                    match &run.branch {
                        Some(table) => format!("; branching over {} steps", table.len()),
                        None => String::new(),
                    }
                );
                break;
            }
            // The Option screen's Close flushes and then leaves like any
            // other screen, so report where leaving lands — that is the half
            // of this action that differs between the two entries.
            Action::SettingsSaved => {
                let leaving = menu.leave(&vfs, &dll)?;
                println!("  (would write Config.DAT) leaving -> {leaving:?}");
                if leaving == Action::Play {
                    println!("  (would resume playback)");
                    break;
                }
            }
            _ => {}
        }
    }

    // The save/load screen's rows carry text the composite draws itself rather
    // than cutting out of art. Print the lines, so what the screen shows is
    // checkable against the install without reading pixels.
    if menu.mode() == Mode::SAVELOAD {
        let slots = &menu.session().slots;
        println!(
            "  {:?} screen, page {} of {}, {} slots filled",
            menu.kind(),
            menu.page() + 1,
            daysengine::ui::saveload::PAGES,
            slots.len()
        );
        for (slot, line) in slots.page(menu.page()) {
            match line {
                Some(line) => println!(
                    "    slot {slot:>3}  {:<24} {:<6} {}",
                    line.when, line.chapter, line.comment
                ),
                None => println!("    slot {slot:>3}  (empty)"),
            }
        }
        match menu.rows().and_then(|rows| rows.tooltip.as_ref()) {
            Some(tip) => {
                let (x, y, w, h) = tip.panel.dst;
                println!(
                    "    expanded comment: panel {w:.0}x{h:.0} at {x:.1},{y:.1}, {} line(s)",
                    tip.lines.len()
                );
            }
            None => println!("    expanded comment: none"),
        }
    }

    if let Some(out) = &args.out {
        // With no override, draw what the save says: the same choice the
        // engine makes, so the PNG shows the title the player would see.
        let under = match &args.backdrop {
            Some(path) => Some(path.as_str()),
            None if menu.mode() == Mode::TITLE => Some(chosen.path.as_str()),
            None => None,
        };
        let backdrop = match under {
            Some(path) => match ending::load_image(&vfs, path) {
                Ok(image) => Some(image),
                Err(err) => {
                    log::warn!("loading backdrop {path}: {err}");
                    None
                }
            },
            None => None,
        };
        let backdrop = backdrop.as_ref().map(|b| menu.screen().to_display(b));
        let image = menu.compose(backdrop.as_ref());
        write_png(out, &image.rgba, image.width, image.height)?;
        println!("wrote {}", out.display());
    }
    Ok(())
}

/// Prints a dialog template out of the player's executable.
fn cmd_dialog(game: &Path, id: Option<&str>, out: Option<&Path>, text: &str) -> Result<()> {
    use daysengine::install::dialog::{comment_dialog, Class, Template};

    let vfs = daysengine::install::vfs::Vfs::mount(game)?;
    let english = film_ini(&vfs).get_bool("UseEnglish").unwrap_or(false);
    let id = match id {
        Some(text) => {
            let text = text.trim_start_matches("0x");
            u16::from_str_radix(text, 16).context("--id takes a hexadecimal resource id")?
        }
        None => comment_dialog(english),
    };

    let t = Template::from_game(game, id)?;
    println!("dialog {:#04x}  {:?}", t.id, t.title);
    println!(
        "  {} x {} dialog units{}",
        t.rect.cx,
        t.rect.cy,
        match &t.font {
            Some((points, face)) => format!(", {points}pt {face}"),
            None => String::new(),
        }
    );
    for item in &t.items {
        let class = match &item.class {
            Class::Named(n) => n.clone(),
            other => format!("{other:?}"),
        };
        println!(
            "  id {:#06x}  {class:<9} ({:>3},{:>3}) {:>3}x{:<3}  style {:08x}  {:?}",
            item.id, item.rect.x, item.rect.y, item.rect.cx, item.rect.cy, item.style, item.text
        );
    }

    if let Some(out) = out {
        use daysengine::ui::comment::{base_units, Comment};
        let exe = std::fs::read(game.join("SCHOOLDAYS HQ.exe"))?;
        let bytes = vfs
            .read_path("System/System/FONTDATA_ENG.DAT")
            .or_else(|_| vfs.read_path("System/System/FONTDATA.DAT"))?;
        let font = days_font::Font::parse(bytes)?;
        let mut dialog = Comment::open_id(&exe, t.id, english, text)?;
        dialog.end();
        let base = base_units(english);
        let image = dialog.compose_image(&font, base);
        println!(
            "  drawn at {}x{} pixels, base units {:?}",
            image.width, image.height, base
        );
        write_png(out, &image.rgba, image.width, image.height)?;
        println!("wrote {}", out.display());
    }
    Ok(())
}

/// Reads `RouteProcSDHQ.dll`, which owns every branching decision the game
/// makes.
fn route_dll(game: &Path) -> Result<Vec<u8>> {
    let path = game.join("RouteProcSDHQ.dll");
    std::fs::read(&path).with_context(|| {
        format!(
            "reading {} — the branch graph's script tables live in it",
            path.display()
        )
    })
}

/// Walks the branch graph from a script, the way playback chains through it.
///
/// This drives the same [`Progress`] the game does — `enter` to place the
/// player, `decide` when a choice settles, `advance` when a script ends — so
/// what it prints is what would be played.
fn cmd_route_play(game: &Path, from: &str, choices: &str, steps: usize) -> Result<()> {
    use daysengine::install::progress::Progress;

    let vfs = daysengine::install::vfs::Vfs::mount(game)?;
    let flags = load_flags(game, &vfs);
    let mut progress = Progress::load(&vfs, &route_dll(game)?, flags)?;
    let answers: Vec<i32> = choices
        .split(',')
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.trim().parse())
        .collect::<std::result::Result<_, _>>()
        .context("--choices takes a comma-separated list of numbers")?;

    if !progress.enter(from) {
        println!("{from} is in no route table, so nothing follows it");
        return Ok(());
    }
    let mut answers = answers.into_iter();
    let mut played = from.to_string();
    for step in 0..steps {
        let (route, scene) = progress.position();
        let ((first, second), raised) = progress.gauge();
        println!(
            "{step:>3}  ROUTE {route:>2} SCENE {scene:>3}  {played}   001={first} 002={second}{}",
            if raised { "  (gauge up)" } else { "" }
        );
        // A choice box settles before the script ends, so the answer is given
        // first and the transition taken after.
        progress.decide(answers.next().unwrap_or(-1), true);
        match progress.advance() {
            Some(next) => played = next,
            None => {
                println!("     the graph names nothing after this");
                return Ok(());
            }
        }
    }
    println!("     stopped after {steps} scripts");
    Ok(())
}

/// Prints the branch graph and the affection tables that drive it.
fn cmd_route(game: &Path, name: Option<&str>, list_scenes: bool, edges: bool) -> Result<()> {
    use days_route::{Machine, Routes};
    use daysengine::install::feeling::{self, Deltas, Thresholds, FIRST, SECOND};

    let vfs = daysengine::install::vfs::Vfs::mount(game)?;
    let dll = route_dll(game)?;
    let routes = Routes::recover(&dll)?;
    let machine = Machine::recover(&dll)?;

    let read = |path: &str| match vfs.read_path(path) {
        Ok(bytes) => bytes,
        Err(err) => {
            log::warn!("reading {path}: {err}");
            Vec::new()
        }
    };
    let deltas = Deltas::parse(&read("Ini/FeelingScript.ini"));
    let thresholds = Thresholds::parse(&read("Ini/StanderdScript.ini"));

    if let Some(name) = name {
        // Both spellings are in use: the tables store "00/00-00-A04" and the
        // feeling tables key on "00-00-A04".
        let full = if name.contains('/') {
            name.to_string()
        } else {
            format!("{}/{name}", &name[..2])
        };
        match routes.find(&full) {
            Some((route, scene)) => {
                println!("{full}  ROUTE {route} (0x{route:02x})  SCENE {scene}");
                let table = routes.route(route).unwrap_or_default();
                println!("  route {route} has {} scenes", table.len());
            }
            None => println!("{full} is in no route table"),
        }
        let credits = deltas.for_script(&full);
        if credits.is_empty() {
            println!("  credits nothing");
        } else {
            for (counter, amount) in &credits {
                let drawn = if counter == FIRST || counter == SECOND {
                    " (on the gauge)"
                } else if *amount == 0 {
                    " (filler)"
                } else {
                    ""
                };
                println!("  credits {counter} {amount:+}{drawn}");
            }
        }
        match thresholds.for_script(&full) {
            Some((counter, amount)) => {
                println!("  gated on {counter} > {amount}");
            }
            None => println!("  not gated"),
        }
        if let Some((route, scene)) = routes.find(&full) {
            if let Some(n) = machine.story(route, scene as u16) {
                println!("  marks story number {n} (SP{n:03})");
            }
            if let Some(step) = machine.step(route, scene as u16) {
                for line in transitions(&step) {
                    println!("  next: {line}");
                }
            }
            // What `_SetFeeling@8` credits here, which is its own switch and
            // not the branch graph's destination.
            if let Some(step) = machine.crediting(route, scene as u16) {
                for line in creditings(&step) {
                    println!("  credits on the way out: {line}");
                }
            }
        }
        return Ok(());
    }

    println!("{} routes, {} scenes", routes.len(), routes.iter().count());
    println!();
    for route in 0..routes.len() {
        let table = routes.route(route).unwrap_or_default();
        let first = table.first().map_or("(empty)", String::as_str);
        let last = table.last().map_or("(empty)", String::as_str);
        println!(
            "  ROUTE {route:>2} (0x{route:02x})  {:>3} scenes  chapter {}  {first} .. {last}",
            table.len(),
            machine
                .chapter(route)
                .map_or_else(|| "?".into(), |n| n.to_string())
        );
        if list_scenes || edges {
            for (scene, script) in table.iter().enumerate() {
                let credits = deltas.for_script(script);
                let moved: Vec<String> = credits
                    .iter()
                    .filter(|(_, a)| *a != 0)
                    .map(|(c, a)| format!("{c}{a:+}"))
                    .collect();
                let gate = thresholds
                    .for_script(script)
                    .map(|(c, a)| format!("  gated {c}>{a}"))
                    .unwrap_or_default();
                let story = machine
                    .story(route, scene as u16)
                    .map(|n| format!("  SP{n:03}"))
                    .unwrap_or_default();
                println!(
                    "      {scene:>3}  {script}{}{gate}{story}",
                    if moved.is_empty() {
                        String::new()
                    } else {
                        format!("  [{}]", moved.join(" "))
                    }
                );
                if edges {
                    match machine.step(route, scene as u16) {
                        Some(step) => {
                            for line in transitions(&step) {
                                println!("           -> {line}");
                            }
                        }
                        None => println!("           -> (no handler)"),
                    }
                }
            }
        }
    }

    if edges {
        check_edges(&routes, &machine);
    }

    println!();
    println!("affection counters: {}", deltas.names().join(", "));
    println!("  {FIRST} and {SECOND} are the two the gauge draws");

    // The counters are not in GlobalFlag.DAT: ROUTE, SCENE and the five of
    // them live in the per-save store, which host slots +0x08/+0x0c reach
    // through `engine + 0x40` while the global store hangs off `engine + 0x3c`.
    println!();
    match slot_feeling(game) {
        Some((slot, route, scene, store)) => {
            let (a, b) = feeling::gauge(&store);
            println!("{slot}: ROUTE {route}, SCENE {scene}");
            for name in deltas.names() {
                let drawn = if name == FIRST || name == SECOND {
                    "  (gauge)"
                } else {
                    ""
                };
                println!("  {name} = {}{drawn}", store.int(name));
            }
            println!(
                "  the branch test (get(\"{SECOND}\") < get(\"{FIRST}\")) takes the {} arm: {}",
                if feeling::first_leads(&store) {
                    "if"
                } else {
                    "else"
                },
                if a == b {
                    "the two are level, and the test is a strict <".to_string()
                } else if feeling::first_leads(&store) {
                    format!("{FIRST} leads by {}", a - b)
                } else {
                    format!("{SECOND} leads by {}", b - a)
                }
            );
        }
        None => println!("no save slot to read the counters from"),
    }
    Ok(())
}

/// One line per path through a scene's recovered decision tree.
///
/// The tree's forks are the questions the handler asks the host, so a line
/// reads as the conditions that hold followed by what is played.
fn transitions(step: &days_route::Transition) -> Vec<String> {
    use days_route::Next;
    paths(step, &|n| match n {
        Next::Scene { route, scene } => format!("ROUTE {route} SCENE {scene}"),
        Next::Named { names, scene } => format!("SCENE {scene}, playing {}", names.join(" or ")),
        Next::Stop => "the route ends".into(),
        Next::Nothing => "nothing".into(),
    })
}

/// One line per path through a scene's recovered crediting tree.
fn creditings(step: &days_route::Crediting) -> Vec<String> {
    paths(step, &|c| match c {
        Some(script) => format!("the deltas of {script}"),
        None => "nothing".into(),
    })
}

/// One line per path through a decision tree, with `leaf` naming the result.
fn paths<L>(step: &days_route::Step<L>, leaf: &dyn Fn(&L) -> String) -> Vec<String> {
    use days_route::{Act, Cmp, Step, Term};

    fn term(t: &Term) -> String {
        match t {
            Term::Const(n) => n.to_string(),
            Term::Choice => "choice".into(),
            Term::Int(n) => format!("counter {n}"),
            Term::Flag(n) => format!("flag {n}"),
            Term::GlobalFlag(n) => format!("global flag {n}"),
            Term::Threshold(s) => format!("gate({s})"),
            Term::HostSlot(s) => format!("host+{s:#04x}"),
            Term::DllWord(a) => format!("dll word {a:#010x}"),
        }
    }
    fn cmp(c: Cmp, negated: bool) -> &'static str {
        match (c, negated) {
            (Cmp::Eq, false) | (Cmp::Ne, true) => "==",
            (Cmp::Ne, false) | (Cmp::Eq, true) => "!=",
            (Cmp::Lt, false) | (Cmp::Ge, true) => "<",
            (Cmp::Le, false) | (Cmp::Gt, true) => "<=",
            (Cmp::Gt, false) | (Cmp::Le, true) => ">",
            (Cmp::Ge, false) | (Cmp::Lt, true) => ">=",
        }
    }
    fn acts(a: &[Act]) -> String {
        let named: Vec<String> = a
            .iter()
            .filter_map(|x| match x {
                Act::SetInt(n, v) => Some(format!("{n}={v}")),
                Act::SetFlag(n, v) => Some(format!("flag {n}={}", *v as u8)),
                Act::SetGlobalFlag(n, v) => Some(format!("global flag {n}={}", *v as u8)),
                Act::Story(n) => Some(format!("SP{n:03}")),
                Act::ClearStory(n) => Some(format!("clear SP{n}")),
                Act::Ending(n) => Some(format!("register ending {n}")),
                Act::ClearRouteFlags => Some("clear the route's flags".into()),
                Act::Host(_) => None,
            })
            .collect();
        if named.is_empty() {
            String::new()
        } else {
            format!("  [{}]", named.join(", "))
        }
    }
    fn walk<L>(
        step: &Step<L>,
        leaf: &dyn Fn(&L) -> String,
        conds: &mut Vec<String>,
        out: &mut Vec<String>,
    ) {
        match step {
            Step::If {
                lhs,
                cmp: c,
                rhs,
                then,
                els,
            } => {
                let side = |negated| format!("{} {} {}", term(lhs), cmp(*c, negated), term(rhs));
                conds.push(side(false));
                walk(then, leaf, conds, out);
                conds.pop();
                conds.push(side(true));
                walk(els, leaf, conds, out);
                conds.pop();
            }
            Step::Do { acts: a, next: n } => {
                let when = if conds.is_empty() {
                    String::new()
                } else {
                    format!("when {}: ", conds.join(" and "))
                };
                out.push(format!("{when}{}{}", leaf(n), acts(a)));
            }
            Step::Unrecovered(why) => out.push(format!("not recovered: {why}")),
        }
    }
    let mut out = Vec::new();
    walk(step, leaf, &mut Vec::new(), &mut out);
    out
}

/// Checks the recovered graph against the name tables it points into.
///
/// Three things have to hold if the recovery is right: every edge names a
/// scene that route's table actually has, every scene has an answer, and the
/// whole graph hangs together from the first scene of route 0 — which is where
/// `StartScript.ini` puts the player.
fn check_edges(routes: &days_route::Routes, machine: &days_route::Machine) {
    use days_route::{Next, Step, Transition};
    use std::collections::{HashSet, VecDeque};

    fn leaves<'a>(step: &'a Transition, out: &mut Vec<&'a Next>) {
        match step {
            Step::If { then, els, .. } => {
                leaves(then, out);
                leaves(els, out);
            }
            Step::Do { next, .. } => out.push(next),
            Step::Unrecovered(_) => {}
        }
    }

    let (mut scenes, mut answered, mut dangling, mut unrecovered) = (0, 0, 0, 0);
    let mut reachable: HashSet<(usize, u16)> = HashSet::new();
    let mut queue: VecDeque<(usize, u16)> = VecDeque::new();
    if routes.script(0, 0).is_some() {
        reachable.insert((0, 0));
        queue.push_back((0, 0));
    }

    for route in 0..routes.len() {
        let table = routes.route(route).unwrap_or_default();
        scenes += table.len();
        for scene in 0..table.len() {
            let Some(step) = machine.step(route, scene as u16) else {
                continue;
            };
            let mut out = Vec::new();
            leaves(&step, &mut out);
            if out.is_empty() {
                unrecovered += 1;
                continue;
            }
            if out.iter().any(|n| !matches!(n, Next::Nothing)) {
                answered += 1;
            }
            for n in out {
                if let Next::Scene { route: r, scene: s } = n {
                    if routes.script(*r as usize, *s as usize).is_none() {
                        dangling += 1;
                    }
                }
            }
        }
    }

    while let Some((route, scene)) = queue.pop_front() {
        let Some(step) = machine.step(route, scene) else {
            continue;
        };
        let mut out = Vec::new();
        leaves(&step, &mut out);
        for n in out {
            let to = match n {
                Next::Scene { route: r, scene: s } => (*r as usize, *s),
                Next::Named { scene: s, .. } => (route, *s),
                _ => continue,
            };
            if routes.script(to.0, to.1 as usize).is_some() && reachable.insert(to) {
                queue.push_back(to);
            }
        }
    }

    println!();
    println!("the recovered graph:");
    println!("  {scenes} scenes, {answered} with a transition");
    println!("  {unrecovered} scenes whose handler could not be decoded");
    println!("  {dangling} edges naming a scene no table has");
    println!(
        "  {} scenes reachable from ROUTE 0 SCENE 0, {} not",
        reachable.len(),
        scenes - reachable.len()
    );
    let stories = machine.stories().count();
    println!("  {stories} scenes mark a story number");

    // `_SetFeeling@8` works out the destination itself rather than being told
    // it, so the two exports agreeing is a check on both: for every scene and
    // every choice, what the crediting names must be the script the branch
    // graph moves to.
    struct Choice(i32);
    impl days_route::Context for Choice {
        fn choice(&self) -> i32 {
            self.0
        }
        fn int(&self, _: &str) -> i32 {
            0
        }
        fn flag(&self, _: &str) -> bool {
            false
        }
        fn global_flag(&self, _: &str) -> bool {
            false
        }
        fn threshold(&self, _: &str) -> bool {
            false
        }
    }
    fn undecoded<L>(step: &Step<L>) -> bool {
        match step {
            Step::If { then, els, .. } => undecoded(then) || undecoded(els),
            Step::Do { .. } => false,
            Step::Unrecovered(_) => true,
        }
    }

    let (mut credits, mut disagree, mut blind) = (0, 0, 0);
    for route in 0..routes.len() {
        for scene in 0..routes.route(route).map_or(0, <[String]>::len) {
            if machine
                .crediting(route, scene as u16)
                .is_none_or(|c| undecoded(&c))
            {
                blind += 1;
            }
            for choice in -1..4 {
                let cx = Choice(choice);
                let Some(script) = machine.credited(route, scene as u16, &cx) else {
                    continue;
                };
                credits += 1;
                let to = match machine.next(route, scene as u16, &cx) {
                    Some((_, Next::Scene { route: r, scene: s })) => {
                        routes.script(r as usize, s as usize)
                    }
                    _ => None,
                };
                if to != Some(script.as_str()) {
                    disagree += 1;
                    log::info!(
                        "route {route} scene {scene} choice {choice}: credits {script} but moves to {to:?}"
                    );
                }
            }
        }
    }
    // The nine that differ in the retail DLL are three scenes with two of
    // `SetFeeling`'s arms swapped — a bug in the game, reproduced rather than
    // corrected. A number other than nine here is a recovery problem.
    println!("  {credits} (scene, choice) pairs credit a script, {disagree} of them naming something the branch graph does not move to");
    println!("  {blind} scenes whose crediting could not be decoded");
}

/// The counters out of the first save slot that decodes.
///
/// A slot's own store is the whole of its state, so this is just the first
/// `Slot` the install has, read in full rather than hunted for by magic.
fn slot_feeling(game: &Path) -> Option<(String, i32, i32, days_save::FlagStore)> {
    let mut slots: Vec<_> = std::fs::read_dir(game.join("Save"))
        .ok()?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("SaveFile") && n.ends_with(".DAT"))
        })
        .collect();
    slots.sort();

    for path in slots {
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        let Ok(slot) = days_save::Slot::parse(&bytes) else {
            continue;
        };
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("save slot")
            .to_string();
        let (route, scene) = (slot.store.int("ROUTE"), slot.store.int("SCENE"));
        return Some((name, route, scene, slot.store));
    }
    None
}
