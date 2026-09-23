mod config;
mod fetch_cmd;
mod manifest_cmd;
mod spike;

use std::path::PathBuf;

use clap::{Parser, Subcommand};

use crate::config::Config;

#[derive(Parser)]
#[command(
    name = "ripper",
    version,
    about = "Project Sekai (CN) story asset ripper"
)]
struct Cli {
    /// Config file (default: ./ripper.toml when present). See ripper.example.toml.
    #[arg(long, global = true)]
    config: Option<PathBuf>,
    /// Override paths.cache.
    #[arg(long, global = true)]
    cache: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Fetch the current manifest (or import a decrypted one), archive it and diff it against the last one.
    Manifest {
        /// Use ios{N} instead of reading the CDN version file.
        #[arg(long)]
        asset_version: Option<u32>,
        /// Import an already decrypted manifest (msgpack, or JSON with a `bundles` map) instead of fetching.
        #[arg(long)]
        from_file: Option<PathBuf>,
        /// Re-download even if this version is already archived.
        #[arg(long)]
        refresh: bool,
        /// Write the diff against the previous archived version as JSON.
        #[arg(long)]
        diff_out: Option<PathBuf>,
    },
    /// Download bundles into the cache (with their manifest dependencies), verifying length and CRC.
    Fetch {
        /// Exact bundle names, e.g. live2d/model/01ichika_normal.
        names: Vec<String>,
        /// Every bundle whose name starts with this prefix (repeatable).
        #[arg(long = "prefix")]
        prefixes: Vec<String>,
        /// Use this archived manifest instead of the latest.
        #[arg(long)]
        asset_version: Option<u32>,
        /// Do not follow manifest `dependencies`.
        #[arg(long)]
        no_deps: bool,
        /// Skip the CRC check over decompressed entries (length is always checked).
        #[arg(long)]
        no_verify: bool,
    },
    /// M0 spike: unpack every deobfuscated bundle under CACHE into OUT for oracle comparison.
    Spike {
        /// Directory of plain UnityFS bundles laid out as <cache>/<bundleName>.
        cache: PathBuf,
        out: PathBuf,
        #[arg(long, default_value = ripper_unity::DEFAULT_UNITY_VERSION)]
        unity_version: String,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let mut config = Config::load(cli.config.as_deref())?;
    if let Some(cache) = cli.cache {
        config.paths.cache = cache;
    }
    match cli.command {
        Command::Manifest {
            asset_version,
            from_file,
            refresh,
            diff_out,
        } => {
            manifest_cmd::run(
                &config,
                manifest_cmd::Args {
                    asset_version,
                    from_file,
                    refresh,
                    diff_out,
                },
            )
            .await
        }
        Command::Fetch {
            names,
            prefixes,
            asset_version,
            no_deps,
            no_verify,
        } => {
            fetch_cmd::run(
                &config,
                fetch_cmd::Args {
                    names,
                    prefixes,
                    asset_version,
                    no_deps,
                    no_verify,
                },
            )
            .await
        }
        Command::Spike {
            cache,
            out,
            unity_version,
        } => tokio::task::spawn_blocking(move || spike::run(&cache, &out, &unity_version)).await?,
    }
}
