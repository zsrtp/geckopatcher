use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueHint};

#[derive(Debug, Parser)]
#[command(author, version)]
/// Patches a game file
pub struct Cli {
    #[command(subcommand)]
    /// Sub command
    pub cmd: Commands,
}

#[derive(Subcommand, Debug)]
/// Command from
pub enum Commands {
    /// Builds the Rom Hack
    Build {
        #[arg(short, long)]
        /// Compiles the Rom Hack into a patch file
        patch: bool,
        #[arg(short, long)]
        /// Legacy option kept for compatibility
        raw: bool,
    },
    /// Applies a patch file to a game to create a Rom Hack
    Apply {
        #[arg(value_hint = ValueHint::FilePath)]
        /// Input path to patch file
        patch: PathBuf,
        #[arg(value_hint = ValueHint::FilePath)]
        /// Input path to original game (GCM or ISO format)
        original_game: PathBuf,
        #[arg(value_hint = ValueHint::Unknown)]
        /// Output path for Rom Hack
        output: PathBuf,
    },
    /// Creates a new Rom Hack with the given name
    New {
        #[arg(value_hint = ValueHint::Other)]
        name: String,
    },
}
