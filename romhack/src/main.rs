use async_std::{fs, task};
use geckolib::IsoBuilder;

#[cfg(feature = "progress")]
mod progress;

fn main() -> color_eyre::eyre::Result<()> {
    color_eyre::install()?;
    #[cfg(feature = "log")]
    env_logger::init();
    #[cfg(feature = "progress")]
    progress::init_cli_progress();

    task::block_on::<_, color_eyre::eyre::Result<()>>(async {
        let iso =
            IsoBuilder::<std::fs::File>::new_patch(toml::from_str(&fs::read_to_string("RomHack.toml").await?)?);
        iso.build_raw().await?;
        Ok(())
    })?;
    Ok(())
}
