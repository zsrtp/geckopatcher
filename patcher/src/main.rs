use async_std::task;

fn main() -> color_eyre::eyre::Result<()> {
    color_eyre::install()?;
    env_logger::init();

    task::block_on::<_, color_eyre::eyre::Result<()>>(async {
        todo!()
        //Ok(())
    })?;
    Ok(())
}
