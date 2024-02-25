#[cfg(feature = "log")]
extern crate log;
#[cfg(feature = "parallel")]
extern crate rayon;
extern crate thiserror;
#[macro_use]
extern crate lazy_static;
extern crate async_std;
extern crate cbc;
extern crate eyre;
extern crate num;
extern crate serde;
extern crate sha1_smol;
#[macro_use]
extern crate static_assertions;

pub mod config;
pub mod crypto;
pub mod iso;
pub(crate) mod logs;
#[cfg(feature = "progress")]
pub mod update;
pub mod vfs;

use config::Config;
use iso::builder::IsoBuilder;
use zip::ZipArchive;

#[cfg(feature = "progress")]
lazy_static! {
    pub static ref UPDATER: std::sync::Arc<std::sync::Mutex<update::Updater<eyre::Report, usize>>> =
        std::sync::Arc::new(std::sync::Mutex::new(update::Updater::default()));
}

/// Open a config from a patch file
pub async fn open_config_from_patch<R: std::io::Read + std::io::Seek>(patch_reader: R) -> Result<IsoBuilder<R>, eyre::Report> {
    let mut zip: ZipArchive<R> = ZipArchive::new(patch_reader)?;

    let config: Config = {
        let toml_file = zip.by_name("RomHack.toml")?;

        toml::from_str(&std::io::read_to_string(toml_file)?)?
    };

    Ok(IsoBuilder::new_with_zip(config, zip))
}
