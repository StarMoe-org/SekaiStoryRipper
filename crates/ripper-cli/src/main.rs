mod audio_lookup;
mod config;
mod fetch_cmd;
mod manifest_cmd;
mod masterdata_cmd;
mod remote_out;
mod s3;
mod story_cmd;
mod unpack_cmd;

use std::path::PathBuf;

use clap::{Parser, Subcommand};

use crate::config::Config;

#[derive(Parser)]
#[command(
    name = "ripper",
    version,
    about = "Project Sekai (CN, JP) story asset ripper"
)]
struct Cli {
    /// Config file (default: ./ripper.toml when present). See ripper.example.toml.
    #[arg(long, global = true)]
    config: Option<PathBuf>,
    /// Game server: cn or jp (default: cdn.region in the config, else cn). Picks the preset
    /// defaults, including separate cache/out directories for JP.
    #[arg(long, global = true, value_parser = parse_region)]
    region: Option<ripper_cdn::Region>,
    /// Override paths.cache.
    #[arg(long, global = true)]
    cache: Option<PathBuf>,
    /// Override paths.out: a directory, or s3://bucket/prefix to publish to S3 (see [s3] in
    /// ripper.example.toml; credentials from AWS_ACCESS_KEY_ID / AWS_SECRET_ACCESS_KEY).
    #[arg(long, global = true)]
    out: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Fetch the current manifest (or import a decrypted one), archive it and diff it against the last one.
    Manifest {
        /// Use this asset version (CN `N` of ios{N}, JP e.g. 6.8.0.50 with cdn.jp.asset_hash)
        /// instead of asking the server.
        #[arg(long)]
        asset_version: Option<String>,
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
    /// Resolve story episodes to the bundles they need (fetches only scenario bundles).
    Plan {
        /// e.g. unit:school-refusal-story-chapter/1, event:120/1-4, card:1, special:2, scenario:<id>, all.
        #[arg(required = true)]
        selectors: Vec<String>,
        #[arg(long)]
        asset_version: Option<String>,
        /// Write every plan as JSON.
        #[arg(long)]
        report: Option<PathBuf>,
        /// Fail when any episode has warnings.
        #[arg(long)]
        strict: bool,
    },
    /// Export episodes completely: fetch + unpack every bundle they need and write episode indexes.
    Rip {
        #[arg(required = true)]
        selectors: Vec<String>,
        #[arg(long)]
        asset_version: Option<String>,
        /// Write a per-episode summary as JSON.
        #[arg(long)]
        report: Option<PathBuf>,
        /// Fail when any episode has warnings.
        #[arg(long)]
        strict: bool,
        /// Unpack again even when the library already holds the bundle content.
        #[arg(long)]
        force: bool,
        /// Also keep ASTC textures' original blocks as .astc files.
        #[arg(long)]
        keep_astc: bool,
    },
    /// Fetch the masterdata tables the resolver needs (from masterdata.url_template) into the cache.
    Masterdata {
        /// Re-download tables that are already cached.
        #[arg(long)]
        refresh: bool,
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
        asset_version: Option<String>,
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
        asset_version: Option<String>,
        /// Do not follow manifest `dependencies`.
        #[arg(long)]
        no_deps: bool,
        /// Unpack again even when the library already holds this bundle content.
        #[arg(long)]
        force: bool,
        /// Also keep ASTC textures' original blocks as .astc files next to the PNGs.
        #[arg(long)]
        keep_astc: bool,
    },
}

fn parse_region(text: &str) -> Result<ripper_cdn::Region, String> {
    match text {
        "cn" => Ok(ripper_cdn::Region::Cn),
        "jp" => Ok(ripper_cdn::Region::Jp),
        _ => Err(format!("unknown region {text:?} (cn or jp)")),
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let mut config = Config::load(cli.config.as_deref(), cli.region)?;
    if let Some(cache) = cli.cache {
        config.paths.cache = cache;
    }
    if let Some(out) = cli.out {
        config.paths.out = out;
    }
    config.attach_remote()?;
    let writes_out = matches!(
        cli.command,
        Command::Plan { .. } | Command::Rip { .. } | Command::Unpack { .. }
    );
    let result = run(&config, cli.command).await;
    if let (Some(remote), true) = (&config.remote, writes_out) {
        // Publish what was written even when the command failed part-way: every bundle that
        // made it to staging is complete (its record is written last).
        eprintln!("publishing to {} ...", remote.location());
        let summary = remote.publish().await?;
        eprintln!(
            "published: {} uploaded ({:.1} MB), {} unchanged",
            summary.uploaded,
            summary.bytes as f64 / 1e6,
            summary.unchanged
        );
    }
    result
}

async fn run(config: &Config, command: Command) -> anyhow::Result<()> {
    match command {
        Command::Manifest {
            asset_version,
            from_file,
            refresh,
            diff_out,
        } => {
            manifest_cmd::run(
                config,
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
                config,
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
        Command::Plan {
            selectors,
            asset_version,
            report,
            strict,
        } => {
            let args = story_cmd::Args {
                selectors,
                asset_version,
                report,
                strict,
                force: false,
                keep_astc: false,
            };
            story_cmd::run_plan(config, args).await
        }
        Command::Rip {
            selectors,
            asset_version,
            report,
            strict,
            force,
            keep_astc,
        } => {
            let args = story_cmd::Args {
                selectors,
                asset_version,
                report,
                strict,
                force,
                keep_astc,
            };
            story_cmd::run_rip(config, args).await
        }
        Command::Masterdata { refresh } => masterdata_cmd::run(config, refresh).await,
        Command::Unpack {
            names,
            prefixes,
            asset_version,
            no_deps,
            force,
            keep_astc,
        } => {
            unpack_cmd::run(
                config,
                unpack_cmd::Args {
                    names,
                    prefixes,
                    asset_version,
                    no_deps,
                    force,
                    keep_astc,
                },
            )
            .await
        }
    }
}
