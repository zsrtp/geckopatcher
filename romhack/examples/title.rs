use async_std::{
    io::{prelude::*},
    sync::{Arc, Mutex},
    task,
};
use geckolib::{iso::read::DiscReader, vfs::GeckoFS};

fn main() -> color_eyre::eyre::Result<()> {
    color_eyre::install()?;
    env_logger::init();

    task::block_on(async {
        let f =
            async_std::fs::File::open(std::env::args().nth(1).expect("No ISO file were provided"))
                .await?;
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
        let fs = GeckoFS::<async_std::fs::File>::parse(f).await?;
        println!("children count: {}", fs.get_child_count());
        <color_eyre::eyre::Result<()>>::Ok(())
    })?;
    Ok(())
}
