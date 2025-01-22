use async_std::task;
use clap::Parser;
use geckolib::iso::builder::PatchBuilder;
use geckolib::parse_config;
use geckolib::{iso::builder::Builder, new, open_config_from_fs_iso, open_config_from_patch};

#[cfg(feature = "progress")]
use geckolib::{update::UpdaterType, UPDATER};

mod progress;

use romhack::cli::{Cli, Commands};

fn main() -> color_eyre::eyre::Result<()> {
    color_eyre::install()?;
    #[cfg(feature = "log")]
    env_logger::init();

    let args = Cli::parse();

    if !args.no_progress {
        progress::init_cli_progress();
    }

    #[cfg(feature = "progress")]
    if let Ok(mut updater) = UPDATER.lock() {
        updater.set_type(UpdaterType::Spinner)?;
        updater.init(Some(4))?;
        updater.set_title("Initializing...".into())?;
    }

    match args.cmd {
        Commands::Build { patch, raw: _ } => {
            task::block_on::<_, color_eyre::eyre::Result<()>>(async {
                if patch {
                    let config = parse_config(std::fs::File::open("RomHack.toml")?)?;
                    let mut builder = PatchBuilder::with_config(config);
                    builder.build().await
                } else {
                    let config = parse_config(&std::fs::File::open("RomHack.toml")?)?;
                    let writer = async_std::fs::OpenOptions::new()
                        .write(true)
                        .create(true)
                        .truncate(true)
                        .open(&config.build.iso)
                        .await?;
                    let disc_reader = async_std::fs::File::open(&config.src.iso).await?;
                    let mut builder = open_config_from_fs_iso(config, disc_reader, writer).await?;
                    builder.build().await
                }
            })
        }
        Commands::Apply {
            patch,
            original_game,
            output,
        } => task::block_on::<_, color_eyre::eyre::Result<()>>(async {
            let mut builder = open_config_from_patch(
                std::fs::OpenOptions::new().read(true).open(patch)?,
                async_std::fs::OpenOptions::new()
                    .read(true)
                    .open(original_game)
                    .await?,
                async_std::fs::OpenOptions::new()
                    .write(true)
                    .create(true)
                    .truncate(true)
                    .open(output)
                    .await?,
            )
            .await?;
            builder.build().await
        }),
        Commands::New { name } => {
            new(&name)?;
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use core::fmt;
    use std::{pin::Pin, task::{Context, Poll}};

    use async_std::io;
    use futures::{AsyncReadExt, AsyncSeek, AsyncWrite};
    use geckolib::{iso::{builder::Builder, read::DiscReader}, open_config_from_patch};

    #[derive(Copy, Clone, Default)]
    pub struct Sink {
        _private: (),
    }
    
    impl fmt::Debug for Sink {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.pad("Sink { .. }")
        }
    }
    
    impl AsyncWrite for Sink {
        #[inline]
        fn poll_write(
            self: Pin<&mut Self>,
            _: &mut Context<'_>,
            buf: &[u8],
        ) -> Poll<io::Result<usize>> {
            Poll::Ready(Ok(buf.len()))
        }
    
        #[inline]
        fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    
        #[inline]
        fn poll_close(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }

    impl AsyncSeek for Sink {
        fn poll_seek(
            self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
            pos: std::io::SeekFrom,
        ) -> Poll<std::io::Result<u64>> {
            match pos {
                std::io::SeekFrom::Start(pos) => Poll::Ready(Ok(pos)),
                std::io::SeekFrom::End(_) => Poll::Ready(Ok(0)),
                std::io::SeekFrom::Current(_) => Poll::Ready(Ok(0)),
            }
        }
    }
    
    #[test]
    fn empty_iso_can_be_read() {
        async_std::task::block_on(async {
            let mut reader =
                DiscReader::new(async_std::fs::File::open("assets/empty.iso").await.unwrap())
                    .await
                    .unwrap();
            let mut buf = Vec::new();
            let ret = reader.read_to_end(&mut buf).await;
            println!("{:?}", ret);
            assert!(ret.is_ok());
            debug_assert_eq!(buf.len(), ret.unwrap());
            debug_assert_eq!(buf.len(), 8388608);
            // debug_assert_eq!(&buf[0..6], b"R00J\0\x01");
        });
    }

    #[test]
    fn empty_patch_can_be_loaded_and_built() {
        async_std::task::block_on(async {
            let mut builder = open_config_from_patch(
                std::fs::File::open("assets/empty.patch").unwrap(),
                async_std::fs::File::open("assets/empty.iso").await.unwrap(),
                Sink::default(),
            )
            .await.unwrap();
            builder.build().await.unwrap();
        });
    }
}
