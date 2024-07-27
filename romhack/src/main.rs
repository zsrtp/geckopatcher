use std::str::FromStr;

use async_std::task;
use clap::Parser;
use geckolib::{
    iso::builder::Builder, new, open_config_from_fs_iso, open_config_from_fs_patch,
    open_config_from_patch,
};

#[cfg(feature = "progress")]
use geckolib::{update::UpdaterType, UPDATER};

#[cfg(feature = "progress")]
mod progress;

use romhack::cli::{Cli, Commands};

fn main() -> color_eyre::eyre::Result<()> {
    color_eyre::install()?;
    #[cfg(feature = "log")]
    env_logger::init();
    #[cfg(feature = "progress")]
    progress::init_cli_progress();

    let args = Cli::parse();

    #[cfg(feature = "progress")]
    if let Ok(mut updater) = UPDATER.lock() {
        updater.set_type(UpdaterType::Spinner)?;
        updater.init(Some(4))?;
        updater.set_title("Initializing...".into())?;
    }

    match args.cmd {
        Commands::Build { patch, raw: _ } => {
            task::block_on::<_, color_eyre::eyre::Result<()>>(async {
                if patch {
                    let mut builder = open_config_from_fs_patch(
                        &async_std::path::PathBuf::from_str("RomHack.toml")?,
                    )
                    .await?;
                    builder.build().await
                } else {
                    let mut builder = open_config_from_fs_iso(&async_std::path::PathBuf::from_str(
                        "RomHack.toml",
                    )?)
                    .await?;
                    builder.build().await
                }
            })
        }
        Commands::Apply {
            patch,
            original_game,
            output,
        } => task::block_on::<_, color_eyre::eyre::Result<()>>(async {
            let mut builder = open_config_from_patch(
                std::fs::OpenOptions::new().read(true).open(patch)?,
                async_std::fs::OpenOptions::new()
                    .read(true)
                    .open(original_game)
                    .await?,
                    async_std::fs::OpenOptions::new().write(true).create(true).truncate(true).open(output).await?,
            )
            .await?;
            builder.build().await
        }),
        Commands::New { name } => {
            new(&name)?;
            Ok(())
        }
    }
}
