use async_std::{
    io::{prelude::SeekExt, BufReader, ReadExt},
    sync::{Arc, Mutex},
};
use geckolib::{
    iso::{disc::DiscType, read::DiscReader, write::DiscWriter},
    vfs::GeckoFS,
};
#[cfg(feature = "progress")]
use romhack::progress;

// Reprocesses a given iso (load iso in to a FileSystem, then save it back into an other iso)
fn main() -> color_eyre::eyre::Result<()> {
    color_eyre::install()?;
    #[cfg(feature = "log")]
    env_logger::init();
    #[cfg(feature = "progress")]
    progress::init_cli_progress();

    async_std::task::block_on(async {
        let f = BufReader::with_capacity(
            0x7C00 * 64 * 8,
            async_std::fs::File::open(std::env::args().nth(1).expect("No ISO file were provided"))
                .await?,
        );
        let f = Arc::new(Mutex::new(DiscReader::new(f).await?));
        {
            let mut guard = f.lock_arc().await;
            guard.seek(std::io::SeekFrom::Start(0)).await?;
            let mut buf = vec![0u8; 0x60];
            guard.read(&mut buf).await?;
            log::info!(
                "[{}] Game Title: {:02X?}",
                String::from_utf8_lossy(&buf[..6]),
                String::from_utf8_lossy(&buf[0x20..0x60])
                    .split_terminator('\0')
                    .find(|s| !s.is_empty())
                    .expect("This game has no title")
            );
        }
        let out = {
            let guard = f.lock_arc().await;
            DiscWriter::new(
                async_std::fs::OpenOptions::new()
                    .write(true)
                    .read(true)
                    .create(true)
                    .open(
                        std::env::args()
                            .nth(2)
                            .expect("No output file was provided"),
                    )
                    .await?,
                guard.get_disc_info(),
            )
            .await?
        };

        let mut out = std::pin::pin!(out);
        let fs = GeckoFS::parse(f).await?;
        {
            let mut fs_guard = fs.lock_arc().await;
            let is_wii = out.get_type() == DiscType::Wii;
            fs_guard.serialize(&mut out, is_wii).await?;
            #[cfg(feature = "log")]
            log::info!("Encrypting the ISO");
            out.finalize().await?;
        }
        <color_eyre::eyre::Result<()>>::Ok(())
    })?;
    Ok(())
}
