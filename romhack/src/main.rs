use async_std::{fs, path::PathBuf, task};
use clap::{Parser, Subcommand, ValueHint};
use geckolib::{iso::builder::{Builder, PatchBuilder}, new};

#[cfg(feature = "progress")]
mod progress;

#[derive(Debug, Parser)]
#[command(author, version)]
/// Patches a game file
struct Cli {
    #[command(subcommand)]
    /// Sub command
    cmd: Commands,
}

#[derive(Subcommand, Debug)]
/// Command from 
enum Commands {
    /// Builds the Rom Hack
    Build {
        #[arg(short, long)]
        /// Compiles the Rom Hack in Rust's debug mode
        debug: bool,
        #[arg(short, long)]
        /// Compiles the Rom Hack as a patch
        patch: bool,
        #[arg(short, long)]
        /// Compiles the Rom Hack from local files
        raw: bool
    },
    /// Applies a patch file to a game to create a Rom Hack
    Apply {
        #[arg(value_hint = ValueHint::FilePath)]
        /// Input path to patch file
        patch: PathBuf,
        #[arg(value_hint = ValueHint::FilePath)]
        /// Input path to original game (GCM or ISO format)
        original_game: PathBuf,
        #[arg(value_hint = ValueHint::Other)]
        /// Output path for Rom Hack
        output: PathBuf,
    },
    /// Creates a new Rom Hack with the given name
    New {
        #[arg(value_hint = ValueHint::Other)]
        name: String
    },
}

fn main() -> color_eyre::eyre::Result<()> {
    color_eyre::install()?;
    #[cfg(feature = "log")]
    env_logger::init();
    #[cfg(feature = "progress")]
    progress::init_cli_progress();

    let args = Cli::parse();

    match args.cmd {
        Commands::Build { debug, patch, raw } => {
            task::block_on::<_, color_eyre::eyre::Result<()>>(async {
                let mut iso =
                    PatchBuilder::with_config(toml::from_str(&fs::read_to_string("RomHack.toml").await?)?);
                iso.build()?;
                Ok(())
            })
        },
        Commands::Apply { patch, original_game, output } => todo!(),
        Commands::New { name } => {
            new(&name)?;
            Ok(())
        },
    }
}
