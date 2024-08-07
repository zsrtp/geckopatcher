#[cfg(not(target_os = "unknown"))]
use async_std::fs::read;
use async_std::io::{prelude::*, Read as AsyncRead, Seek as AsyncSeek};
use eyre::Context;
use futures::AsyncWrite;
use std::collections::HashMap;
#[cfg(not(target_os = "unknown"))]
use std::{
    fs::File as StdFile,
    io::{BufWriter, Write},
};
use std::io::{Read, Seek};
use std::path::{Path, PathBuf};
use zip::ZipArchive;
#[cfg(not(target_os = "unknown"))]
use zip::ZipWriter;

use self::fs_source::FSSource;
use crate::patch::assembler::{Assembler, Instruction};
use crate::patch::banner::Banner;
use crate::config::Config;

use crate::patch::dol::DolFile;
use crate::iso::write::DiscWriter;
use crate::vfs::{self, Directory, GeckoFS};
#[cfg(feature = "progress")]
use crate::UPDATER;
use crate::{patch::{framework_map, linker}, warn};

use super::{disc::DiscType, read::DiscReader};

mod fs_source;

pub trait Builder {
    type Error;

    fn build(&mut self) -> impl std::future::Future<Output = Result<(), Self::Error>>;
}

/// A builder for creating an ISO
pub struct IsoBuilder<R1, R2, W> {
    pub config: Config,
    fs: FSSource<R1>,
    gfs: GeckoFS<R2>,
    reader: DiscReader<R2>,
    writer: W,
}

impl<RConfig, RDisc, W> IsoBuilder<RConfig, RDisc, W> {
    pub fn new_with_zip(
        config: Config,
        zip: ZipArchive<RConfig>,
        gfs: GeckoFS<RDisc>,
        reader: DiscReader<RDisc>,
        writer: W,
    ) -> Self {
        Self::internal_new(config, FSSource::Zip(Box::new(zip)), gfs, reader, writer)
    }

    #[cfg(not(target_os = "unknown"))]
    pub fn new_with_fs<P: AsRef<Path>>(
        config: Config,
        path: P,
        gfs: GeckoFS<RDisc>,
        reader: DiscReader<RDisc>,
        writer: W,
    ) -> Self {
        Self::internal_new(
            config,
            FSSource::FS(path.as_ref().to_path_buf()),
            gfs,
            reader,
            writer,
        )
    }

    fn internal_new(
        config: Config,
        fs: FSSource<RConfig>,
        gfs: GeckoFS<RDisc>,
        reader: DiscReader<RDisc>,
        writer: W,
    ) -> Self {
        Self {
            config,
            fs,
            gfs,
            reader,
            writer,
        }
    }

    pub fn config_mut(&mut self) -> &mut Config {
        &mut self.config
    }

    pub fn config(&self) -> &Config {
        &self.config
    }
}

fn add_file_to_iso<R: AsyncRead + AsyncSeek + 'static, R2: std::io::Read + std::io::Seek, P: AsRef<Path>>(
    iso_path: &String,
    actual_path: &P,
    iso: &mut Directory<R>,
    files: &mut FSSource<R2>,
) -> eyre::Result<()> {
    if files.is_file(actual_path) {
        if let Ok(mut updater) = UPDATER.lock() {
            updater.set_message(iso_path.to_string())?;
        }

        let mut file = files.get_file(actual_path)?;
        if let Some(f) = iso.resolve_node_mut(iso_path).and_then(|n| n.as_file_mut()) {
            let mut buffer = Vec::new();
            file.read_to_end(&mut buffer)?;
            f.set_data(buffer.into_boxed_slice())?;
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
            dir.add_file(vfs::File::new(vfs::FileDataSource::Box {
                data: data.into_boxed_slice(),
                name: file_name,
            }));
        }
    }
    Ok(())
}

#[cfg(not(target_os = "unknown"))]
fn add_node_to_iso<R: AsyncRead + AsyncSeek + 'static, R2: Read + Seek>(
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
            add_node_to_iso(&iso_path, &actual_path, iso, files)?;
        }
    } else {
        add_file_to_iso(iso_path, actual_path, iso, files)?;
    }
    Ok(())
}

fn patch_instructions(
    mut original: DolFile,
    intermediate: Option<DolFile>,
    instructions: &[Instruction],
) -> eyre::Result<Vec<u8>> {
    if let Some(intermediate) = intermediate {
        original.append(intermediate);
    }
    original
        .patch(instructions)
        .context("Couldn't patch the DOL")?;

    Ok(original.to_bytes())
}

impl<RConfig, RDisc, W> Builder for IsoBuilder<RConfig, RDisc, W>
where
    RConfig: Read + Seek,
    RDisc: AsyncRead + AsyncSeek + Clone + Unpin + 'static,
    W: AsyncWrite + AsyncSeek + Clone + Unpin,
{
    type Error = eyre::Report;

    async fn build(&mut self) -> eyre::Result<()> {
        #[cfg(feature = "progress")]
        if let Ok(mut updater) = UPDATER.lock() {
            updater.set_message("Loading game...".into())?;
        }

        let disc = &mut self.gfs;

        #[cfg(feature = "progress")]
        if let Ok(mut updater) = UPDATER.lock() {
            updater.set_message("".into())?;
            updater.set_title("Replacing files...".into())?;
        }

        for (iso_path, actual_path) in &self.config.files {
            #[cfg(target_os = "unknown")]
            add_file_to_iso(iso_path, actual_path, disc.root_mut(), &mut self.fs)?;
            #[cfg(not(target_os = "unknown"))]
            add_node_to_iso(iso_path, actual_path, disc.root_mut(), &mut self.fs)?;
        }

        let original_symbols = if let Some(framework_map) = self
            .config
            .src
            .map
            .as_mut()
            .and_then(|m| disc.root_mut().resolve_node_mut(m))
            .and_then(|n| n.as_file_mut())
        {
            #[cfg(feature = "progress")]
            if let Ok(mut updater) = UPDATER.lock() {
                updater.set_message("".into())?;
                updater.set_title("Parsing symbol map...".into())?;
            }

            framework_map::parse(framework_map).await?
        } else {
            #[cfg(feature = "progress")]
            if let Ok(mut updater) = UPDATER.lock() {
                updater.set_message("".into())?;
                updater.set_title("[Warning] No symbol map specified or it wasn't found".into())?;
            }
            HashMap::new()
        };

        #[cfg(feature = "progress")]
        if let Ok(mut updater) = UPDATER.lock() {
            updater.set_message("".into())?;
            updater.set_title("Linking...".into())?;
        }

        let mut libs_to_link;
        let linked = if let Some(link) = &self.config.link {
            libs_to_link = Vec::with_capacity(link.libs.len() + 1);

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
                base_address.base10_parse::<u32>().context("Invalid Base Address")?,
                link.entries.clone(),
                &original_symbols,
            )
            .context("Couldn't link the Rom Hack")?;

            Some(linked)
        } else {
            None
        };

        #[cfg(feature = "progress")]
        if let Ok(mut updater) = UPDATER.lock() {
            updater.set_message("".into())?;
            updater.set_title("Creating symbol map...".into())?;
        }

        let instructions = if let Some(patch) = self.config.src.patch.take() {
            #[cfg(feature = "progress")]
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

            let mut assembler = Assembler::new(linked.as_ref().map(|l| l.symbol_table.clone()), &original_symbols);
            assembler
                .assemble_all_lines(lines)
                .context("Couldn't assemble the patch file lines")?
        } else {
            Vec::new()
        };

        {
            #[cfg(feature = "progress")]
            if let Ok(mut updater) = UPDATER.lock() {
                updater.set_message("".into())?;
                updater.set_title("Patching game...".into())?;
            }

            let main_dol = disc
                .sys_mut()
                .get_file_mut("Start.dol")
                .context("Dol file not found")?;

            let original = DolFile::parse(main_dol).await?;
            main_dol.set_data(
                patch_instructions(original, linked.map(|l| l.dol), &instructions)
                    .context("Couldn't patch the game")?
                    .into(),
            )?;
        }

        if self.reader.get_type() == DiscType::Gamecube {
            #[cfg(feature = "progress")]
            if let Ok(mut updater) = UPDATER.lock() {
                updater.set_message("".into())?;
                updater.set_title("Patching banner...".into())?;
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
                banner_file.set_data(banner.to_bytes(is_japanese).to_vec().into())?;
            } else {
                warn!("No banner to patch");
                #[cfg(feature = "progress")]
                if let Ok(mut updater) = UPDATER.lock() {
                    updater.set_message("".into())?;
                    updater.set_title("Patching banner...".into())?;
                }
            }
        }

        // Finalize disc and write it back into a file

        let out: DiscWriter<W> = DiscWriter::from_reader(self.writer.clone(), &self.reader);
        std::pin::pin!(out.clone()).init().await?;
        // let out = DiscWriter::Gamecube(self.writer.clone());

        let mut out = std::pin::pin!(out);
        disc.serialize(&mut out).await?;

        #[cfg(feature = "progress")]
        if let Ok(mut updater) = UPDATER.lock() {
            updater.set_title("Finished".into())?;
            updater.finish()?;
        }

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
    S: Into<Box<str>> + ToOwned<Owned = SToOwned> + std::fmt::Display,
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
            updater.set_message("".into())?;
            updater.set_title("Storing replacement files...".into())?;
        }

        let mut new_map = HashMap::new();
        let mut index = 0;
        for (iso_path, actual_path) in config.files.iter() {
            add_entry_to_zip(&mut index, iso_path, actual_path, &mut zip, &mut new_map)?;
        }
        config.files = new_map;

        if let Some(link) = &mut config.link {
            crate::info!("Storing libraries");
    
            #[cfg(feature = "progress")]
            if let Ok(mut updater) = UPDATER.lock() {
                updater.set_message("".into())?;
                updater.set_title("Storing libraries...".into())?;
            }
    
            let libs = &mut link.libs;
            if !libs.is_empty() {
                write_file_to_zip(
                    &mut zip,
                    "libcompiled.a",
                    &async_std::fs::read(libs.first().unwrap()).await?,
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
        }

        if let Some(path) = &mut config.src.patch {
            crate::info!("Storing patch.asm");

            #[cfg(feature = "progress")]
            if let Ok(mut updater) = UPDATER.lock() {
                updater.set_message("Storing patch.asm...".into())?;
            }

            write_file_to_zip(&mut zip, "patch.asm", &read(path).await?)?;
        }

        if let Some(path) = &config.info.image {
            crate::info!("Storing banner");

            #[cfg(feature = "progress")]
            if let Ok(mut updater) = UPDATER.lock() {
                updater.set_message("".into())?;
                updater.set_title("Storing banner...".into())?;
            }

            write_file_to_zip(&mut zip, "banner.dat", &read(path).await?)?;
        }

        crate::info!("Storing patch index");

        #[cfg(feature = "progress")]
        if let Ok(mut updater) = UPDATER.lock() {
            updater.set_message("".into())?;
            updater.set_title("Storing patch index...".into())?;
        }

        config.src.iso = PathBuf::new();
        config.build = Default::default();
        write_file_to_zip(
            &mut zip,
            "RomHack.toml",
            toml::to_string(&config)?.as_bytes(),
        )?;

        zip.finish()?;

        #[cfg(feature = "progress")]
        if let Ok(mut updater) = UPDATER.lock() {
            updater.set_title("Finished".into())?;
            updater.finish()?;
        }

        Ok(())
    }
}
