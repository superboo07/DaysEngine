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
    /// Decode every movie referenced by a script, checking frame counts against
    /// the timeline the script declares.
    Timing {
        /// Script name, e.g. "00-00-A00".
        name: String,
    },
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
fn script_paths(vfs: &days_vfs::Vfs) -> Vec<(String, String)> {
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
    let vfs = days_vfs::Vfs::mount(game)?;
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
    let vfs = days_vfs::Vfs::mount(game)?;
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
    let vfs = days_vfs::Vfs::mount(game)?;
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
    let vfs = days_vfs::Vfs::mount(game)?;
    println!("ffmpeg {}", days_media::ffmpeg_version());

    let handle = vfs
        .resolve(path)
        .with_context(|| format!("no asset at {path}"))?;
    let entry = vfs.entry(handle);
    let name = entry.name.clone();
    let bytes = vfs.read(handle)?;
    println!("{} ({} bytes)", name, bytes.len());

    if name.to_ascii_lowercase().ends_with(".wmv") {
        let mut decoder = days_media::VideoDecoder::open(bytes)?;
        println!("video {}x{}", decoder.width(), decoder.height());
        let mut frames = 0usize;
        let mut last = 0.0;
        let mut first: Option<days_media::VideoFrame> = None;
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
        let audio = days_media::decode_audio(bytes)?;
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
fn write_ppm(path: &Path, frame: &days_media::VideoFrame) -> Result<()> {
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
    let vfs = days_vfs::Vfs::mount(game)?;
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
        let mut decoder = days_media::VideoDecoder::open(vfs.read(handle)?)?;
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
