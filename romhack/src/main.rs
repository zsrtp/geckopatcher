use async_std::{fs, task};
use geckolib::iso::builder::{PatchBuilder, Builder};

#[cfg(feature = "progress")]
mod progress;

fn main() -> color_eyre::eyre::Result<()> {
    color_eyre::install()?;
    #[cfg(feature = "log")]
    env_logger::init();
    #[cfg(feature = "progress")]
    progress::init_cli_progress();

    task::block_on::<_, color_eyre::eyre::Result<()>>(async {
        let mut iso =
            PatchBuilder::with_config(toml::from_str(&fs::read_to_string("RomHack.toml").await?)?);
        iso.build()?;
        Ok(())
    })
}
