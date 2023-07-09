#[cfg(feature = "log")]
extern crate log;
#[cfg(not(target = "wasm32"))]
extern crate rayon;
extern crate thiserror;
#[cfg(feature = "web")]
extern crate wasm_bindgen;
#[macro_use]
extern crate lazy_static;
extern crate async_std;
extern crate block_modes;
extern crate eyre;
extern crate serde;
extern crate sha1_smol;

pub mod config;
pub mod crypto;
pub mod iso;
pub(crate) mod logs;
pub mod vfs;

use config::Config;
use eyre::Result;
use std::collections::HashMap;
use std::io::{Seek, Write};
use std::path::PathBuf;
use std::{fs::File, io::BufWriter};
use zip::{write::FileOptions, ZipWriter};

pub enum IsoBuilder {
    Raw { config: Config },
    Patch { config: Config },
}

#[cfg(not(feature = "web"))]
fn write_file_to_zip<R: Write + Seek, S: Into<String>>(
    zip: &mut ZipWriter<R>,
    filename: S,
    data: &[u8],
) -> Result<()> {
    zip.start_file(filename, FileOptions::default())?;
    zip.write_all(data)?;
    Ok(())
}

#[cfg(not(feature = "web"))]
fn add_file_to_zip(
    index: usize,
    iso_path: &String,
    actual_path: &PathBuf,
    zip: &mut ZipWriter<BufWriter<File>>,
    new_map: &mut HashMap<String, PathBuf>,
) -> Result<()> {
    let zip_path = format!("replace{}.dat", index);
    new_map.insert(iso_path.clone(), PathBuf::from(&zip_path));
    write_file_to_zip(zip, zip_path, &std::fs::read(actual_path)?)?;
    Ok(())
}

#[cfg(not(feature = "web"))]
fn add_entry_to_zip(
    index: &mut usize,
    iso_path: &String,
    actual_path: &PathBuf,
    zip: &mut ZipWriter<BufWriter<File>>,
    new_map: &mut HashMap<String, PathBuf>,
) -> Result<()> {
    if actual_path.is_file() {
        *index = *index + 1;
        add_file_to_zip(*index, iso_path, actual_path, zip, new_map)?;
    } else if actual_path.is_dir() {
        for entry in std::fs::read_dir(actual_path)? {
            let entry = entry?;
            let entry_path = entry.path();
            let file_name = entry_path.file_name().expect("Entry has no name");
            let iso_path = String::from(iso_path)
                + &String::from('/')
                + &String::from(file_name.to_str().unwrap());
            add_entry_to_zip(index, &iso_path, &entry_path, zip, new_map)?;
        }
    }
    Ok(())
}

impl IsoBuilder {
    pub async fn new_raw(config: Config) -> Self {
        Self::Raw { config }
    }

    pub async fn new_patch(config: Config) -> Self {
        Self::Patch { config }
    }

    pub async fn build(self) -> Result<()> {
        match self {
            Self::Raw { config } => {
                Self::build_raw(config).await?;
            }
            Self::Patch { config } => {
                Self::build_patch(config).await?;
            }
        };
        Ok(())
    }

    #[cfg(not(feature = "web"))]
    async fn build_raw(mut _config: Config) -> Result<()> {
        Ok(())
    }

    #[cfg(not(feature = "web"))]
    async fn build_patch(mut config: Config) -> Result<()> {
        crate::info!("Creating patch file");
        config.build.iso.set_extension("patch");
        let mut zip = ZipWriter::new(BufWriter::new(File::create(&config.build.iso)?));
        crate::info!("Storing replacement files");

        let mut new_map = HashMap::new();
        let mut index = 0;
        for (iso_path, actual_path) in config.files.iter() {
            add_entry_to_zip(&mut index, iso_path, actual_path, &mut zip, &mut new_map)?;
        }
        config.files = new_map;

        crate::info!("Storing libraries");

        if let Some(link) = &mut config.link {
            if let Some(libs) = &mut link.libs {
                if libs.is_empty() {
                    return Err(eyre::eyre!("No libraries suplied"));
                }
                write_file_to_zip(
                    &mut zip,
                    "libcompiled.a",
                    &async_std::fs::read(libs.get(0).unwrap()).await?,
                )?;
                libs.remove(0);
                let mut modified_libs = Vec::new();
                for (index, lib_path) in libs.iter().enumerate() {
                    let zip_path = format!("lib{}.a", index);

                    crate::info!("Storing {:?} as {}", lib_path, zip_path);

                    modified_libs.push(zip_path.clone());
                    write_file_to_zip(&mut zip, zip_path, &async_std::fs::read(lib_path).await?)?;
                }

                if let Some(path) = &mut config.src.patch {
                    crate::info!("Storing patch.asm");
                    write_file_to_zip(&mut zip, "patch.asm", &async_std::fs::read(path).await?)?;
                }
            } else {
                return Err(eyre::eyre!("No libraries suplied"));
            }
        }

        if let Some(path) = &config.info.image {
            crate::info!("Storing banner");
            write_file_to_zip(&mut zip, "banner.dat", &async_std::fs::read(path).await?)?;
        }

        crate::info!("Storing patch index");

        config.src.iso = PathBuf::new();
        config.build = Default::default();
        write_file_to_zip(
            &mut zip,
            "RomHack.toml",
            &toml::to_string(&config)?.as_bytes(),
        )?;

        Ok(())
    }
}
