use async_std::{
    io::{prelude::*, BufReader},
    task,
};
use geckolib::iso::disc::DiscReader;

fn main() -> color_eyre::eyre::Result<()> {
    color_eyre::install()?;
    env_logger::init();

    task::block_on(async {
        let f = async_std::fs::File::open(std::env::args().nth(1).expect("No ISO file were provided")).await?;
        let mut f = BufReader::new(DiscReader::new(f).await?);

        f.seek(std::io::SeekFrom::Start(0)).await?;
        let mut buf = vec![0u8; 0x60];
        f.read(&mut buf).await?;
        println!(
            "[{}] Game Title: {:02X?}",
            String::from_utf8_lossy(&buf[..6]),
            String::from_utf8_lossy(&buf[0x20..0x60]).split_terminator('\0').next().expect("This game has no title")
        );
        <color_eyre::eyre::Result<()>>::Ok(())
    })?;
    Ok(())
}
