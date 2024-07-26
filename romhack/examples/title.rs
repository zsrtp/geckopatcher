use std::io::SeekFrom;

use async_std::path::PathBuf;
use async_std::{
    io::prelude::*,
    task,
};
use clap::{Parser, ValueHint};
use geckolib::iso::disc::DiscType;
use geckolib::iso::read::DiscReader;
use geckolib::vfs::GeckoFS;
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

    task::block_on(async {
        let f = async_std::fs::File::open(args.file).await?;
        let mut f = DiscReader::new(f).await?;
        let is_wii = f.get_type() == DiscType::Wii;

        f.seek(std::io::SeekFrom::Start(0)).await?;
        let mut buf = vec![0u8; 0x60];
        f.read(&mut buf).await?;
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
            let mut fs = GeckoFS::parse(f).await?;
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
                    let mut out_file = async_std::fs::OpenOptions::new()
                        .write(true)
                        .create(true)
                        .open(banner_out_path)
                        .await?;
                    async_std::io::copy(banner, &mut out_file).await?;
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
            map_file.read(&mut buf).await?;
            #[cfg(feature = "log")]
            log::info!("str: {:?}", String::from_utf8_lossy(&buf));
        }
        <color_eyre::eyre::Result<()>>::Ok(())
    })?;
    Ok(())
}
