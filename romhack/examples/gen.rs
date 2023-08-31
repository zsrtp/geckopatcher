use std::sync::Arc;

use async_std::sync::Mutex;
use futures::{AsyncRead, AsyncSeek, AsyncWrite};
use geckolib::{
    iso::{
        disc::{DiscType, PartHeader, TitleMetaData, WiiDiscRegionAgeRating, WiiPartition},
        write::DiscWriter,
    },
    vfs::GeckoFS,
};
#[cfg(feature = "progress")]
use romhack::progress;
use lazy_static::lazy_static;

lazy_static! {
    static ref DEFAULT_ISO_HDR: Box<[u8]> = {let mut vec = Vec::from(b"RZDE01\x00\x01\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x5D\x1C\x9E\xA3\x00\x00\x00\x00Test Wii ISO\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x01\x01".as_slice()); vec.extend(std::iter::repeat(0).take(0x39E)); vec.into_boxed_slice()};
}

struct DummyReaderWriter {}

impl AsyncRead for DummyReaderWriter {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
        _buf: &mut [u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        unreachable!()
    }
}

impl AsyncSeek for DummyReaderWriter {
    fn poll_seek(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
        _pos: std::io::SeekFrom,
    ) -> std::task::Poll<std::io::Result<u64>> {
        unreachable!()
    }
}

impl AsyncWrite for DummyReaderWriter {
    fn poll_write(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
        _buf: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        unreachable!()
    }

    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        unreachable!()
    }

    fn poll_close(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        unreachable!()
    }
}

// Generates an valid empty ISO
fn main() -> color_eyre::eyre::Result<()> {
    color_eyre::install()?;
    #[cfg(feature = "log")]
    env_logger::init();
    #[cfg(feature = "progress")]
    progress::init_cli_progress();

    async_std::task::block_on(async {
        let out = {
            let mut game_title = [0u8; 64];
            let title_string = "Test Wii ISO";
            game_title[..title_string.len()].copy_from_slice(title_string.as_bytes());
            DiscWriter::new(
                async_std::fs::OpenOptions::new()
                    .write(true)
                    .read(true)
                    .create(true)
                    .open(
                        std::env::args()
                            .nth(1)
                            .expect("No output file was provided"),
                    )
                    .await?,
                Some(geckolib::iso::disc::WiiDisc {
                    disc_header: geckolib::iso::disc::WiiDiscHeader {
                        disc_id: b'R',
                        game_code: [b'Z', b'D'],
                        region_code: b'E',
                        maker_code: [b'0', b'1'],
                        disc_number: 0,
                        disc_version: 1,
                        audio_streaming: false,
                        streaming_buffer_size: 0,
                        unk1: Default::default(),
                        wii_magic: 0x5D1C9EA3,
                        gc_magic: 0,
                        game_title,
                        disable_hash_verif: 0,
                        disable_disc_encrypt: 0,
                        padding: [0; 0x39E],
                    },
                    disc_region: geckolib::iso::disc::WiiDiscRegion {
                        region: 1,
                        age_rating: WiiDiscRegionAgeRating::default(),
                    },
                    partitions: geckolib::iso::disc::WiiPartitions {
                        data_idx: 0,
                        part_info: geckolib::iso::disc::PartInfo {
                            offset: 0,
                            entries: Vec::new(),
                        },
                        partitions: vec![WiiPartition {
                            part_type: geckolib::iso::disc::PartitionType::Data,
                            part_offset: 0x50000,
                            header: PartHeader::default(),
                            tmd: TitleMetaData::default(),
                            cert: vec![0x00].into_boxed_slice(),
                        }],
                    },
                }),
            )
            .await?
        };

        let mut out = std::pin::pin!(out);
        let mut fs = GeckoFS::<DummyReaderWriter>::new();

        fs.sys_mut().add_file(geckolib::vfs::File::new(
            geckolib::vfs::FileDataSource::Box(Arc::new(Mutex::new(DEFAULT_ISO_HDR.clone()))),
            "iso.hdr",
            0,
            DEFAULT_ISO_HDR.len(),
            0,
        ));
        fs.root_mut().add_file(geckolib::vfs::File::new(
            geckolib::vfs::FileDataSource::Box(Arc::new(Mutex::new(vec![b't', b'e', b's', b't'].into_boxed_slice()))),
            "test.txt",
            0,
            4,
            0,
        ));
        {
            let is_wii = out.get_type() == DiscType::Wii;
            fs.serialize(&mut out, is_wii).await?;
            #[cfg(feature = "log")]
            log::info!("Encrypting the ISO");
            out.finalize().await?;
        }
        <color_eyre::eyre::Result<()>>::Ok(())
    })?;
    Ok(())
}
