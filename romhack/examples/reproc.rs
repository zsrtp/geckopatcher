use std::path::PathBuf;

use async_std::io::BufReader;
#[cfg(feature = "log")]
use async_std::io::{prelude::SeekExt, ReadExt};
use clap::{arg, command, Parser, ValueHint};
use geckolib::{
    iso::{disc::DiscType, read::DiscReader, write::DiscWriter},
    vfs::GeckoFS,
};
#[cfg(feature = "progress")]
use romhack::progress;

#[derive(Debug, Parser)]
#[command(author, version)]
/// Reprocess a game file (to the level of individual files)
///
/// Takes a SOURCE file and reprocess it (extract and re-pack)
/// into a DEST file. This extracts the whole FileSystem from
/// the SOURCE file.
struct Args {
    #[arg(value_hint = ValueHint::FilePath)]
    /// Game file to reprocess
    source: PathBuf,
    #[arg(value_hint = ValueHint::AnyPath)]
    /// Where to write the reprocessed file
    dest: PathBuf,
}

// Reprocesses a given iso (load iso in to a FileSystem, then save it back into an other iso)
fn main() -> color_eyre::eyre::Result<()> {
    color_eyre::install()?;
    #[cfg(feature = "log")]
    env_logger::init();
    #[cfg(feature = "progress")]
    progress::init_cli_progress();

    let args = Args::parse();

    futures::executor::block_on(async {
        let f = BufReader::with_capacity(
            0x7C00 * 64 * 8,
            async_std::fs::File::open(args.source).await?,
        );
        // let f = Arc::new(Mutex::new(DiscReader::new(f).await?));
        let mut f = DiscReader::new(f).await?;
        let disc_info = f.get_disc_info();
        #[cfg(feature = "log")]
        {
            f.seek(std::io::SeekFrom::Start(0)).await?;
            let mut buf = vec![0u8; 0x60];
            f.read(&mut buf).await?;
            log::info!(
                "[{}] Game Title: {:02X?}",
                String::from_utf8_lossy(&buf[..6]),
                String::from_utf8_lossy(&buf[0x20..0x60])
                    .split_terminator('\0')
                    .find(|s| !s.is_empty())
                    .expect("This game has no title")
            );
        }
        let mut out: DiscWriter<async_std::fs::File> = DiscWriter::new(
            async_std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .open(args.dest)
                .await?,
            disc_info,
        );
        if let DiscWriter::Wii(wii_out) = out.clone() {
            std::pin::pin!(wii_out).init().await?;
        }

        let mut fs = GeckoFS::parse(f).await?;
        let is_wii = out.get_type() == DiscType::Wii;
        #[cfg(feature = "log")]
        log::info!("Encrypting the ISO");
        fs.serialize(&mut out, is_wii).await?;
        <color_eyre::eyre::Result<()>>::Ok(())
    })?;
    Ok(())
}
