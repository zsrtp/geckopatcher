use async_std::{fs, task};
use geckolib::IsoBuilder;

fn main() -> color_eyre::eyre::Result<()> {
    color_eyre::install()?;
    #[cfg(feature = "log")]
    {
        let mut logger = env_logger::Builder::from_default_env();
        logger.filter_level(log::LevelFilter::Info).init();
    }

    task::block_on::<_, color_eyre::eyre::Result<()>>(async {
        let iso =
            IsoBuilder::new_patch(toml::from_str(&fs::read_to_string("RomHack.toml").await?)?)
                .await;
        iso.build().await?;
        Ok(())
    })?;
    Ok(())
}
