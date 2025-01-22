use std::{path::PathBuf, pin::pin};

use async_std::io::ReadExt;
use clap::{command, Parser, ValueHint};
use color_eyre::eyre::eyre;
use geckolib::{
    iso::{
        disc::{WiiDisc, WiiDiscHeader}, read::DiscReader, write::DiscWriter
    },
    vfs::{self, GeckoFS},
};
#[cfg(feature = "progress")]
use romhack::progress;

#[cfg(feature = "progress")]
use geckolib::{update::UpdaterType, UPDATER};

#[derive(Debug, Parser)]
#[command(author, version, about, long_about = None)]
/// Extract Title information from the FILE.
struct Args {
    #[arg(value_hint = ValueHint::FilePath)]
    /// The file to extract the info from
    input: PathBuf,
    #[arg(value_hint = ValueHint::AnyPath)]
    /// Where to output the empty ISO image
    output: PathBuf,
    /// New Title for the ISO image
    title: Option<String>,
}

// Generates an valid empty ISO
fn main() -> color_eyre::eyre::Result<()> {
    color_eyre::install()?;
    #[cfg(feature = "log")]
    env_logger::init();
    #[cfg(feature = "progress")]
    progress::init_cli_progress();

    let args = Args::parse();

    if let Some(new_title) = args.title.as_ref() {
        if new_title.len() > 64 {
            return Err(eyre!("New title \"{}\" is too long.", new_title))
        }
    }

    #[cfg(feature = "progress")]
    if let Ok(mut updater) = UPDATER.lock() {
        updater.set_type(UpdaterType::Spinner)?;
        updater.init(Some(4))?;
        updater.set_title("Initializing...".into())?;
    }

    async_std::task::block_on(async {
        let mut new_title: String = args.title.unwrap_or("Empty Wii Disk".into());
        if new_title.len() < 64 {
            new_title.push('\0');
        }
        let new_title = new_title.as_bytes();
        let reader =
            DiscReader::new(async_std::fs::File::open(args.input).await?)
                .await?;
        let disc = reader.get_disc_info();
        let new_disc = disc.map(|disc| {
            let mut game_title = disc.disc_header.game_title;
            game_title[..new_title.len()].copy_from_slice(new_title);
            WiiDisc {
                disc_header: WiiDiscHeader {
                    game_code: [b'0', b'0'],
                    game_title,
                    ..disc.disc_header
                },
                disc_region: disc.disc_region,
                partitions: disc.partitions.clone(),
            }
        });
        let mut gfs = GeckoFS::parse(reader).await?;
        let children: Vec<_> = gfs.root().iter().map(|item| item.name()).collect();
        for item in children {
            gfs.root_mut().rm(item)?;
        }
        gfs.root_mut()
            .add_file(vfs::File::new(vfs::FileDataSource::Box {
                data: "Hello, World!\n".as_bytes().to_vec().into_boxed_slice(),
                name: "test.txt".into(),
            }));
        let Ok(iso_hdr) = gfs.sys_mut().get_file_mut("iso.hdr") else {
            return Err(eyre!("No \"iso.hdr\" in the system section of the disc."));
        };
        let mut header = vec![0u8; iso_hdr.len()?];
        iso_hdr.read_to_end(&mut header);
        header[1..=2].copy_from_slice(b"00");
        header[0x20..0x20+new_title.len()].copy_from_slice(new_title);
        iso_hdr.set_data(header.into_boxed_slice())?;
        let writer = DiscWriter::new(
            async_std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .open(args.output)
                .await?,
                new_disc,
        );
        pin!(writer.clone()).init().await?;
        let mut writer = pin!(writer);
        gfs.serialize(&mut writer).await?;

        #[cfg(feature = "progress")]
        if let Ok(mut updater) = UPDATER.lock() {
            updater.set_title("Finished".into())?;
            updater.finish()?;
        }

        <color_eyre::eyre::Result<()>>::Ok(())
    })?;
    Ok(())
}
