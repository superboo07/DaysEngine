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
            let mut ar = Archive::open(&p, &key)?;
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
        Cmd::Verify { pack } => {
            let packs = select_packs(&game, pack.as_deref())?;
            let (mut ok, mut bad) = (0usize, 0usize);
            for p in packs {
                let mut ar = Archive::open(&p, &key)?;
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
