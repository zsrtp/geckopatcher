use std::io::SeekFrom;

use async_std::{
    io::{prelude::*, BufReader},
    sync::{Arc, Mutex},
    task,
};
use geckolib::{iso::read::DiscReader, vfs::GeckoFS};

fn main() -> color_eyre::eyre::Result<()> {
    color_eyre::install()?;
    env_logger::init();

    task::block_on(async {
        let f =
            BufReader::with_capacity(0x7C00 * 64 * 8, async_std::fs::File::open(std::env::args().nth(1).expect("No ISO file were provided"))
            .await?);
        let mut f = DiscReader::new(f).await?;

        f.seek(std::io::SeekFrom::Start(0)).await?;
        let mut buf = vec![0u8; 0x60];
        f.read(&mut buf).await?;
        println!(
            "[{}] Game Title: {:02X?}",
            String::from_utf8_lossy(&buf[..6]),
            String::from_utf8_lossy(&buf[0x20..0x60])
                .split_terminator('\0')
                .next()
                .expect("This game has no title")
        );
        let f = Arc::new(Mutex::new(f));
        let fs = GeckoFS::parse(f).await?;
        let mut guard = fs.lock_arc().await;
        let file = guard.main_dol_mut();
        // let mut buf = Vec::new();
        // file.read_to_end(&mut buf).await?;
        // println!("Main dol: {}", buf.iter().take(0x100).map(|x| format!("{:02X}", x)).collect::<Vec<String>>().join(""));
        let size = file.seek(SeekFrom::End(0)).await? as usize;
        file.seek(SeekFrom::Start(0)).await?;
        let mut buf = vec![0u8; size];
        file.read(&mut buf).await?;
        println!("Main dol: {}", buf.iter().take(0x100).map(|x| format!("{:02X}", x)).collect::<Vec<String>>().join(""));
        <color_eyre::eyre::Result<()>>::Ok(())
    })?;
    Ok(())
}
