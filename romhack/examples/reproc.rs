use async_std::{
    io::{prelude::SeekExt, BufReader, ReadExt, WriteExt},
    sync::{Arc, Mutex},
};
use geckolib::{
    iso::{read::DiscReader, write::DiscWriter, consts},
    vfs::GeckoFS,
};

// Reprocesses a given iso (load iso in to a FileSystem, then save it back into an other iso)
fn main() -> color_eyre::eyre::Result<()> {
    color_eyre::install()?;
    env_logger::init();

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
            println!(
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
            Arc::new(Mutex::new(DiscWriter::from_reader(guard, async_std::fs::OpenOptions::new().write(true).read(true).create(true).open(std::env::args().nth(2).expect("No output file were provided")).await?).await?))
        };

        let fs = GeckoFS::parse(f).await?;
        {
            let mut out_guard = out.lock_arc().await;
            let mut fs_guard = fs.lock_arc().await;
            let file = fs_guard.sys_mut().resolve_node("iso.hdr").unwrap().as_file_mut().unwrap();
            let mut buf = vec![0u8; 6];
            file.read_exact(&mut buf).await?;
            file.seek(std::io::SeekFrom::Start(0)).await?;
            println!("iso.hdr: {:?}", String::from_utf8_lossy(&buf[..6]));
            let mut buf: Vec<u8> = Vec::new();
            file.read_to_end(&mut buf).await?;
            out_guard.write_all(&buf).await?;
            buf.clear();
            let path = match out_guard.get_type() {
                geckolib::iso::disc::DiscType::Gamecube => "map/Final/Release/frameworkF.map",
                geckolib::iso::disc::DiscType::Wii => "map/Rfinal/Release/RframeworkF.map",
            };
            let file2 = fs_guard.root_mut().resolve_node(path).unwrap().as_file_mut().unwrap();
            file2.read_to_end(&mut buf).await?;
            out_guard.seek(std::io::SeekFrom::Start(consts::HEADER_LENGTH as u64)).await?;
            out_guard.write_all(&buf).await?;
            out_guard.flush().await?;
            out_guard.finalize().await?;
        }
        <color_eyre::eyre::Result<()>>::Ok(())
    })?;
    Ok(())
}
