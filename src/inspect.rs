//! The inspection tools — `daysengine <subcommand>`, when the engine is asked
//! to report on an install rather than play it.
//!
//! Nothing here is needed to play; it exists so the archive and script formats
//! can be checked against real data rather than against our assumptions. It is
//! **the verification path**: a screen composed here can be diffed, and a claim
//! about a recovered format can be checked against the player's own files,
//! neither of which a window on a desktop allows.
//!
//! These were a second binary, `days`, until they were not. One program does
//! one job; two programs built from one library, differing only in which half
//! of it they call, is an extra artifact to ship and a second place for an
//! argument to be spelled differently. [`run`] is the whole entry point, and
//! `main` hands over to it when the first argument names a subcommand.

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use days_gpk::{Archive, Key};
use daysengine::install::binaries::{self, Binaries};
use daysengine::ui::ending;
use std::path::{Path, PathBuf};

#[derive(Parser)]
#[command(
    name = "daysengine",
    about = "Inspect a School Days HQ or Shiny Days installation"
)]
pub struct Cli {
    /// Game directory: the one holding `Packs`, the game executable and the
    /// menu and route modules.
    ///
    /// Defaults to wherever this binary lives, so dropping it into the game
    /// folder and running it works with no arguments.
    ///
    /// Global, so it reads the same either side of the subcommand:
    /// `daysengine --game DIR key` and `daysengine key --game DIR` are one
    /// command.
    #[arg(long, short = 'g', env = "DAYS_GAME_DIR", global = true)]
    game: Option<PathBuf>,

    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
pub enum Cmd {
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
    Render(RenderArgs),
    /// Composite a UI screen to PNG, without a display.
    ///
    /// The screen is drawn exactly as the game draws it: base art from the
    /// packs, widget sprites from the `_CHIP` sheet, positioned by the table in
    /// the user's own menu module. `--active` selects widgets by 1-based
    /// region ID, matching the `.CMAP`.
    Ui(UiArgs),
    /// Composite the backlog screen over one script's lines, without a display.
    ///
    /// The screen the control bar's third menu button raises. The lines are the
    /// script's own `[PrintText]` statements in the order the engine would have
    /// logged them, wrapped and laid out by the recovered rules, so this is how
    /// the drawing is checked against the player's own install.
    Backlog(BacklogArgs),
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
        /// Jump to this story point of `--slot`, the way the route map does,
        /// and report where it puts the player.
        #[arg(long, value_name = "SP")]
        story: Option<u32>,
        /// Read every save file, write it back, and check the bytes match.
        #[arg(long)]
        roundtrip: bool,
    },
    /// Print DaysEngine's own settings, and a template for the file they
    /// come from.
    ///
    /// These are the engine's choices, not the game's: which filter scales a
    /// movie frame, which scales the UI art. The game's own settings are
    /// `daysengine config`.
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
    Config {
        /// Read the file, write it back, and check the retail reader would
        /// still take it.
        #[arg(long)]
        roundtrip: bool,
    },
    /// Print the replay scene table recovered from the user's own menu module.
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
    /// Print the branch graph recovered from the user's own route module.
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
        /// After the walk, rewind this many parts the way the control bar's
        /// widget 2 does — `_GetBackScriptFile@12`, with each part's feeling
        /// deltas taken back off the counters.
        #[arg(long, default_value_t = 0, value_name = "N")]
        rewind: usize,
        /// The dress to walk in, as the dress-select screen commits it. Only
        /// Shiny Days has the mechanism; non-zero is the left dress, which is
        /// the one the `Z` recordings are of.
        #[arg(long, default_value_t = 0)]
        dress: u32,
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
pub struct BarArgs {
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
    /// Arm widget 2's latch, as its first press does: the second press then
    /// rewinds a part instead of restarting this one. The latch lives until
    /// the script clock passes frame 72 — see `bar::RESTART_LATCH_FRAMES`.
    #[arg(long)]
    latched: bool,
    /// Answer `_GetSuperSkipFlag@0` true whatever the player's `Config.DAT`
    /// says. That setting is what gates widget 4, the skip button — with it
    /// off the button is dead, and a dead widget is also silent, unhovered and
    /// captionless.
    #[arg(long)]
    super_skip: bool,
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
    /// over a bar that is otherwise faded away, plays its rise or its fall,
    /// and slides to `--feeling` over 1.5 seconds.
    #[arg(long)]
    gauge: bool,
    /// The pair the gauge was last settled at, as `001,002`. With `--gauge`
    /// this is where the ramp starts, so `--after` shows it part way to
    /// `--feeling`. Defaults to `--feeling`, which is a ramp that never moves.
    #[arg(long)]
    feeling_was: Option<String>,
    /// Composite the strip at this width in pixels rather than its art set's
    /// own, the way the player's window does when it is not that wide. This is
    /// the scaling path, so it is what a stair-stepped edge would show up in.
    #[arg(long)]
    at_width: Option<u32>,
    /// PNG to write the composited bar to. It has an alpha channel: the strip
    /// is a layer the engine draws over the frame, not a picture with a black
    /// bar in it.
    #[arg(long, short = 'o')]
    out: Option<PathBuf>,
}

#[derive(clap::Args)]
pub struct SelectArgs {
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
    /// Answer the box with this choice, or -1 to decline it, and report the
    /// fade that follows.
    #[arg(long)]
    pick: Option<i32>,
}

#[derive(clap::Args)]
pub struct RenderArgs {
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
    /// Answer `_ChangeSubtitle@4` true, so the `Ex01` episode title card
    /// goes over the start of the end roll. The route module only says
    /// this at one position, and only for a save that has flag 894.
    #[arg(long)]
    ending_card: bool,
    /// The rate the control bar's speed row is on, which is the only thing
    /// that decides how long the ending card stays up.
    #[arg(long, default_value_t = 1.0)]
    rate: f32,
}

#[derive(clap::Args)]
pub struct UiArgs {
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
    /// Report the Option screen's per-tab page tables, for a module that ships
    /// one hit map for the whole screen and lays its pages out in records.
    #[arg(long)]
    pages: bool,
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
pub struct BacklogArgs {
    /// Script whose `[PrintText]` lines fill the log, as `daysengine script` names
    /// it, e.g. "05-SH-A00".
    script: String,
    /// Resolution: standard, wide, note or full.
    #[arg(long, short = 'r', default_value = "wide")]
    resolution: String,
    /// Which entry the screen is on. The last one by default, which is where
    /// `FUN_100039a0` opens it.
    #[arg(long)]
    at: Option<usize>,
    /// Region IDs to draw in their active (hover/selected) state.
    #[arg(long = "active")]
    active: Vec<usize>,
    /// Composite at this window size, e.g. "1920x1080".
    #[arg(long, value_name = "WxH")]
    at_size: Option<String>,
    /// PNG to write.
    #[arg(long, short = 'o')]
    out: Option<PathBuf>,
}

#[derive(clap::Args)]
pub struct MenuArgs {
    /// Composite and hit-test at this window size, e.g. "1920x1080", the way
    /// the player's window does rather than at the hit map's own size.
    #[arg(long, value_name = "WxH")]
    at_size: Option<String>,
    /// Events to replay, comma separated: `down`, `up`, `left`, `right`,
    /// `enter`, `esc`, `at:X:Y` to point at a pixel, and `click:X:Y` to point
    /// and confirm. A slider is dragged with `press:X:Y`, then `drag:X:Y` for
    /// each step, then `release`. `tick` draws one frame and `tick:N` draws N,
    /// which is how the dress-select slide is stopped part-way; until one is
    /// used, every event lets whatever is moving finish first, the way the
    /// frames between two clicks do on the player's machine.
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
    /// Fill the backlog with one script's `[PrintText]` lines, as a run that
    /// had played it would have. Only the backlog reads them.
    #[arg(long, value_name = "SCRIPT")]
    lines_from: Option<String>,
    /// Open the screen the way the in-game control bar opens it, over live
    /// playback, instead of starting at the title. Takes `setSystemInit`'s own
    /// code, which is what the bar passes host `+0xf8`: 4 for the save screen,
    /// 5 for the load screen, 2 for the Option screen, 3 for the backlog.
    ///
    /// This is the entry that decides where Close goes, so it is the only way
    /// to check that from here. With it, `-e click:X:Y` on Close reports
    /// `Play` — leave the menus and resume — where a title-rooted run reports
    /// `Opened(Mode(2))`.
    #[arg(long, value_name = "CODE")]
    from_bar: Option<u32>,
    /// Open this menu mode directly instead of starting at the title.
    ///
    /// An inspection entry, not a route the game has: it says nothing about
    /// how the original reaches the mode. `--mode 9`, the dress-select screen,
    /// is reachable the way the original reaches it — the title screen's
    /// `START` — so this is a shortcut to it rather than the only way in; see
    /// [`daysengine::ui::dress`].
    #[arg(long, value_name = "N", conflicts_with = "from_bar")]
    mode: Option<i32>,
    /// Stand in a playthrough loaded from this slot, so the screens that ask
    /// what the *run* has done have something to answer from.
    ///
    /// The route map is the screen this matters to: its cells are charted from
    /// the global store but can only be picked where the run's own store
    /// carries the story point. Pair it with `--from-bar 5`, since the module
    /// asks that second question only when the control bar opened it.
    #[arg(long, value_name = "N")]
    run_from_slot: Option<u32>,
    /// Stand in a peripheral on this `Port number`, one-based, so the SOMCON
    /// tab can be inspected as it looks with a device held.
    ///
    /// There is no device here: this tool has no SDL and opens no controller.
    /// The tab's find button, its port buttons and its test all go out to the
    /// engine as requests (`Action::Som`) and the engine answers them, so
    /// without something standing in for one the tab can only ever be seen in
    /// its empty state — and the base art changes with the answer.
    #[arg(long, value_name = "N")]
    som_port: Option<usize>,
}

/// Runs whichever inspection subcommand the command line names.
///
/// Parses the whole command line itself rather than taking an already-parsed
/// `Cli`, so that clap owns every error message and `--help` a subcommand can
/// produce. `main` decides only *whether* to come here.
pub fn run() -> Result<()> {
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
    let exe = binaries::find_executable(&game)?;
    let key = Key::from_executable(&exe)
        .with_context(|| format!("recovering archive key from {}", exe.display()))?;

    match cli.cmd {
        Cmd::Key => {
            println!("{}", exe.display());
            println!("key recovered ({} packs readable)", packs(&game)?.len());
        }
        Cmd::List { pack, filter } => {
            // Through the VFS rather than straight off the pack files, so what
            // this lists is what the game can see: one row per logical path,
            // from whichever patch overlay won it.
            let vfs = daysengine::install::vfs::Vfs::mount(&game)?;
            let prefix = pack.map(|p| format!("{}/", p.to_ascii_lowercase()));
            let mut rows: Vec<_> = vfs
                .entries()
                .filter(|&(logical, h)| {
                    prefix.as_deref().is_none_or(|p| logical.starts_with(p))
                        && matches(&vfs.entry(h).name, filter.as_deref())
                })
                .collect();
            rows.sort_unstable_by_key(|&(logical, _)| logical);
            for (_, h) in &rows {
                let e = vfs.entry(*h);
                let layer = vfs.layer_of(*h);
                let from = match layer
                    .extension()
                    .is_some_and(|x| x.eq_ignore_ascii_case("gpk"))
                {
                    true => String::new(),
                    false => format!("  <- {}", pack_name(layer)),
                };
                println!(
                    "{:<12} {:>12} {:>12}  {}{from}",
                    vfs.pack_of(*h),
                    e.size,
                    e.decoded_len(),
                    e.name,
                );
            }
            eprintln!("{} entries", rows.len());
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
        Cmd::Render(args) => cmd_render(&game, &args)?,
        Cmd::Ui(args) => cmd_ui(&game, &args)?,
        Cmd::Backlog(args) => cmd_backlog(&game, &args)?,
        Cmd::Menu(args) => cmd_menu(&game, &args)?,
        Cmd::Save {
            all,
            grep,
            slot,
            story,
            roundtrip,
        } => {
            if roundtrip {
                cmd_save_roundtrip(&game)?
            } else if let (Some(n), Some(sp)) = (slot, story) {
                cmd_save_story(&game, n, sp)?
            } else if let Some(n) = slot {
                cmd_save_slot(&game, n, all)?
            } else {
                cmd_save(&game, all, grep.as_deref())?
            }
        }
        Cmd::Config { roundtrip } => cmd_config(&game, roundtrip)?,
        Cmd::Dialog { id, out, text } => cmd_dialog(&game, id.as_deref(), out.as_deref(), &text)?,
        Cmd::Replay { unlocked } => cmd_replay(&game, unlocked)?,
        Cmd::Route {
            name,
            scenes,
            edges,
            play,
            choices,
            steps,
            rewind,
            dress,
        } => match play {
            Some(from) => cmd_route_play(&game, &from, &choices, steps, rewind, dress)?,
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
                // `[PlayBgm]` names a pair, not a file: the engine opens
                // `<path>_int` and `<path>_loop`, and most tracks ship only
                // those. Checking the bare name reported every looping track
                // as missing. `[EndBGM]` really is a bare file — it is a
                // one-shot in sound slot 8 — so it stays a plain lookup.
                days_script::Command::PlayBgm { path } => {
                    checked += 1;
                    if vfs.resolve_bgm(path).is_none() {
                        *missing.entry(format!("{path}_loop.ogg")).or_default() += 1;
                    }
                    continue;
                }
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
    println!("  [Rumble] Strength   = {}%", settings.rumble_strength);
    println!("  [Input] every control, with what it is bound to:");
    let width = daysengine::install::binding::Action::ALL
        .into_iter()
        .map(|action| action.key().len())
        .max()
        .unwrap_or(0);
    for (action, triggers) in settings.bindings.all() {
        let list: Vec<String> = triggers.iter().map(ToString::to_string).collect();
        println!(
            "          {:<width$} = {}",
            action.key(),
            if list.is_empty() {
                "(unbound)".to_string()
            } else {
                list.join(", ")
            }
        );
    }
    println!(
        "\nA file of the defaults is written beside the binary on its first run, \
         and anything a later version adds is written into it — additively, \
         never over what you wrote. Print a fresh one with: \
         days settings --template > {}",
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

fn cmd_render(game: &Path, args: &RenderArgs) -> Result<()> {
    use days_script::Frame;

    let RenderArgs {
        name,
        at,
        out,
        bar,
        following_record,
        ending_card,
        rate,
    } = args;
    let (bar, following_record, ending_card, rate) = (*bar, *following_record, *ending_card, *rate);

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
    // The player's `MenVoice`, because a refused male line drives no mouth:
    // the option moves the picture as well as the sound.
    {
        use daysengine::install::config::{Config, Flag};
        stage.set_men_voice(Config::load(game).flag(Flag::MenVoice));
    }
    stage.set_ending_card(ending_card);
    stage.set_rate(rate);
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
                strip.point_at(Some((0, 0)), 0);
                strip.point_at(Some((0, 0)), daysengine::ui::bar::FADE_IN_MS + 1);
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
            "movie={} still={} card={} text={:?} fade={:?} select={:?}",
            visual
                .movie_id
                .map_or("-".to_string(), |(clip, index)| format!("{clip}#{index}")),
            visual.still.map(|s| s.path.as_str()).unwrap_or("-"),
            visual.card.map(|c| c.path.as_str()).unwrap_or("-"),
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
    let path = Binaries::discover(game)?
        .menu
        .with_context(|| no_module(game, &binaries::MENU_EXPORTS))?;
    std::fs::read(&path).with_context(|| {
        format!(
            "reading {} — the UI widget tables live in it",
            path.display()
        )
    })
}

/// The message for an install with no module exporting what we need.
fn no_module(game: &Path, exports: &[&str]) -> String {
    format!(
        "no .dll in {} exports {}",
        game.display(),
        exports.join(" and ")
    )
}

/// How long one `tick` of `daysengine menu` stands for.
///
/// The menus animate two ways. The dress-select slide counts host ticks, so a
/// `tick` is one of those whatever this says. The confirm popup's dim counts
/// milliseconds off `timeGetTime`, so it needs a length for the frame — and
/// this tool has no display to take one from. One presented frame at 60 Hz is
/// what the player's machine gives both of them. **It is this tool's
/// stand-in, not a recovered value**: the original is paced by whatever panel
/// is in front of it.
const TICK: std::time::Duration = std::time::Duration::from_micros(16_667);

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
    let mut bar = Bar::load(&vfs, &dll, resolution)?;
    // The scaling the SDL path does every frame: the strip is composited at
    // the width it is drawn at. See `Bar::set_output_width`.
    if let Some(width) = args.at_width {
        bar.set_output_width(width);
        let (w, h) = bar.strip();
        println!("compositing the strip at {w}x{h}");
    }

    let config = daysengine::install::config::Config::load(game);
    let counters = |pair: &str, flag: &str| -> anyhow::Result<(i32, i32)> {
        let (a, b) = pair
            .split_once(',')
            .with_context(|| format!("{flag} wants 001,002, got {pair}"))?;
        Ok((a.trim().parse()?, b.trim().parse()?))
    };
    let feeling = match &args.feeling {
        Some(pair) => Some(counters(pair, "--feeling")?),
        None => {
            slot_feeling(game).map(|(_, _, _, store)| daysengine::install::feeling::gauge(&store))
        }
    };
    let was = match &args.feeling_was {
        Some(pair) => Some(counters(pair, "--feeling-was")?),
        None => feeling,
    };
    let state = State {
        auto: args.auto,
        gauge: feeling,
        gauge_raised: args.gauge,
        paused: args.paused,
        replay: args.replay,
        message: args.message,
        skippable: !args.no_script && State::from_config(&config).skippable,
        // The skip button's own gate, which is the `SuperSkip` setting and
        // not `+0x88`: `FUN_10023fb0` case 4 asks `_GetSuperSkipFlag@0`. Off
        // in the player's own `Config.DAT` means a dead skip button, so it
        // comes from there unless `--super-skip` overrides it.
        super_skip: args.super_skip || State::from_config(&config).super_skip,
        following_record: args.following_record,
        ..State::default()
    };
    // The two move together in the engine, and the rate is what decides whether
    // the readout is drawn, so a `--speed` that left it behind would hide it.
    let mut state = state;
    state.set_speed(args.speed.min(bar::SPEEDS.len() - 1));

    // The level is the bar's own, and the only thing that moves it is a press
    // on one of the ten cells -- so that is how it is set here too.
    // Settling takes one update to start the ramp and one past its end to
    // finish it, exactly as the original's two calls per frame do.
    let settled = args
        .after
        .unwrap_or(bar::FADE_IN_MS.max(bar::FADE_OUT_MS) + 1);
    // The gauge draws a pair of its own that chases the save's, so it has to be
    // put somewhere before anything asks it what it looks like. `--feeling-was`
    // is the settle MenuBar `+0x38` does at the start of a script; a raise then
    // ramps from there to `--feeling`, one step per frame, and `--after` is how
    // far into that ramp the picture is taken.
    let mut gauge_sound = None;
    let mut ramp = bar::gauge::Anim::default();
    let mut fill = bar::gauge::Fill::default();
    if let Some((first, second)) = was {
        ramp.settle(first, second);
        fill.settle(first);
    }
    if let (true, Some((first, second))) = (args.gauge, feeling) {
        ramp.advance(0, first, second);
        gauge_sound = ramp.advance(0, first, second).sound;
        ramp.advance(settled, first, second);
        fill.advance(0, first);
        let fill_sound = fill.advance(0, first).sound;
        fill.advance(settled, first);
        if matches!(bar.layout().map(|l| l.gauge), Some(bar::Gauge::Fill { .. })) {
            gauge_sound = fill_sound;
        }
    }
    state.gauge_leads = ramp.leads();
    state.gauge_fill = fill.value();
    if let Some(want) = args.transparency {
        match bar.layout().map(|l| l.knob) {
            // School Days HQ's ten cells: pressing one stores its level.
            Some(bar::Knob::Cells) => {
                match (bar::indicator::FIRST_WIDGET..)
                    .take(bar::indicator::CELLS)
                    .find(|w| bar::indicator::level_for(*w) == Some(want))
                {
                    Some(widget) => {
                        if bar.press(widget, state, 0, None) == Act::None {
                            anyhow::bail!(
                                "the cells are dead without --following-record, so the press \
                                 that sets the transparency is swallowed"
                            );
                        }
                    }
                    None => anyhow::bail!(
                        "no cell sets a transparency of {want}; level 1 is unreachable"
                    ),
                }
            }
            // Shiny Days' knob is dragged, so there is no press that reaches a
            // level. Put it where that level would be: the run is 0 to 10 over
            // the knob's own travel.
            Some(bar::Knob::Drag { .. }) => {
                if want > 10 {
                    anyhow::bail!("--transparency runs 0 to 10, not {want}");
                }
                let across = want as f32 / 10.0;
                bar.set_knob(bar::slider::MIN_X + across * bar::slider::TRAVEL);
            }
            None => anyhow::bail!("this module's strip has no recovered transparency control"),
        }
    }

    let (w, h) = bar.screen().size();
    println!(
        "{} at {} — {w}x{h}, scale {:.2}, {} widgets ({}/{} boxes matched the table), {}",
        bar::PATH,
        resolution.name(),
        bar.screen().scale(),
        bar.widgets(),
        bar.screen().atlas().matched,
        bar.widgets(),
        match bar.layout() {
            Some(layout) => format!("records recovered from {}", layout.module),
            None => "no recovered record table addresses this strip".to_string(),
        },
    );
    println!(
        "state: auto {} paused {} replay {} message {} skippable {} \
         super-skip {} following-record {} speed x{}",
        state.auto,
        state.paused,
        state.replay,
        state.message,
        state.skippable,
        state.super_skip,
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
        let solidity = bar.transparency();
        println!(
            "replay indicator: {}, {} (alpha {})",
            if state.following_record {
                "live, slider live"
            } else {
                "off, slider dead"
            },
            match solidity {
                bar::Solidity::Level(level) => format!("transparency {level} of 10"),
                bar::Solidity::Knob(x) => format!(
                    "knob at x {x:.1} of {:.0}..{:.0}",
                    bar::slider::MIN_X,
                    bar::slider::MAX_X
                ),
            },
            solidity.alpha(),
        );
        if let bar::Solidity::Level(level) = solidity {
            match bar::indicator::knob(level) {
                Some((src, dst)) => println!(
                    "  knob   sheet ({:.0},{:.0}) {:.0}x{:.0} -> strip ({:.1},{:.1}) {:.0}x{:.0}",
                    src.x, src.y, src.w, src.h, dst.x, dst.y, dst.w, dst.h,
                ),
                None => println!("  knob   level {level} has no case in FUN_10027030"),
            }
        }
    }

    // The gauge is the one thing on the strip that is not a chip record: its
    // three pieces are cut from the sheet at sizes worked out from the two
    // counters, so print the cut rather than a record number.
    match state.gauge {
        None => println!("gauge: no save to read the counters from"),
        Some((first, second)) => {
            let (lead, _) = state.gauge_leads;
            let level = matches!(bar.layout().map(|l| l.gauge), Some(bar::Gauge::Fill { .. }));
            println!(
                "gauge: {} {first}, {} {second} — {}, {}",
                daysengine::install::feeling::FIRST,
                daysengine::install::feeling::SECOND,
                if level {
                    format!(
                        "{} alone, at {}",
                        daysengine::install::feeling::FIRST,
                        state.gauge_fill
                    )
                } else {
                    format!(
                        "lead {lead:+}px to the {}",
                        if lead >= 0.0 { "first" } else { "second" }
                    )
                },
                if state.gauge_raised {
                    "raised, so it draws even with the bar faded out"
                } else {
                    "drawn with the bar"
                },
            );
            if state.gauge_raised {
                println!(
                    "  ramp   {} -> {first},{second} at {settled}ms of {}, then held {}ms — {}",
                    match was {
                        Some((a, b)) => format!("{a},{b}"),
                        None => "0,0".to_string(),
                    },
                    bar::gauge::RAMP_MS,
                    bar::gauge::HOLD_MS,
                    match gauge_sound {
                        Some(se) => format!(
                            "plays {} ({})",
                            se.key(),
                            se.path(&film_ini(&vfs))
                                .unwrap_or("no such key in FILMENGINE.INI")
                        ),
                        None => "silent, neither counter moved".to_string(),
                    },
                );
            }
            let p = if matches!(bar.layout().map(|l| l.gauge), Some(bar::Gauge::Fill { .. })) {
                // A level, not a lead: one bar whose width is the ramped
                // counter, and no pieces at all.
                println!(
                    "  bar    {} wide, from {} alone",
                    state.gauge_fill.max(0.0).round(),
                    daysengine::install::feeling::FIRST,
                );
                bar::gauge::Pieces::default()
            } else {
                bar::gauge::pieces_at(lead, -lead)
            };
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
    if bar.layout().is_none() {
        println!(
            "  neither recovered record table addresses a strip with {} regions, so the bar \
             draws only the widget under the pointer",
            bar.widgets()
        );
    }
    for widget in 0..bar.widgets() {
        let rect = bar.screen().atlas().widgets[widget].dst;
        let act = bar
            .layout()
            .map_or(Act::None, |l| l.action(widget, state, args.latched));
        let shown = match act {
            Act::None => "-".to_string(),
            Act::ToggleAuto => "toggle the auto flag".to_string(),
            Act::TogglePause => "toggle pause".to_string(),
            Act::Seek { code, rewind } => match (code, rewind) {
                (bar::Seek::RESTART, _) => "seek: restart this part".to_string(),
                (bar::Seek::END_OF_PART, true) => {
                    "seek: out of this part, and back to the one before it".to_string()
                }
                (bar::Seek::END_OF_PART, false) => {
                    "seek: to this part's choice, else out of it".to_string()
                }
                (bar::Seek::SKIP, _) => {
                    "seek: to this part's choice, else chase one across the parts after it"
                        .to_string()
                }
                (other, _) => format!("seek code {}", other.0),
            },
            Act::Speed(i) => format!("speed x{}", bar::SPEEDS[i]),
            Act::Menu(m) => format!("open menu {}", m.0),
            Act::Leave => "leave playback".to_string(),
            Act::Transparency(n) => format!("set the replay indicator to {n} of 10"),
            Act::GrabKnob => "take hold of the replay indicator's knob".to_string(),
        };
        println!(
            "  {widget:3}  {:6}  ({:4},{:3}) {:3}x{:<3}  {:5}  {:>7}  {shown}",
            widget + 1,
            rect.x,
            rect.y,
            rect.width,
            rect.height,
            bar.enabled(widget, state),
            bar::caption(widget)
                .map(|c| c.to_string())
                .unwrap_or_else(|| "-".to_string()),
        );
    }
    if let (true, Some(layout)) = (state.auto, bar.layout()) {
        match layout.auto_lit {
            bar::Auto::Animated { frames, .. } => println!(
                "widget 0 is on animation record {} of {frames} after {}ms",
                layout.auto_lit.frame(args.elapsed, state.speed),
                args.elapsed
            ),
            bar::Auto::Lit(record) => {
                println!("widget 0 is lit: record {record}, and this module's bar has no animation")
            }
        }
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
    bar.point_at(at, 0);
    let hovered = bar.point_at(at, settled);
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
            art.modulate(sign.alpha);
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
            None if select.path.is_empty() => {
                "FILMENGINE.INI names no map for it; splitting the screen".to_string()
            }
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
    // Where each line lands, which is the whole of what the box looks like:
    // `System/Select` ships no art, so a label that is placed wrong is a label
    // nobody can see. The picture is 800x450 here, the space
    // `daysengine::playback::text::Geometry::native` works in.
    let geometry = daysengine::playback::text::Geometry::native(
        film.get_bool("LeftArrangement").unwrap_or(false),
    );
    let english = film.get_bool("UseEnglish").unwrap_or(false);
    println!(
        "placed in {}x{}, strip {:.1} wide",
        geometry.screen.0,
        geometry.screen.1,
        daysengine::ui::select::strip_width(choice.count(), select.layout, english, geometry),
    );
    for (i, label) in choice.labels.iter().enumerate() {
        let lines = metrics.lines(label);
        let places = daysengine::ui::select::place(
            &lines,
            i,
            choice.count(),
            select.layout,
            english,
            geometry,
        );
        for (line, at) in lines.iter().zip(places) {
            println!(
                "  label {i}: at ({:7.2},{:7.2}) x{:.3}/{:.3}  {line:?}",
                at.x, at.y, at.x_scale, at.y_scale
            );
        }
    }

    if let Some(pick) = args.pick {
        // Answered through the real tick, so this is the engine's own fade and
        // not a second implementation of it: a pointer on the box being picked
        // plus a press, or the right button for a decline.
        // A window long enough that the answer is the player's and not the
        // timeout's; the box above is built with a one-frame one.
        let mut choice = Choice::new(&args.label, args.label2.as_deref(), Frame(0), Frame(1000));
        let pointer = match usize::try_from(pick)
            .ok()
            .and_then(|i| Some((i, select.bounds()?)))
        {
            Some((i, bounds)) => match bounds.get(i) {
                Some(r) => {
                    let (w, h) = select.map_size().unwrap_or((1, 1));
                    (
                        (f64::from(r.x as u16) + f64::from(r.width) / 2.0) / f64::from(w),
                        (f64::from(r.y as u16) + f64::from(r.height) / 2.0) / f64::from(h),
                    )
                }
                None => bail!("there is no box {pick}"),
            },
            None => (0.5, 0.5),
        };
        let mut rng = |_: usize| 0;
        let hover = daysengine::ui::select::Input {
            pointer,
            ..Default::default()
        };
        choice.tick(Frame(0), &select, hover, false, &mut rng);
        let answering = daysengine::ui::select::Input {
            pick: pick >= 0,
            dismiss: pick < 0,
            ..hover
        };
        let event = choice.tick(Frame(1), &select, answering, false, &mut rng);
        println!("answered at frame 1: {event:?}");
        for frame in 1.. {
            if !choice.visible(Frame(frame)) {
                println!("  frame {frame}: the box is gone");
                break;
            }
            let colours: Vec<String> = (0..choice.count())
                .map(|i| match choice.label_colour(i, Frame(frame)) {
                    Some(c) => format!(
                        "#{:02x}{:02x}{:02x} alpha {:3}",
                        c.red, c.green, c.blue, c.alpha
                    ),
                    None => "not drawn         ".to_string(),
                })
                .collect();
            println!("  frame {frame}: {}", colours.join("   "));
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

/// Reports the page tables a screen lays its contents out in.
///
/// Each set is anchored to its own screen's frame table, so the screen asked
/// for is what picks between them.
fn ui_pages(dll: &[u8], screen: &str, frame: usize) {
    let stem = screen.to_ascii_lowercase();
    if stem.contains("option") {
        option_tables(dll, frame);
    } else if stem.contains("replay") {
        replay_tables(dll, frame);
    } else {
        println!("{screen} has no page tables");
    }
}

/// The Option screen's three tab pages.
fn option_tables(dll: &[u8], frame: usize) {
    use daysengine::ui::option_pages::{self, Pages};
    use daysengine::ui::options::Tab;

    let Some(pages) = Pages::locate(dll, frame) else {
        println!("no Option page tables in this module");
        return;
    };
    for tab in Tab::ALL {
        let base = pages.base(tab);
        let count = option_pages::records(tab);
        println!(
            "{} page: {count} records at DLL offset {base:#x}, widgets {}..={}",
            tab.variant(),
            option_pages::FIRST,
            option_pages::FIRST + count - 1,
        );
        for widget in option_pages::FIRST..option_pages::FIRST + count {
            let Some(w) = pages.widget(dll, tab, widget) else {
                println!("  widget {widget:3}  unreadable");
                continue;
            };
            // The pointer lands on the last pixel of a record, not the first:
            // the shipped test is half-open low and closed high.
            let back = pages.hit(dll, tab, w.dst.x + w.dst.width, w.dst.y + w.dst.height);
            println!(
                "  widget {widget:3}  dst ({:4},{:4}) {:4}x{:<3}  src ({:4},{:4})  hit -> {}",
                w.dst.x,
                w.dst.y,
                w.dst.width,
                w.dst.height,
                w.src_x,
                w.src_y,
                back.map_or_else(|| "none".to_string(), |n| n.to_string()),
            );
        }
        if tab == Tab::Sound {
            for slider in 0..option_pages::SLIDERS {
                let (Some(track), Some(knob)) = (pages.track(dll, slider), pages.knob(dll, slider))
                else {
                    continue;
                };
                let half = option_pages::slider_knob_x(&track, &knob, 0.5);
                println!(
                    "  slider {slider}  widget {}  track {}x{} at ({},{})  knob {}x{}  half -> x {half}, reads back {:.3}",
                    option_pages::FIRST_SLIDER + slider,
                    track.dst.width,
                    track.dst.height,
                    track.dst.x,
                    track.dst.y,
                    knob.dst.width,
                    knob.dst.height,
                    option_pages::slider_value(&track, &knob, half),
                );
            }
        }
    }
}

/// The Replay screen's two views.
fn replay_tables(dll: &[u8], frame: usize) {
    use daysengine::ui::replay::View;
    use daysengine::ui::replay_pages::{self, Pages};

    let Some(pages) = Pages::locate(dll, frame) else {
        println!("no Replay view tables in this module");
        return;
    };
    for view in [View::HScene, View::PlayData] {
        let base = pages.base(view);
        let count = replay_pages::records(view);
        println!(
            "{} view: {count} records at DLL offset {base:#x}, widgets {}..={}",
            view.variant(),
            replay_pages::FIRST,
            replay_pages::FIRST + count - 1,
        );
        for widget in replay_pages::FIRST..replay_pages::FIRST + count {
            let Some(w) = pages.widget(dll, view, widget) else {
                println!("  widget {widget:3}  unreadable");
                continue;
            };
            // The pointer lands on the last pixel of a record, not the first:
            // the shipped test is half-open low and closed high.
            let back = pages.hit(dll, view, w.dst.x + w.dst.width, w.dst.y + w.dst.height);
            let hover = pages.hover(dll, view, widget, 0).map_or_else(
                || "none".to_string(),
                |h| format!("({},{})", h.src_x, h.src_y),
            );
            println!(
                "  widget {widget:3}  dst ({:4},{:4}) {:4}x{:<3}  src ({:4},{:4})  hit -> {}  hover src {hover}",
                w.dst.x,
                w.dst.y,
                w.dst.width,
                w.dst.height,
                w.src_x,
                w.src_y,
                back.map_or_else(|| "none".to_string(), |n| n.to_string()),
            );
        }
    }
    println!("play-data list run at DLL offset {:#x}", pages.list());
    for page in 0..replay_pages::HSCENE_PAGES {
        let mark = pages.page_mark(dll, View::HScene, page);
        let first = pages.thumbnail(dll, page, 0);
        println!(
            "  HScene page {page}: scenes {}..={}, lit button src {}, first thumbnail src {}",
            replay_pages::scene_of(page, 0).map_or(-1, |s| s as i32),
            replay_pages::scene_of(page, replay_pages::THUMBNAILS - 1).map_or(-1, |s| s as i32),
            mark.map_or_else(
                || "none".to_string(),
                |m| format!("({},{})", m.src_x, m.src_y)
            ),
            first.map_or_else(
                || "none".to_string(),
                |t| format!("({},{})", t.src_x, t.src_y)
            ),
        );
    }
    for page in 0..replay_pages::pages(View::PlayData) {
        let top = replay_pages::window_top(page);
        let mark = pages.page_mark(dll, View::PlayData, page);
        println!(
            "  PlayData page {page}: entries {}..={}, panel {} of the strip over pages \
             {top}..={}, lit button src {}",
            page * replay_pages::PER_PAGE,
            page * replay_pages::PER_PAGE + replay_pages::PER_PAGE - 1,
            page - top,
            top + replay_pages::PLAYDATA_PANELS - 1,
            mark.map_or_else(
                || "none".to_string(),
                |m| format!("({},{})", m.src_x, m.src_y)
            ),
        );
    }
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
        // Where the per-region records actually sit. More than one run means
        // the screen's records are interleaved with other states of the same
        // widgets, and the alternates below are numbered from the end of the
        // last of them.
        for (first, at, count) in &atlas.segments {
            println!(
                "  run of {count} from region {} at {at:#x} (record {})",
                first + 1,
                (at - atlas.offset) / 24
            );
        }
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
        if args.out.is_none() && !args.pages {
            return Ok(());
        }
    }

    if args.pages {
        ui_pages(&dll, &args.screen, screen.atlas().offset);
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

/// A pack file's name without the `[FileExtend]`: `System`, `System.000`.
fn pack_name(p: &Path) -> String {
    daysengine::install::vfs::pack_display_name(p)
}

/// Every pack file in `Packs`: the base packs and their patch overlays.
///
/// The overlays are `System.GPK.000` .. `System.GPK.009`, so they are not
/// `.GPK` files by extension and have to be recognised by shape. They are pack
/// files all the same, which is why `verify` reads them and `extract` can name
/// one — `daysengine extract System.000`.
fn packs(dir: &Path) -> Result<Vec<PathBuf>> {
    let packs_dir = dir.join("Packs");
    let mut out = Vec::new();
    for entry in
        std::fs::read_dir(&packs_dir).with_context(|| format!("reading {}", packs_dir.display()))?
    {
        let path = entry?.path();
        if is_pack_file(&path) {
            out.push(path);
        }
    }
    out.sort();
    Ok(out)
}

fn is_pack_file(path: &Path) -> bool {
    let name = path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_ascii_lowercase();
    match name.rsplit_once('.') {
        Some((_, "gpk")) => true,
        Some((head, tail)) => {
            head.ends_with(".gpk") && tail.len() == 3 && tail.bytes().all(|b| b.is_ascii_digit())
        }
        None => false,
    }
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
    if let Ok(dll) = route_dll(game) {
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

/// Loads a slot and jumps to one of its story points, as the route map does.
///
/// The same two calls the game makes — `Progress::from_slot` then
/// `Progress::from_story` — so what this reports is where the jump would put
/// the player, and what it left of the run's history.
fn cmd_save_story(game: &Path, slot: u32, story: u32) -> Result<()> {
    use daysengine::install::progress::Progress;

    let vfs = daysengine::install::vfs::Vfs::mount(game)?;
    let film = film_ini(&vfs);
    let flags = load_flags(game, &vfs);
    let mut progress = Progress::load(&vfs, &route_dll(game)?, flags)?;
    if progress.load_from(game, &film, slot).is_none() {
        println!("slot {slot} is empty");
        return Ok(());
    }
    let before: Vec<String> = progress.marks().map(str::to_owned).collect();
    println!("slot {slot} carries {} story points", before.len());
    let Some(script) = progress.from_story(story) else {
        println!("SP{story:03} is not one of them, so the route map would grey it");
        return Ok(());
    };
    let (route, scene) = progress.position();
    println!("SP{story:03} plays {script} at ROUTE {route} SCENE {scene}");
    let after: Vec<String> = progress.marks().map(str::to_owned).collect();
    println!(
        "  {} story points left, {} erased: {}",
        after.len(),
        before.len() - after.len(),
        before
            .iter()
            .filter(|m| !after.contains(m))
            .cloned()
            .collect::<Vec<_>>()
            .join(" ")
    );
    Ok(())
}

/// Prints what one save slot holds.
///
/// `all` prints the store entry by entry rather than only the counters, which
/// is what says whether a name is in the store at all — a distinction the
/// getters hide, since a name that is not there reads as zero.
fn cmd_save_slot(game: &Path, n: u32, all: bool) -> Result<()> {
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
    if all {
        for (name, value) in slot.store.iter() {
            println!("      {name} = {value:?}");
        }
    } else {
        for name in ["001", "002", "000", "003", "004"] {
            if let Some(v) = int(name) {
                println!("      {name} = {v}");
            }
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
fn cmd_config(game: &Path, roundtrip: bool) -> Result<()> {
    use daysengine::install::config::{Channel, Config, Flag, Sound};

    let path = Config::path(game);
    let config = Config::load(game);
    println!("{}", path.display());
    if roundtrip {
        return config_roundtrip(&path, &config);
    }
    // Which units the three volumes are in is the menu module's answer, and a
    // module this tool cannot read leaves the settings unreadable too rather
    // than read in the wrong units.
    let sound = daysengine::ui::paths::Paths::from_module(&system_menu_dll(game)?).sound();
    println!();
    match sound {
        Sound::Levels => println!("volumes (level, then what reaches the sound layer):"),
        Sound::Fractions => println!("volumes (fraction, then what reaches the sound layer):"),
    }
    let muted = config.flag(Flag::Mute);
    for channel in Channel::ALL {
        let held = match sound {
            Sound::Levels => format!("{:>2}/10", config.volume(channel)),
            Sound::Fractions => format!("{:>5.3}", config.fraction(channel)),
        };
        let note = match (sound, muted) {
            (Sound::Levels, true) => {
                format!(
                    "   (muted: played at level {})",
                    config.effective_level(channel)
                )
            }
            (Sound::Fractions, true) => "   (muted: silenced)".to_string(),
            (_, false) => String::new(),
        };
        let db = config.attenuation_db(channel, sound);
        println!(
            "  {:<12} {held}   {:>6} cB {:>7.2} dB   gain {:.3}{note}",
            channel.key(),
            (db * 100.0).round() as i32,
            db,
            config.gain(channel, sound),
        );
    }
    println!("  {:<12} {:>6.3}", "MasterVolume", config.master_volume());
    // Whether the menus' own sounds follow Mute is the one place the two models
    // differ that is not visible from the three rows above.
    println!(
        "  {:<12} {:>27.3}   ({})",
        "menu sounds",
        config.system_se_gain(sound),
        match sound {
            Sound::Levels | Sound::Fractions => "SeVolume, silenced by Mute",
        }
    );
    if sound == Sound::Levels {
        println!();
        println!("the ladder, level by level:");
        print!("  ");
        for level in 0..=daysengine::install::config::MAX_VOLUME {
            print!("{level:>2}:{:<6} ", config.centibels(level));
        }
        println!();
    }
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

/// Checks that a settings file we write is one the retail game still reads.
///
/// The retail reader is `FUN_0046c520`, and what it will take is narrow: it
/// reads the whole file into a **1024-byte** buffer, checks four bytes of
/// magic, and hands the rest to `FUN_0046c310`, one `inflate` with `Z_FINISH`
/// into another 1024-byte buffer that then becomes a C string. So the file and
/// the text both have to fit, the stream has to finish in that one pass, and
/// every key has to be findable as `[Key]="` from position 0 — the getters'
/// search is `wcsstr` (`FUN_0046e330`).
///
/// The one difference this is allowed to report is a dropped fragment. The
/// retail writer replaces a key's line with a run of characters the length of
/// the *new* line rather than up to the newline, so a shorter value leaves the
/// tail of the old one behind and a longer one eats the next line's `[`. Those
/// fragments are unreachable for a reader that looks for `[Key]="`, and this
/// engine drops them instead of carrying them forward.
fn config_roundtrip(path: &Path, config: &daysengine::install::config::Config) -> Result<()> {
    use daysengine::install::config::{Config, MAGIC};

    /// Both of the retail reader's stack buffers.
    const BUFFER: usize = 1024;

    let original =
        std::fs::read(path).with_context(|| format!("{} cannot be read", path.display()))?;
    let before = inflate_config(&original)?;
    let ours = config.encode();
    let after = inflate_config(&ours)?;

    println!();
    println!(
        "  read back      {} bytes in, {} bytes out",
        original.len(),
        ours.len()
    );
    let mut bad = 0;
    let mut check = |what: &str, ok: bool, note: String| {
        println!(
            "  {:<14} {}  {note}",
            what,
            if ok { "ok  " } else { "FAIL" }
        );
        if !ok {
            bad += 1;
        }
    };
    check(
        "file size",
        ours.len() <= BUFFER,
        format!("{} of the reader's {BUFFER}-byte read buffer", ours.len()),
    );
    check(
        "magic",
        ours.starts_with(&MAGIC),
        format!("{:?}", String::from_utf8_lossy(&ours[..ours.len().min(4)])),
    );
    // A text that exactly fills the buffer leaves no room for the terminator
    // the reader relies on, so the limit is one short of it.
    check(
        "text size",
        after.len() < BUFFER,
        format!(
            "{} of the reader's {BUFFER}-byte inflate buffer",
            after.len()
        ),
    );
    check(
        "no NUL",
        !after.contains('\0'),
        "the reader stops the text at the first one".to_string(),
    );

    // Every key the file carries has to survive, whoever wrote the file.
    let reread = Config::parse(&ours).context("our own file will not parse")?;
    let mut lost = Vec::new();
    for (key, value) in config.entries() {
        match reread.get(key) {
            Some(back) if back == value => {}
            _ => lost.push(key.clone()),
        }
        if !find_key(&after, key).is_some_and(|v| v == value) {
            lost.push(format!("{key} (as the retail reader finds it)"));
        }
    }
    check(
        "keys",
        lost.is_empty(),
        if lost.is_empty() {
            format!("all {} come back", config.entries().len())
        } else {
            format!("lost {}", lost.join(", "))
        },
    );

    // What the file lost, which should only ever be the retail writer's own
    // unreachable fragments.
    let dropped: Vec<&str> = before
        .lines()
        .filter(|line| !line.is_empty() && !after.lines().any(|ours| ours == *line))
        .collect();
    if dropped.is_empty() {
        println!("  lines          ok    every line of the original is still there");
    } else {
        println!(
            "  lines          note  dropped {} line(s) the retail reader could",
            dropped.len()
        );
        println!("                       not have found anyway:");
        for line in dropped {
            println!("                         {line:?}");
        }
    }

    println!();
    if bad == 0 {
        println!("the retail reader would take this file");
    } else {
        bail!("{bad} check(s) failed: the retail game would not read this file");
    }
    Ok(())
}

/// The container, decoded the way `FUN_0046c520` decodes it.
fn inflate_config(bytes: &[u8]) -> Result<String> {
    use daysengine::install::config::MAGIC;

    let body = bytes
        .strip_prefix(&MAGIC)
        .context("not a Config.DAT: the magic is wrong")?;
    let text = miniz_oxide::inflate::decompress_to_vec_zlib(body)
        .map_err(|e| anyhow::anyhow!("the deflate stream will not inflate: {e}"))?;
    Ok(String::from_utf8_lossy(&text).into_owned())
}

/// `[Key]="` by `wcsstr`, then to the first `"` or `,` — `FUN_0046e330` and
/// `FUN_0046dfb0`, which is how every retail getter finds a value.
fn find_key<'t>(text: &'t str, key: &str) -> Option<&'t str> {
    let at = text.find(&format!("[{key}]=\""))? + key.len() + 4;
    let rest = &text[at..];
    Some(&rest[..rest.find(['"', ','])?])
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
    use daysengine::ui::paths::Paths;
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
    // A run's worth of backlog, for the one screen that reads it. There is no
    // playback here to have logged any, so a script stands in.
    let lines = match &args.lines_from {
        None => Vec::new(),
        Some(name) => {
            let wanted = name.to_uppercase();
            let (_, path) = script_paths(&vfs)
                .into_iter()
                .find(|(n, _)| *n == wanted)
                .with_context(|| format!("no script named {name}"))?;
            let script = days_script::Script::parse(&wanted, &vfs.read_path(&path)?)?;
            script
                .events
                .iter()
                .filter_map(|e| match &e.command {
                    days_script::Command::PrintText { speaker, text } => {
                        Some(daysengine::ui::backlog::Entry {
                            speaker: speaker.clone(),
                            text: text.clone(),
                        })
                    }
                    _ => None,
                })
                .collect()
        }
    };
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
        sound: Paths::from_module(&dll).sound(),
        scenes: Scenes::recover(&dll).unwrap_or_else(|err| {
            log::warn!("no replay scene table: {err}");
            Scenes::from_scenes(Vec::new())
        }),
        // No run has answered the dress-select screen at the point either of
        // these is built; the host object's constructor leaves the same zero.
        dress: None,
        display: Display {
            wide: resolution != Resolution::Standard,
            full_screen: false,
        },
        // `UseSOM` is the half of this the settings file carries; whether a
        // port is held is the engine's answer, and `--som-port` is what
        // stands in for one here.
        som: Som {
            enabled: Config::load(game).flag(daysengine::install::config::Flag::UseSom),
            attached: args.som_port.is_some(),
            port: args.som_port.and_then(|n| n.checked_sub(1)).unwrap_or(0),
            testing: false,
        },
        slots: daysengine::ui::saveload::Slots::read(game, &film, &flags, english),
        english,
        text_input: film.get_bool("TextInput").unwrap_or(false),
        run: args.run_from_slot.and_then(|n| {
            let slot = daysengine::install::save::load_slot(game, &film, n);
            if slot.is_none() {
                log::warn!("slot {n} is empty, so there is no run to stand in");
            }
            slot.map(|slot| slot.store)
        }),
        // The backlog's two answers. There is no run here to have logged any
        // lines, so the screen opens empty — which is what it does in the
        // original before the first `[PrintText]` too.
        flow: daysengine::ui::backlog::Flow::from_setting(
            film.get_u32("BackLogType").unwrap_or(0).into(),
        ),
        ruby: Config::load(game).int_or(
            daysengine::ui::backlog::RUBY_SETTING,
            film.get_u32("AgateUsing").unwrap_or(0) as i32,
        ) != 0,
        lines: lines.clone(),
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
            ("dress select", Mode::DRESS_SELECT),
        ];
        let paths = Paths::from_module(&dll);
        for (name, mode) in modes {
            let variant = if mode == Mode::TITLE {
                save.title_variant()
            } else {
                mode.default_variant()
            };
            let stem = paths.stem(mode.0, variant).unwrap_or_default();
            match Menu::open(&vfs, &dll, mode, session(), resolution) {
                Ok(menu) => println!(
                    "  mode {:>2}  {name:<14} {stem:<34} ok, {} widgets",
                    mode.0,
                    menu.screen().widget_count()
                ),
                Err(err) => println!("  mode {:>2}  {name:<14} {stem:<34} {err}", mode.0),
            }
        }
        // The dress-select popup is not a mode, so it cannot be opened like
        // one: it is the second hit map inside mode 9, and the only way to it
        // is to commit to a dress. See `daysengine::ui::dress`.
        match Menu::open(&vfs, &dll, Mode::DRESS_SELECT, session(), resolution) {
            Ok(mut menu) => {
                // Point at the first dress the way a player would, then
                // confirm: `Menu::confirm` acts on the selection.
                if let Some((x, y)) = menu.screen().widget_point(0) {
                    menu.point_at(x, y);
                }
                let acted = menu.confirm(&vfs, &dll).and_then(|action| {
                    // Committing starts the slide; the popup's map arrives
                    // thirty ticks later. `daysengine` gets those ticks from
                    // its frame loop, so here they are pumped by hand.
                    while menu.moving() {
                        menu.tick(&vfs, &dll, TICK)?;
                    }
                    Ok(action)
                });
                println!(
                    "  mode  9  dress popup   {:<34} {}",
                    paths.dress_select_popup().unwrap_or_default(),
                    match acted {
                        Ok(action) => format!(
                            "ok, {} widgets, committing -> {action:?}",
                            menu.screen().widget_count()
                        ),
                        Err(err) => err.to_string(),
                    }
                );
            }
            Err(err) => println!("  mode  9  dress popup   {:<34} {err}", ""),
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
        // The backlog is the fourth, and the one with no mode integer:
        // `setSystemInit` code 3 selects `DAT_1004ffc8`, whose constructor
        // installs `MENU::BackLogView::vftable`, and `SystemInit`'s switch has
        // no case that reaches it. See `daysengine::ui::backlog`.
        match Menu::open_backlog(&vfs, &dll, session(), resolution) {
            Ok(mut menu) => {
                let leaving = menu.leave(&vfs, &dll);
                println!(
                    "  code 3  {:<14} {:<8}  ok, {} widgets, leaving -> {:?}",
                    "backlog",
                    menu.variant(),
                    menu.screen().widget_count(),
                    leaving
                );
            }
            Err(err) => println!("  code 3  {:<14} {err}", "backlog"),
        }
        return Ok(());
    }

    // `--from-bar` is the whole point of this branch: a screen opened over
    // playback is a different entry, and the entry is what decides where Close
    // goes. Without it every run here is title-rooted, which is exactly the
    // blind spot that let a bar-opened Close land on the title.
    let mut menu = match args.from_bar {
        None => {
            let mode = args.mode.map_or(Mode::TITLE, Mode);
            Menu::open(&vfs, &dll, mode, session(), resolution)
                .with_context(|| format!("opening menu mode {}", mode.0))?
        }
        // Code 3 is the backlog, which has no mode integer of its own.
        Some(3) => Menu::open_backlog(&vfs, &dll, session(), resolution)
            .context("opening the backlog over playback")?,
        Some(code) => {
            let (mode, kind) = match code {
                4 => (Mode::SAVELOAD, Kind::Save),
                5 => (Mode::SAVELOAD, Kind::Load),
                // The Option screen has no save/load job to set.
                2 => (Mode::OPTION, Kind::Load),
                other => bail!(
                    "--from-bar {other} is not a menu the control bar can open;                      it passes host +0xf8 code 4 (save), 5 (load), 2 (option) or                      3 (backlog)."
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
        "{} ({}) — {} widgets, entry {:?}",
        menu.showing().name(),
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

    let mut stepping = false;
    for event in args
        .events
        .split(',')
        .map(str::trim)
        .filter(|e| !e.is_empty())
    {
        // The player's machine draws between one click and the next, and the
        // dress-select slide moves on those frames. So an event that follows a
        // click arrives after the slide has run, the way the player's would.
        // A `tick` takes that over: once frames are being counted by hand,
        // nothing is settled behind the caller's back, which is how a click
        // part-way through the slide is reached.
        let tick_event = event.starts_with("tick");
        if menu.moving() && !stepping && !tick_event {
            let mut ticks = 0;
            let mut settled = Action::Stay;
            while menu.moving() {
                let action = menu.tick(&vfs, &dll, TICK)?;
                if !matches!(action, Action::Stay) {
                    settled = action;
                }
                ticks += 1;
            }
            println!("  {:<12} -> {ticks} frames, {settled:?}", "(settling)");
        }
        stepping |= tick_event;
        let (action, label) = match event {
            "down" => (menu.navigate(&vfs, &dll, Dir::Down)?, "down".to_string()),
            "up" => (menu.navigate(&vfs, &dll, Dir::Up)?, "up".to_string()),
            "left" => (menu.navigate(&vfs, &dll, Dir::Left)?, "left".to_string()),
            "right" => (menu.navigate(&vfs, &dll, Dir::Right)?, "right".to_string()),
            "enter" => (menu.confirm(&vfs, &dll)?, "enter".to_string()),
            "esc" => (menu.cancel(&vfs, &dll)?, "esc".to_string()),
            "yes" => (menu.confirm_popup(&vfs, &dll)?, "yes".to_string()),
            "release" => (menu.release(), "release".to_string()),
            // One frame of whatever the screen animates itself, for looking at
            // the dress-select slide or the confirm popup's dim part-way
            // through: `tick` is one, `tick:N` is N. See
            // `daysengine::ui::dress::SLIDE_FRAMES` and
            // `daysengine::ui::menu::Dim`.
            "tick" => (menu.tick(&vfs, &dll, TICK)?, "tick".to_string()),
            other if other.starts_with("tick:") => {
                let count: usize = other["tick:".len()..]
                    .parse()
                    .with_context(|| format!("{other:?} needs a frame count, as tick:N"))?;
                let mut acted = Action::Stay;
                for _ in 0..count {
                    let action = menu.tick(&vfs, &dll, TICK)?;
                    if !matches!(action, Action::Stay) {
                        acted = action;
                    }
                }
                (acted, format!("tick {count}"))
            }
            other => {
                // Points are written `at:X:Y` rather than `at:X,Y` so the comma
                // stays free as the separator between events.
                let (kind, point) = other
                    .split_once(':')
                    .filter(|(k, _)| ["at", "click", "press", "drag"].contains(k))
                    .with_context(|| {
                        format!(
                            "unknown menu event {other:?}; expected down, up, enter, \
                             esc, yes, left, right, release, tick, tick:N, \
                             at:X:Y, click:X:Y, press:X:Y or drag:X:Y"
                        )
                    })?;
                let (x, y) = point
                    .split_once(':')
                    .with_context(|| format!("{other:?} needs both coordinates, as {kind}:X:Y"))?;
                let x: u32 = x.parse().with_context(|| format!("bad x in {other:?}"))?;
                let y: u32 = y.parse().with_context(|| format!("bad y in {other:?}"))?;
                let pointed = menu.point_at(x, y);
                match kind {
                    "at" => (pointed, format!("at {x},{y}")),
                    // A drag is the pointer moving with the button already
                    // down, so it is the same motion event and the drag latch
                    // is what makes it one.
                    "drag" => (pointed, format!("drag {x},{y}")),
                    "press" => (menu.press(x, y), format!("press {x},{y}")),
                    _ => (menu.confirm(&vfs, &dll)?, format!("click {x},{y}")),
                }
            }
        };
        println!(
            "  {label:<12} -> {action:?}  {} selection {:?}",
            menu.showing().name(),
            menu.selection()
        );
        match action {
            Action::Play => {
                // The dress-select screen is the only thing that puts one on
                // the session, so this line appears only on a run that came
                // through the title's START on a module that has mode 9.
                if let Some(dress) = menu.session().dress {
                    println!("  (committed dress {dress}, which is what NewRadish takes)");
                }
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
            // The SOMCON tab asks the engine, never the screen, so the tool
            // has to answer it the way the engine does. `--som-port` is what
            // stands in for a device; without one every request honestly
            // answers "nothing here".
            Action::Som(request) => {
                let mut som = menu.session().som;
                som.enabled = menu
                    .session()
                    .config
                    .flag(daysengine::install::config::Flag::UseSom);
                let stand_in = args.som_port.and_then(|n| n.checked_sub(1));
                match request {
                    daysengine::ui::options::SomRequest::Detect => {
                        som.attached = stand_in.is_some();
                        som.port = stand_in.unwrap_or(0);
                    }
                    daysengine::ui::options::SomRequest::Release => {
                        som.attached = false;
                        som.testing = false;
                    }
                    daysengine::ui::options::SomRequest::Port(port) => {
                        som.attached = stand_in.is_some();
                        som.port = port;
                    }
                    daysengine::ui::options::SomRequest::Test(on) => {
                        som.testing = on && som.attached
                    }
                }
                if !som.enabled {
                    som.attached = false;
                    som.testing = false;
                }
                println!("  (peripheral: {som:?})");
                menu.set_som(&vfs, &dll, som)?;
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

    // The last event gets the frames that follow it too, on the same terms as
    // every event before it: a player who clicks and then stops still watches
    // whatever it started finish. A run that counted its own frames keeps
    // them, so a slide stopped part-way stays stopped.
    if menu.moving() && !stepping {
        let mut ticks = 0;
        let mut settled = Action::Stay;
        while menu.moving() {
            let action = menu.tick(&vfs, &dll, TICK)?;
            if !matches!(action, Action::Stay) {
                settled = action;
            }
            ticks += 1;
        }
        println!("  {:<12} -> {ticks} frames, {settled:?}", "(settling)");
    }

    // The save/load screen's rows carry text the composite draws itself rather
    // than cutting out of art. Print the lines, so what the screen shows is
    // checkable against the install without reading pixels.
    if menu.showing().is(Mode::SAVELOAD) {
        let slots = &menu.session().slots;
        println!(
            "  {:?} screen, page {} of {}, {} slots filled",
            menu.kind(),
            menu.page() + 1,
            daysengine::ui::saveload::PAGES,
            slots.len()
        );
        // The ten rows the records have under them, which a drag can leave
        // starting part-way into a page — see `saveload::slot_at`.
        let rest = menu
            .list_slide()
            .map_or(0, daysengine::ui::saveload::Slide::rest_row);
        let live = (0..daysengine::ui::saveload::PER_PAGE).map(|row| {
            let slot = daysengine::ui::saveload::slot_at(menu.page(), rest, row);
            (slot, slots.get(slot))
        });
        for (slot, line) in live {
            match line {
                Some(line) => println!(
                    "    slot {slot:>3}  {:<24} {:<6} {}",
                    line.when, line.chapter, line.comment
                ),
                None => println!("    slot {slot:>3}  (empty)"),
            }
        }
        // The list slides a strip of page-panels on the module that has one,
        // and a page button starts that slide rather than changing the page.
        // Report where the strip is, so a run stopped part-way with `tick:N`
        // says so instead of looking like a page that did not turn.
        if let Some(slide) = menu.list_slide() {
            let offset = slide.offset(slide.shown());
            println!(
                "    strip: banks from page {}, shown page {} row {} offset {offset:.1}{}",
                slide.window_top() + 1,
                slide.page() + 1,
                slide.rest_row(),
                if slide.busy() {
                    ", sliding"
                } else {
                    " (at rest)"
                }
            );
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

    // The route map's cells are story points, and which of them are charted
    // and which can be picked is the whole of what the screen decides. Print
    // them, since neither is visible in a widget count.
    if menu.showing().is(Mode::ROUTEMAP) {
        let (episode, page, chart) = menu.chart();
        let charted = menu.charted();
        let pickable = menu.pickable();
        let marker = menu.marker();
        println!(
            "  route map, episode {} page {}, {} story points",
            episode + 1,
            page + 1,
            chart.cells
        );
        for cell in 0..chart.cells {
            let story = daysengine::ui::routemap::story(episode, chart.base, cell);
            println!(
                "    {:<6} {:<10} {:<14} {}",
                daysengine::ui::routemap::story_flag(story),
                if charted[cell] { "charted" } else { "blank" },
                if pickable[cell] {
                    "can be picked"
                } else {
                    "not this run"
                },
                if marker == Some(cell) {
                    "you are here"
                } else {
                    ""
                }
            );
        }
    }

    // The dress-select screen's own base art is the transparent plate, so what
    // it is drawn over is `[DressBG]`. It is a clip in the shipped install,
    // which the game plays and a PNG can only hold the first frame of; see
    // `daysengine::ui::dress::BACKGROUND_FPS`.
    let dress_back = daysengine::ui::dress::background(&start_script_ini(&vfs));
    if menu.showing().is(Mode::DRESS_SELECT) {
        match &dress_back {
            Some(background) => println!("dress background {background:?}"),
            None => println!("dress background: [DressBG] names nothing"),
        }
    }

    if let Some(out) = &args.out {
        // With no override, draw what the save says: the same choice the
        // engine makes, so the PNG shows the title the player would see.
        let dress_back = dress_back.as_ref().map(|b| b.path());
        let under = match &args.backdrop {
            Some(path) => Some(path.as_str()),
            None if menu.wants_title_backdrop() => Some(chosen.path.as_str()),
            None if menu.showing().is(Mode::DRESS_SELECT) => dress_back,
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
        let exe = std::fs::read(Binaries::discover(game)?.executable)?;
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

/// Reads the route module, which owns every branching decision the game makes.
fn route_dll(game: &Path) -> Result<Vec<u8>> {
    let path = Binaries::discover(game)?
        .route
        .with_context(|| no_module(game, &binaries::ROUTE_EXPORTS))?;
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
fn cmd_route_play(
    game: &Path,
    from: &str,
    choices: &str,
    steps: usize,
    rewind: usize,
    dress: u32,
) -> Result<()> {
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

    // The dress goes in before the run starts: `film_start` is what puts it
    // into the store, after `_ZeroReset@4` has emptied everything else.
    progress.set_dress(dress);
    // A walk starts where a film run starts, so the store carries what
    // `_ZeroReset@4` seeds and nothing else. See `Progress::film_start`.
    progress.film_start();
    if !progress.enter(from) {
        println!("{from} is in no route table, so nothing follows it");
        return Ok(());
    }
    println!(
        "the store a film run starts with: {}",
        progress
            .store()
            .iter()
            .map(|(name, value)| format!("{name}={value:?}"))
            .collect::<Vec<_>>()
            .join(", ")
    );
    // What the player's own store carries before the walk touches it, so the
    // read record the walk builds can be held against what the retail game
    // actually wrote. See `Progress::mark_read`.
    let was_read: std::collections::BTreeSet<String> = progress
        .global()
        .iter()
        .filter(|(_, value)| matches!(value, days_save::Value::Bool(true)))
        .map(|(name, _)| name.to_owned())
        .collect();
    let mut walked: Vec<String> = Vec::new();

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
        // What the engine will mark, which is the qualified name after the
        // swap, not the spelling the walk was asked for.
        walked.push(progress.script().to_owned());
        match progress.advance() {
            Some(next) => played = next,
            None => {
                println!("     the graph names nothing after this");
                read_record(&progress, &was_read, &walked);
                rewind_from(&mut progress, rewind);
                return Ok(());
            }
        }
    }
    println!("     stopped after {steps} scripts");
    read_record(&progress, &was_read, &walked);
    rewind_from(&mut progress, rewind);
    Ok(())
}

/// Walks the rewind back from where the forward walk stopped.
///
/// This is the control bar's widget 2 pressed twice, over and over: each step
/// asks `_GetBackScriptFile@12` where this part came from and takes the part's
/// feeling deltas back off the counters on the way. The counters are printed
/// beside each step because they are the half that is easy to get wrong — a
/// rewind that moved but did not un-credit would read the same here and play
/// differently three branches later.
fn rewind_from(progress: &mut daysengine::install::progress::Progress, steps: usize) {
    if steps == 0 {
        return;
    }
    println!("rewinding {steps} parts:");
    for step in 0..steps {
        let Some(back) = progress.back() else {
            println!("     there is nothing before this part");
            return;
        };
        let (route, scene) = progress.position();
        let ((first, second), _) = progress.gauge();
        println!("{step:>3}  ROUTE {route:>2} SCENE {scene:>3}  {back}   001={first} 002={second}");
    }
}

/// Reports the per-script read record the walk built, against the record the
/// player's own game wrote.
///
/// Every name marked should be a name the retail engine spells the same way,
/// so one this engine set that the player's store has never carried is either
/// a scene they have not reached or a spelling this engine got wrong. A walk
/// over ground the player has covered separates the two: if the whole walk
/// comes back unknown, the spelling is wrong.
fn read_record(
    progress: &daysengine::install::progress::Progress,
    was_read: &std::collections::BTreeSet<String>,
    walked: &[String],
) {
    let marked: std::collections::BTreeSet<&str> = progress
        .global()
        .iter()
        .filter(|(name, value)| {
            matches!(value, days_save::Value::Bool(true)) && !was_read.contains(*name)
        })
        .map(|(name, _)| name)
        .collect();
    let known = walked.iter().filter(|n| was_read.contains(*n)).count();
    println!(
        "\nthe read record: {} scripts played, {known} of them already read in the player's \
         own store, {} name{} newly marked",
        walked.len(),
        marked.len(),
        if marked.len() == 1 { "" } else { "s" }
    );
    for name in &marked {
        println!("  newly marked  {name}");
    }
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
            // And where the control bar's rewind goes from here, which is a
            // second export with its own 55-way switch — see
            // `days_route::Machine::back_step`.
            match machine.back_step(route, scene as u16) {
                Some(step) => {
                    for line in transitions(&step) {
                        println!("  back: {line}");
                    }
                }
                None => println!("  back: this module exports no rewind"),
            }
            // What `_SetFeeling@8` credits here, which is its own switch and
            // not the branch graph's destination.
            if let Some(step) = machine.crediting(route, scene as u16) {
                for line in creditings(&step) {
                    println!("  credits on the way out: {line}");
                }
            }
            // What the three ending exports answer here, under a save whose
            // flags are all clear and then all set. A position they do not
            // name answers the same either way and prints nothing.
            struct Flags(bool);
            impl days_route::Context for Flags {
                fn choice(&self) -> i32 {
                    -1
                }
                fn int(&self, _: &str) -> i32 {
                    0
                }
                fn flag(&self, _: &str) -> bool {
                    self.0
                }
                fn global_flag(&self, _: &str) -> bool {
                    self.0
                }
                fn threshold(&self, _: &str) -> bool {
                    self.0
                }
            }
            let (clear, set) = (Flags(false), Flags(true));
            let scene = scene as u16;
            let view = (
                machine.end_roll_view(route, scene, &clear),
                machine.end_roll_view(route, scene, &set),
            );
            if view != (true, true) {
                println!(
                    "  end roll: plays with the flags clear {}, with them set {}",
                    view.0, view.1
                );
            }
            let letter = |pick: Option<bool>| match pick {
                Some(true) => "A",
                Some(false) => "B",
                None => "the path as written",
            };
            let pair = (
                machine.end_roll_select(route, scene, &clear),
                machine.end_roll_select(route, scene, &set),
            );
            if pair != (None, None) {
                println!(
                    "  end roll pair: {} with the flags clear, {} with them set",
                    letter(pair.0),
                    letter(pair.1)
                );
            }
            let card = (
                machine.change_subtitle(route, scene, &clear),
                machine.change_subtitle(route, scene, &set),
            );
            if card != (false, false) {
                println!(
                    "  ending card: the Ex01 one with the flags clear {}, with them set {}",
                    card.0, card.1
                );
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
                    if let Some(step) = machine.back_step(route, scene as u16) {
                        for line in transitions(&step) {
                            println!("           <- {line}");
                        }
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
        Next::Bookmark { route, name } => format!("ROUTE {route} SCENE the {name} bookmark"),
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
                Act::Uncredit(n) => Some(format!("take back {n}'s deltas")),
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

    // The rewind is its own export and works its answer out independently, so
    // the two agreeing is a check on both: from wherever `_GetBackScriptFile@12`
    // says this scene came, the branch graph has to be able to reach it again.
    // A bookmarked arm reads the scene out of the save and cannot be checked
    // without one, and an arm that stops has nothing to check.
    let (mut backs, mut agree, mut bookmarked, mut stops, mut blind_back) = (0, 0, 0, 0, 0);
    let mut stays = 0;
    let mut skipped: Vec<String> = Vec::new();
    for route in 0..routes.len() {
        let table = routes.route(route).unwrap_or_default();
        for scene in 0..table.len() {
            let scene = scene as u16;
            let Some(step) = machine.back_step(route, scene) else {
                continue;
            };
            let mut out = Vec::new();
            leaves(&step, &mut out);
            if out.is_empty() {
                blind_back += 1;
                continue;
            }
            for n in out {
                match n {
                    Next::Scene { route: r, scene: s } => {
                        backs += 1;
                        let forward = machine.step(*r as usize, *s);
                        let mut onward = Vec::new();
                        if let Some(f) = forward.as_ref() {
                            leaves(f, &mut onward);
                        }
                        let reaches = onward.iter().any(|f| {
                            matches!(f, Next::Scene { route: fr, scene: fs }
                                if *fr as usize == route && *fs == scene)
                        });
                        if reaches {
                            agree += 1;
                        } else if (*r as usize, *s) == (route, scene) {
                            // The handler names this very scene: there is
                            // nothing before it, so the rewind replays it.
                            stays += 1;
                        } else {
                            skipped.push(format!(
                                "    ROUTE {route} SCENE {scene} <- ROUTE {r} SCENE {s}, \
                                 whose own edge goes elsewhere"
                            ));
                        }
                    }
                    Next::Bookmark { .. } => bookmarked += 1,
                    Next::Stop => stops += 1,
                    Next::Named { .. } | Next::Nothing => {}
                }
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
    println!(
        "  the rewind: {backs} edges naming a scene, {agree} of which the branch graph \
         leads back from and {stays} naming the scene itself"
    );
    println!(
        "  {bookmarked} rewind arms read a BS**** bookmark, {stops} stop, \
         {blind_back} undecoded"
    );
    println!(
        "  {} rewind edges name a scene whose own forward edge goes somewhere else",
        skipped.len()
    );
    for line in skipped.iter().take(20) {
        println!("{line}");
    }

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

/// Reports and draws the backlog screen over one script's lines.
///
/// The lines are the script's own `[PrintText]` statements in timeline order,
/// which is the order `FUN_0043dbe0` would have logged them in. The real log
/// runs across scripts, so this is one script's worth of it rather than a
/// session's — enough to check the wrap, the stacking and the placement against
/// the player's own art and font.
fn cmd_backlog(game: &Path, args: &BacklogArgs) -> Result<()> {
    use daysengine::ui::backlog::{self, Entry, Flow};
    use daysengine::ui::paths::Paths;
    use daysengine::ui::screen::{Resolution, Screen, WidgetState};

    let vfs = daysengine::install::vfs::Vfs::mount(game)?;
    let dll = system_menu_dll(game)?;
    let film = film_ini(&vfs);
    let config = daysengine::install::config::Config::load(game);
    let english = film.get_bool("UseEnglish").unwrap_or(false);
    let flow = Flow::from_setting(film.get_u32("BackLogType").unwrap_or(0).into());
    // `[AgateUsing]` is the shipped default and `Config.DAT`'s `UseAgate` is
    // what the engine actually reads, so the file wins where it has an answer.
    let ruby = config.int_or(
        backlog::RUBY_SETTING,
        film.get_u32("AgateUsing").unwrap_or(0) as i32,
    ) != 0;

    let resolution = Resolution::from_name(&args.resolution)
        .with_context(|| format!("unknown resolution {}", args.resolution))?;
    let stem = Paths::from_module(&dll)
        .backlog_stem(flow)
        .context("the menu module holds no backlog screen")?;
    let mut screen = Screen::load(&vfs, &dll, stem, resolution)?;
    if let Some(size) = &args.at_size {
        let (w, h) = parse_size(size)?;
        screen.fit_to(w, h);
    }

    let wanted = args.script.to_uppercase();
    let (_, path) = script_paths(&vfs)
        .into_iter()
        .find(|(n, _)| *n == wanted)
        .with_context(|| format!("no script named {}", args.script))?;
    let script = days_script::Script::parse(&wanted, &vfs.read_path(&path)?)?;
    let entries: Vec<Entry> = script
        .events
        .iter()
        .filter_map(|e| match &e.command {
            days_script::Command::PrintText { speaker, text } => Some(Entry {
                speaker: speaker.clone(),
                text: text.clone(),
            }),
            _ => None,
        })
        .collect();

    let at = args
        .at
        .unwrap_or_else(|| backlog::opening_entry(entries.len()));
    let (w, h) = screen.size();
    println!(
        "{stem} at {} — {w}x{h}, {} lines, {} of them, entry {at} of {}",
        resolution.name(),
        if english { "English" } else { "Japanese" },
        entries.len(),
        entries.len().saturating_sub(1),
    );
    println!(
        "  [BackLogType] {:?}, {} columns, rows to {}{}",
        flow,
        flow.columns(english),
        flow.limit(english),
        if ruby {
            " — ruby is on, and this engine does not draw it"
        } else {
            ""
        }
    );
    for (index, top) in backlog::visible(&entries, at, flow, english) {
        let entry = &entries[index];
        let lines = backlog::wrap(&entry.text, flow, english);
        println!(
            "  entry {index:4} at {top:5}, {:3} tall, {} {:?}",
            backlog::height(entry, flow, english),
            if entry.speaker.is_empty() {
                "no speaker".to_string()
            } else {
                format!("speaker {:?}", entry.speaker)
            },
            lines,
        );
    }

    if let Some(out) = &args.out {
        let font = load_font(&vfs)?;
        let mut states = vec![WidgetState::Resting; screen.widget_count()];
        for id in &args.active {
            if let Some(state) = states.get_mut(id.saturating_sub(1)) {
                *state = WidgetState::Active;
            }
        }
        let buffer = backlog::draw(&font, &entries, at, flow, english);
        let image = backlog::compose(&screen, &states, &buffer);
        write_png(out, &image.rgba, image.width, image.height)?;
        println!(
            "wrote {} at {}x{}",
            out.display(),
            image.width,
            image.height
        );
    }
    Ok(())
}

/// The glyph store, whichever of the two an install ships.
fn load_font(vfs: &daysengine::install::vfs::Vfs) -> Result<days_font::Font> {
    let bytes = vfs
        .read_path("System/System/FONTDATA_ENG.DAT")
        .or_else(|_| vfs.read_path("System/System/FONTDATA.DAT"))?;
    Ok(days_font::Font::parse(bytes)?)
}
