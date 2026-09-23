mod spike;

use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "ripper",
    version,
    about = "Project Sekai (CN) story asset ripper"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// M0 spike: unpack every deobfuscated bundle under CACHE into OUT for oracle comparison.
    Spike {
        /// Directory of plain UnityFS bundles laid out as <cache>/<bundleName>.
        cache: PathBuf,
        out: PathBuf,
        #[arg(long, default_value = ripper_unity::DEFAULT_UNITY_VERSION)]
        unity_version: String,
    },
}

fn main() -> anyhow::Result<()> {
    match Cli::parse().command {
        Command::Spike {
            cache,
            out,
            unity_version,
        } => spike::run(&cache, &out, &unity_version),
    }
}
