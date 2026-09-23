mod config;
mod fetch_cmd;
mod manifest_cmd;
mod unpack_cmd;

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
    /// Override paths.out.
    #[arg(long, global = true)]
    out: Option<PathBuf>,
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
    /// Fetch bundles if needed and unpack them into <out>/library/<bundleName>/ (skips up-to-date ones).
    Unpack {
        names: Vec<String>,
        #[arg(long = "prefix")]
        prefixes: Vec<String>,
        #[arg(long)]
        asset_version: Option<u32>,
        /// Do not follow manifest `dependencies`.
        #[arg(long)]
        no_deps: bool,
        /// Unpack again even when the library already holds this bundle content.
        #[arg(long)]
        force: bool,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let mut config = Config::load(cli.config.as_deref())?;
    if let Some(cache) = cli.cache {
        config.paths.cache = cache;
    }
    if let Some(out) = cli.out {
        config.paths.out = out;
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
        Command::Unpack {
            names,
            prefixes,
            asset_version,
            no_deps,
            force,
        } => {
            unpack_cmd::run(
                &config,
                unpack_cmd::Args {
                    names,
                    prefixes,
                    asset_version,
                    no_deps,
                    force,
                },
            )
            .await
        }
    }
}
