#[cfg(not(target_os = "unknown"))]
use async_std::fs::read;
use zip::ZipArchive;
#[cfg(not(target_os = "unknown"))]
use std::collections::HashMap;
#[cfg(not(target_os = "unknown"))]
use std::{
    fs::File as StdFile,
    io::{BufWriter, Seek, Write},
    path::{Path, PathBuf},
};
#[cfg(not(target_os = "unknown"))]
use zip::ZipWriter;

use self::fs_source::FSSource;
use crate::config::Config;

#[cfg(feature = "progress")]
use crate::UPDATER;

mod fs_source;

pub trait Builder {
    type Error;

    fn build(&mut self) -> impl std::future::Future<Output = Result<(), Self::Error>> + Send;
}

/// A builder for creating an ISO
pub struct IsoBuilder<R> {
    config: Config,
    fs: FSSource<R>,
}

impl<R> IsoBuilder<R> {
    pub fn new_with_zip(config: Config, zip: ZipArchive<R>) -> Self {
        Self::internal_new(config, FSSource::Zip(Box::new(zip)))
    }

    #[cfg(not(target_os = "unknown"))]
    pub fn new_with_fs<P: AsRef<Path>>(config: Config, path: P) -> Self {
        Self::internal_new(config, FSSource::FS(path.as_ref().to_path_buf()))
    }

    fn internal_new(config: Config, fs: FSSource<R>) -> Self {
        Self { config, fs }
    }

    pub fn config_mut(&mut self) -> &mut Config {
        &mut self.config
    }

    pub fn config(&self) -> &Config {
        &self.config
    }
}

impl<R: Send> Builder for IsoBuilder<R> {
    type Error = eyre::Report;

    async fn build(&mut self) -> eyre::Result<()> {
        todo!()
    }
}

#[cfg(not(target_os = "unknown"))]
pub struct PatchBuilder {
    config: Config,
}

#[cfg(not(target_os = "unknown"))]
impl PatchBuilder {
    pub fn with_config(config: Config) -> Self {
        Self { config }
    }
}

#[cfg(not(target_os = "unknown"))]
fn write_file_to_zip<R: Write + Seek, S: Into<Box<str>> + ToOwned<Owned = SToOwned>, SToOwned: Into<Box<str>>>(
    zip: &mut ZipWriter<R>,
    filename: S,
    data: &[u8],
) -> eyre::Result<()> {
    use zip::write::FileOptions;

    zip.start_file(filename, FileOptions::<()>::default())?;
    zip.write_all(data)?;
    Ok(())
}

#[cfg(not(target_os = "unknown"))]
fn add_file_to_zip(
    index: usize,
    iso_path: &str,
    actual_path: &PathBuf,
    zip: &mut ZipWriter<BufWriter<StdFile>>,
    new_map: &mut HashMap<String, PathBuf>,
) -> eyre::Result<()> {
    let zip_path = format!("replace{}.dat", index);
    new_map.insert(iso_path.to_owned(), PathBuf::from(&zip_path));
    write_file_to_zip(zip, zip_path, &std::fs::read(actual_path)?)?;
    Ok(())
}

#[cfg(not(target_os = "unknown"))]
fn add_entry_to_zip(
    index: &mut usize,
    iso_path: &String,
    actual_path: &PathBuf,
    zip: &mut ZipWriter<BufWriter<StdFile>>,
    new_map: &mut HashMap<String, PathBuf>,
) -> eyre::Result<()> {
    if actual_path.is_file() {
        *index += 1;
        add_file_to_zip(*index, iso_path, actual_path, zip, new_map)?;

        #[cfg(feature = "progress")]
        if let Ok(mut updater) = UPDATER.lock() {
            updater.set_message(format!("Storing {:?} as {}...", actual_path, iso_path))?;
        }
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

#[cfg(not(target_os = "unknown"))]
impl Builder for PatchBuilder {
    type Error = eyre::Report;

    async fn build(&mut self) -> eyre::Result<()> {
        let config = &mut self.config;

        #[cfg(feature = "progress")]
        if let Ok(mut updater) = UPDATER.lock() {
            updater.set_message("Creating patch file...".into())?;
        }

        crate::info!("Creating patch file");
        config.build.iso.set_extension("patch");
        let mut zip = ZipWriter::new(BufWriter::new(StdFile::create(&config.build.iso)?));
        crate::info!("Storing replacement files");

        #[cfg(feature = "progress")]
        if let Ok(mut updater) = UPDATER.lock() {
            updater.set_title("Storing replacement files...".into())?;
            updater.set_message("".into())?;
        }

        let mut new_map = HashMap::new();
        let mut index = 0;
        for (iso_path, actual_path) in config.files.iter() {
            add_entry_to_zip(&mut index, iso_path, actual_path, &mut zip, &mut new_map)?;
        }
        config.files = new_map;

        crate::info!("Storing libraries");

        #[cfg(feature = "progress")]
        if let Ok(mut updater) = UPDATER.lock() {
            updater.set_title("Storing libraries...".into())?;
            updater.set_message("".into())?;
        }

        if let Some(link) = &mut config.link {
            if let Some(libs) = &mut link.libs {
                if !libs.is_empty() {
                    write_file_to_zip(
                        &mut zip,
                        "libcompiled.a",
                        &async_std::fs::read(libs.get(0).unwrap()).await?,
                    )?;
                    libs.remove(0);
                }
                let mut modified_libs = Vec::new();
                for (index, lib_path) in libs.iter().enumerate() {
                    let zip_path = format!("lib{}.a", index);

                    crate::info!("Storing {:?} as {}", lib_path, zip_path);

                    #[cfg(feature = "progress")]
                    if let Ok(mut updater) = UPDATER.lock() {
                        updater.set_message(format!("Storing {:?} as {}", lib_path, zip_path))?;
                    }

                    modified_libs.push(zip_path.clone());
                    write_file_to_zip(&mut zip, zip_path, &read(lib_path).await?)?;
                }

                if let Some(path) = &mut config.src.patch {
                    crate::info!("Storing patch.asm");

                    #[cfg(feature = "progress")]
                    if let Ok(mut updater) = UPDATER.lock() {
                        updater.set_message("Storing patch.asm...".into())?;
                    }

                    write_file_to_zip(&mut zip, "patch.asm", &read(path).await?)?;
                }
            }
        }

        if let Some(path) = &config.info.image {
            crate::info!("Storing banner");

            #[cfg(feature = "progress")]
            if let Ok(mut updater) = UPDATER.lock() {
                updater.set_title("Storing banner...".into())?;
                updater.set_message("".into())?;
            }

            write_file_to_zip(&mut zip, "banner.dat", &read(path).await?)?;
        }

        crate::info!("Storing patch index");

        #[cfg(feature = "progress")]
        if let Ok(mut updater) = UPDATER.lock() {
            updater.set_title("Storing patch index...".into())?;
            updater.set_message("".into())?;
        }

        config.src.iso = PathBuf::new();
        config.build = Default::default();
        write_file_to_zip(
            &mut zip,
            "RomHack.toml",
            toml::to_string(&config)?.as_bytes(),
        )?;

        zip.finish()?;

        Ok(())
    }
}
