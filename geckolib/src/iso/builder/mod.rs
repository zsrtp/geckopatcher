#[cfg(not(target_os = "unknown"))]
use async_std::fs::read;
use async_std::fs::OpenOptions;
use async_std::io::{prelude::*, Read as AsyncRead, Seek as AsyncSeek};
use async_std::sync::Mutex;
use eyre::Context;
#[cfg(not(target_os = "unknown"))]
use std::collections::HashMap;
use std::sync::Arc;
#[cfg(not(target_os = "unknown"))]
use std::{
    fs::File as StdFile,
    io::{BufWriter, Read, Seek, Write},
    path::{Path, PathBuf},
};
use zip::ZipArchive;
#[cfg(not(target_os = "unknown"))]
use zip::ZipWriter;

use self::fs_source::FSSource;
use crate::assembler::{Assembler, Instruction};
use crate::banner::Banner;
use crate::config::Config;

use crate::dol::DolFile;
use crate::iso::write::DiscWriter;
use crate::vfs::{self, Directory, GeckoFS};
#[cfg(feature = "progress")]
use crate::UPDATER;
use crate::{framework_map, linker, warn};

use super::disc::DiscType;
use super::read::DiscReader;

mod fs_source;

pub trait Builder {
    type Error;

    fn build(&mut self) -> impl std::future::Future<Output = Result<(), Self::Error>>;
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

fn add_file_to_iso<R: AsyncRead + AsyncSeek + 'static, R2: Read + Seek>(
    iso_path: &String,
    actual_path: &PathBuf,
    iso: &mut Directory<R>,
    files: &mut FSSource<R2>,
) -> eyre::Result<()> {
    if files.is_dir(actual_path) {
        let names = files.get_names(actual_path)?;
        for name in names {
            let iso_path = String::from(iso_path) + &String::from('/') + &name;
            let mut actual_path = actual_path.clone();
            actual_path.push(name);
            add_file_to_iso(&iso_path, &actual_path, iso, files)?;
        }
    } else {
        if let Ok(mut updater) = UPDATER.lock() {
            updater.set_message(format!("{}", iso_path))?;
        }

        let mut file = files.get_file(actual_path)?;
        if let Some(f) = iso
            .resolve_node_mut(&iso_path)
            .and_then(|n| n.as_file_mut())
        {
            let mut buffer = Vec::new();
            file.read_to_end(&mut buffer)?;
            f.set_data(buffer.into_boxed_slice());
        } else {
            let mut p = PathBuf::from(iso_path);
            let file_name = p
                .file_name()
                .expect("File name is invalid")
                .to_string_lossy()
                .into_owned();
            p.pop();
            let dir = iso.mkdirs(p)?;
            let mut data = Vec::new();
            file.read_to_end(&mut data)?;
            dir.add_file(vfs::File::new(
                vfs::FileDataSource::Box(Arc::new(Mutex::new(data.into_boxed_slice()))),
                file_name,
                0,
                0,
                0,
            ));
        }
    }
    Ok(())
}

fn patch_instructions(
    mut original: DolFile,
    intermediate: DolFile,
    instructions: &[Instruction],
) -> eyre::Result<Vec<u8>> {
    original.append(intermediate);
    original
        .patch(instructions)
        .context("Couldn't patch the DOL")?;

    Ok(original.to_bytes())
}

impl<R: Send + Read + Seek> Builder for IsoBuilder<R> {
    type Error = eyre::Report;

    async fn build(&mut self) -> eyre::Result<()> {
        if let Ok(mut updater) = UPDATER.lock() {
            updater.set_message("Loading game...".into())?;
        }

        let reader = DiscReader::new(
            OpenOptions::new()
                .read(true)
                .open(&self.config.src.iso)
                .await?,
        )
        .await?;
        let wii_disc_info = reader.get_disc_info();
        let mut disc = GeckoFS::parse(reader).await?;

        if let Ok(mut updater) = UPDATER.lock() {
            updater.set_title("Replacing files...".into())?;
            updater.set_message("".into())?;
        }

        for (iso_path, actual_path) in &self.config.files {
            add_file_to_iso(iso_path, actual_path, disc.root_mut(), &mut self.fs)?;
        }

        let original_symbols = if let Some(framework_map) = self
            .config
            .src
            .map
            .as_mut()
            .and_then(|m| disc.root_mut().resolve_node_mut(m))
            .and_then(|n| n.as_file_mut())
        {
            if let Ok(mut updater) = UPDATER.lock() {
                updater.set_title("Parsing symbol map...".into())?;
                updater.set_message("".into())?;
            }

            framework_map::parse(framework_map).await?
        } else {
            if let Ok(mut updater) = UPDATER.lock() {
                updater.set_title("[Warning] No symbol map specified or it wasn't found".into())?;
                updater.set_message("".into())?;
            }
            HashMap::new()
        };

        if let Ok(mut updater) = UPDATER.lock() {
            updater.set_title("Linking...".into())?;
            updater.set_message("".into())?;
        }

        let link = &self.config.link;

        let mut libs_to_link = Vec::with_capacity(link.libs.len() + 1);

        for lib_path in &link.libs {
            let file_buf = {
                let mut buf = Vec::new();
                self.fs
                    .get_file(lib_path)
                    .context(format!(
                        "Couldn't load \"{}\". Did you build the project correctly?",
                        lib_path.display()
                    ))?
                    .read_to_end(&mut buf)?;
                buf
            };
            libs_to_link.push(file_buf);
        }

        libs_to_link.push(linker::BASIC_LIB.to_owned());

        let base_address: syn::LitInt =
            syn::parse_str(&link.base).context("Invalid Base Address")?;

        let linked = linker::link(
            &libs_to_link,
            base_address.value() as u32,
            link.entries.clone(),
            &original_symbols,
        )
        .context("Couldn't link the Rom Hack")?;

        if let Ok(mut updater) = UPDATER.lock() {
            updater.set_title("Creating symbol map...".into())?;
            updater.set_message("".into())?;
        }

        let instructions = if let Some(patch) = self.config.src.patch.take() {
            if let Ok(mut updater) = UPDATER.lock() {
                updater.set_message("Parsing patch".into())?;
            }

            let mut asm = self.fs.get_file(&patch).context(format!(
                "Couldn't read the patch file \"{}\".",
                patch.display()
            ))?;

            let mut buf = String::new();
            asm.read_to_string(&mut buf)?;

            let lines = &buf.lines().collect::<Vec<_>>();

            let mut assembler = Assembler::new(linked.symbol_table, &original_symbols);
            assembler
                .assemble_all_lines(lines)
                .context("Couldn't assemble the patch file lines")?
        } else {
            Vec::new()
        };

        {
            if let Ok(mut updater) = UPDATER.lock() {
                updater.set_title("Patching game...".into())?;
                updater.set_message("".into())?;
            }

            let main_dol = disc
                .sys_mut()
                .get_file_mut("Start.dol")
                .context("Dol file not found")?;

            let original = DolFile::parse(main_dol).await?;
            main_dol.set_data(
                patch_instructions(original, linked.dol, &instructions)
                    .context("Couldn't patch the game")?
                    .into(),
            );
        }

        if wii_disc_info.is_none() {
            if let Ok(mut updater) = UPDATER.lock() {
                updater.set_title("Patching banner...".into())?;
                updater.set_message("".into())?;
            }

            if let Ok(banner_file) = disc.root_mut().get_file_mut("opening.bnr") {
                // TODO Not always true
                let is_japanese = true;
                let mut buf = Vec::new();
                banner_file.read_to_end(&mut buf).await?;
                let mut banner =
                    Banner::parse(is_japanese, &buf).context("Couldn't parse the banner")?;

                if let Some(game_name) = self.config.info.game_name.take() {
                    banner.game_name = game_name;
                }
                if let Some(developer_name) = self.config.info.developer_name.take() {
                    banner.developer_name = developer_name;
                }
                if let Some(full_game_name) = self.config.info.full_game_name.take() {
                    banner.full_game_name = full_game_name;
                }
                if let Some(full_developer_name) = self.config.info.full_developer_name.take() {
                    banner.full_developer_name = full_developer_name;
                }
                if let Some(game_description) = self.config.info.description.take() {
                    banner.game_description = game_description;
                }
                if let Some(image_path) = self.config.info.image.take() {
                    let image = {
                        let mut img_file = self.fs.get_file(image_path)?;
                        let mut buf = Vec::new();
                        img_file.read_to_end(&mut buf)?;
                        image::load_from_memory(&buf)
                            .context("Couldn't open the banner replacement image")?
                            .to_rgba8()
                    };
                    banner.image.copy_from_slice(&image);
                }
                banner_file.set_data(banner.to_bytes(is_japanese).to_vec().into());
            } else {
                warn!("No banner to patch");
                if let Ok(mut updater) = UPDATER.lock() {
                    updater.set_title("Patching banner...".into())?;
                    updater.set_message("".into())?;
                }
            }
        }

        // Finalize disc and write it back into a file

        let out = {
            DiscWriter::new(
                // async_std::io::BufWriter::with_capacity(
                //     1 << 22u8,
                    async_std::fs::OpenOptions::new()
                        .write(true)
                        .read(true)
                        .create(true)
                        .open(self.config.build.iso.clone())
                        .await?,
                // ),
                wii_disc_info,
            )
            .await?
        };

        if let Ok(mut updater) = UPDATER.lock() {
            updater.finish()?;
            updater.reset()?;
        }
        let mut out = std::pin::pin!(out);
        let is_wii = out.get_type() == DiscType::Wii;
        disc.serialize(&mut out, is_wii).await?;
        out.finalize().await?;

        Ok(())
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
fn write_file_to_zip<
    R: Write + Seek,
    S: Into<Box<str>> + ToOwned<Owned = SToOwned>,
    SToOwned: Into<Box<str>>,
>(
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

        {
            let libs = &mut config.link.libs;
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
