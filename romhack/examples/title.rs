use std::io::SeekFrom;
use std::path::PathBuf;

use clap::{Parser, ValueHint};
use futures::io::{AsyncReadExt, AsyncSeekExt};
use futures_lite::future::block_on;
use geckolib::async_file::AsyncFile;
use geckolib::iso::disc::DiscType;
use geckolib::iso::read::DiscReader;
use geckolib::vfs::GeckoFSLegacy;
#[cfg(feature = "progress")]
use romhack::progress::init_cli_progress;

#[derive(Debug, Parser)]
#[command(author, version, about, long_about = None)]
/// Extract Title information from the FILE.
struct Args {
    #[arg(value_hint = ValueHint::FilePath)]
    /// The file to extract the info from
    file: PathBuf,
    #[arg(value_hint = ValueHint::AnyPath, )]
    /// Where to output the opening banner file
    banner_out: Option<PathBuf>,
}

fn main() -> color_eyre::eyre::Result<()> {
    color_eyre::install()?;
    #[cfg(feature = "log")]
    env_logger::init();
    #[cfg(feature = "progress")]
    init_cli_progress();

    let args = Args::parse();

    block_on(async {
        let f = AsyncFile::new(std::fs::File::open(args.file)?);
        let mut f = DiscReader::new(f).await?;
        let is_wii = f.get_type() == DiscType::Wii;

        f.seek(std::io::SeekFrom::Start(0)).await?;
        let mut buf = vec![0u8; 0x60];
        let _ = f.read(&mut buf).await?;
        #[cfg(feature = "log")]
        log::info!(
            "[{}] Game Title: {:02X?}",
            String::from_utf8_lossy(&buf[..6]),
            String::from_utf8_lossy(&buf[0x20..0x60])
                .split_terminator('\0')
                .next()
                .expect("This game has no title")
        );
        {
            let mut fs = GeckoFSLegacy::parse(f).await?;
            let file = fs.sys_mut().get_file_mut("Start.dol")?;
            let size = file.seek(SeekFrom::End(0)).await? as usize;
            file.seek(SeekFrom::Start(0)).await?;
            let mut buf = vec![0u8; size];
            #[cfg(feature = "log")]
            {
                let num_read = file.read(&mut buf).await?;
                log::info!("file size: 0x{:08X}; read amount: 0x{:08X}", size, num_read);
                log::info!(
                    "Main dol: {}",
                    buf.iter()
                        .take(0x110)
                        .map(|x| format!("{:02X}", x))
                        .collect::<Vec<String>>()
                        .join("")
                );
                log::info!(
                    "Has banner: {:?}",
                    fs.root_mut().get_file("opening.bnr").is_ok()
                );
            }
            if let Some(banner_out_path) = args.banner_out {
                if let Ok(banner) = fs.root_mut().get_file_mut("opening.bnr") {
                    let mut out_file = AsyncFile::new(
                        std::fs::OpenOptions::new()
                            .write(true)
                            .create(true)
                            .truncate(true)
                            .open(banner_out_path)?,
                    );
                    futures::io::copy(banner, &mut out_file).await?;
                }
            }
            buf.resize(32, 0);
            buf.fill(0);
            let map_file = fs.root_mut().get_file_mut(if is_wii {
                "map/Rfinal/Release/RframeworkF.map"
            } else {
                "map/Final/Release/frameworkF.map"
            })?;
            map_file.seek(SeekFrom::Start(0)).await?;
            let _ = map_file.read(&mut buf).await?;
            #[cfg(feature = "log")]
            log::info!("str: {:?}", String::from_utf8_lossy(&buf));
        }
        <color_eyre::eyre::Result<()>>::Ok(())
    })?;
    Ok(())
}
