use std::sync::Arc;
use std::sync::atomic::AtomicU64;

use async_std::fs::{File, OpenOptions};
use clap::Parser;
use geckolib::iso::builder::PatchBuilder;
use geckolib::iso::read::DiscReader;
use geckolib::parse_config;
use geckolib::{iso::builder::Builder, new, open_config_from_fs_iso, open_config_from_patch};

#[cfg(feature = "progress")]
use geckolib::{UPDATER, update::UpdaterType};

mod progress;

use romhack::cli::{Cli, Commands};
use smol::block_on;

fn main() -> color_eyre::eyre::Result<()> {
    block_on(async_main())
}

async fn async_main() -> color_eyre::eyre::Result<()> {
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
            if patch {
                let config = parse_config(std::fs::File::open("RomHack.toml")?)?;
                let mut builder = PatchBuilder::with_config(config);
                builder.build().await
            } else {
                let config = parse_config(&std::fs::File::open("RomHack.toml")?)?;
                let writer = OpenOptions::new()
                    .write(true)
                    .create(true)
                    .truncate(true)
                    .open(&config.build.iso)
                    .await?;
                let disc_reader = File::open(&config.src.iso).await?;
                let mut builder = open_config_from_fs_iso(config, disc_reader, writer).await?;
                builder.build().await
            }
        }
        Commands::Apply {
            patch,
            original_game,
            output,
        } => {
            let mut builder = open_config_from_patch(
                std::fs::OpenOptions::new().read(true).open(patch)?,
                OpenOptions::new().read(true).open(original_game).await?,
                OpenOptions::new()
                    .write(true)
                    .create(true)
                    .truncate(true)
                    .open(output)
                    .await?,
            )
            .await?;
            builder.build().await
        }
        Commands::New { name } => {
            new(&name)?;
            Ok(())
        }
        Commands::Extract {
            original_game,
            patched_game,
            output,
        } => {
            println!("Extracting... Failed. Not implemented.");
            extract(
                std::fs::OpenOptions::new().read(true).open(original_game)?,
                std::fs::OpenOptions::new().read(true).open(patched_game)?,
                std::path::PathBuf::from_iter(output.iter()),
            )?;
            Ok(())
        }
    }
}

fn extract(
    original: std::fs::File,
    patched: std::fs::File,
    output: std::path::PathBuf,
) -> color_eyre::eyre::Result<()> {
    let original_fs =
        geckolib::vfs::tree::GeckoFS::parse_sync(DiscReader::new_sync(Arc::new(original))?)?;
    let patched_fs =
        geckolib::vfs::tree::GeckoFS::parse_sync(DiscReader::new_sync(Arc::new(patched))?)?;
    println!("output: {:?}", output);
    let sum = AtomicU64::new(0);
    patched_fs
        .enumerate_nodes_dfs(patched_fs.root)
        .filter_map(|(p, file)| file.as_file_ref().map(|f| (p, f)))
        .for_each(|(path, file)| {
            use std::io::Read;
            let original_file =
                if let Some(o_file) = original_fs.get_file_ref(original_fs.root, &path).and_then(|o_f| o_f.as_file_ref()) {
                    o_file
                } else {
                    return;
                };
            let mut patched_data = Vec::new();
            let mut original_data = Vec::new();
            if file.clone().read_to_end(&mut patched_data).and(original_file.clone().read_to_end(&mut original_data)).is_ok() {
                let patch = geckolib::diff::diff(original_data.as_slice(), patched_data.as_slice());
                if let Ok(Some(patch_val)) = patch {
                    use std::io::BufReader;
                    sum.fetch_add(patch_val.len() as u64, std::sync::atomic::Ordering::Relaxed);

                    let repatched = geckolib::diff::patch(original_data.as_slice(), BufReader::new(patch_val.as_slice()));
                    println!("/{}; {:?} B; {:?}", path.to_string_lossy(), patch_val.len(), repatched.is_ok_and(|r| r.len() == patched_data.len() && r == patched_data));
                }
            }
        });
    patched_fs
        .enumerate_nodes_dfs(patched_fs.sys)
        .filter(|(p,_)| p.to_string_lossy() == "Start.dol")
        .filter_map(|(p, file)| file.as_file_ref().map(|f| (p, f)))
        .for_each(|(path, file)| {
            use std::io::Read;
            let original_file =
                if let Some(o_file) = original_fs.get_file_ref(original_fs.sys, &path).and_then(|o_f| o_f.as_file_ref()) {
                    o_file
                } else {
                    return;
                };
            let mut patched_data = Vec::new();
            let mut original_data = Vec::new();
            if file.clone().read_to_end(&mut patched_data).and(original_file.clone().read_to_end(&mut original_data)).is_ok() {
                let patch = geckolib::diff::diff(original_data.as_slice(), patched_data.as_slice());
                if let Ok(Some(patch_val)) = patch {
                    use std::io::BufReader;
                    sum.fetch_add(patch_val.len() as u64, std::sync::atomic::Ordering::Relaxed);

                    let repatched = geckolib::diff::patch(original_data.as_slice(), BufReader::new(patch_val.as_slice()));
                    println!("&&systemdata/{}; {:?} B; {:?}", path.to_string_lossy(), patch_val.len(), repatched.is_ok_and(|r| r.len() == patched_data.len() && r == patched_data));
                }
            }
        });
    println!("Total size of patches: {} B", sum.into_inner());
    Ok(())
}

#[cfg(test)]
mod tests {
    use core::fmt;
    use std::{
        pin::Pin,
        task::{Context, Poll},
    };

    use async_std::io;
    use futures::{AsyncReadExt, AsyncSeek, AsyncWrite, executor::block_on};
    use geckolib::{
        iso::{builder::Builder, read::DiscReader},
        open_config_from_patch,
    };

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
        block_on(async {
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
        block_on(async {
            let mut builder = open_config_from_patch(
                std::fs::File::open("assets/empty.patch").unwrap(),
                async_std::fs::File::open("assets/empty.iso").await.unwrap(),
                Sink::default(),
            )
            .await
            .unwrap();
            builder.build().await.unwrap();
        });
    }
}
