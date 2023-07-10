use super::disc::*;
use crate::crypto::{aes_decrypt_inplace, consts, WiiCryptoError, Unpackable};
use async_std::io::prelude::SeekExt;
use async_std::io::{Read as AsyncRead, Seek as AsyncSeek, ReadExt};
use async_std::task::ready;
use byteorder::{BE, ByteOrder};
use eyre::Result;
#[cfg(feature = "log")]
use log::{debug, trace, warn};
use pin_project::pin_project;
use rayon::prelude::{IntoParallelRefMutIterator, ParallelIterator};
use std::io::SeekFrom;
use std::pin::Pin;
use std::task::Poll;

#[derive(Debug)]
#[pin_project]
pub struct GCDiscReader<R: AsyncRead + AsyncSeek> {
    #[pin]
    reader: R,
}

impl<R> GCDiscReader<R>
where
    R: AsyncRead + AsyncSeek,
{
    pub fn new(reader: R) -> Self {
        Self { reader }
    }
}

impl<R> Clone for GCDiscReader<R>
where
    R: AsyncRead + AsyncSeek + Clone,
{
    fn clone(&self) -> Self {
        Self {
            reader: self.reader.clone(),
        }
    }
}

impl<R> AsyncSeek for GCDiscReader<R>
where
    R: AsyncRead + AsyncSeek,
{
    fn poll_seek(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        pos: std::io::SeekFrom,
    ) -> std::task::Poll<std::io::Result<u64>> {
        self.project().reader.poll_seek(cx, pos)
    }
}

impl<R: AsyncRead + AsyncSeek> AsyncRead for GCDiscReader<R> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut [u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        self.project().reader.poll_read(cx, buf)
    }
}

#[derive(Debug, Clone)]
enum WiiDiscReaderState {
    Seeking,
    Reading(Vec<u8>),
}

#[derive(Debug)]
#[pin_project]
pub struct WiiDiscReader<R: AsyncRead + AsyncSeek> {
    pub disc_info: WiiDisc,
    // Virtual cursor which tracks where in the decrypted partition we are reading from.
    cursor: u64,
    state: WiiDiscReaderState,
    #[pin]
    reader: Pin<Box<R>>,
}

async fn get_partitions<R: AsyncRead + AsyncSeek>(
    reader: &mut Pin<&mut R>,
    part_info: &PartInfo,
) -> Result<WiiPartitions> {
    #[cfg(feature = "log")]
    debug!("Fetching partitions from reader");
    let mut ret_vec: Vec<WiiPartition> = Vec::new();
    let mut data_idx: Option<usize> = None;
    for (i, entry) in part_info.entries.iter().enumerate() {
        let mut buf = [0u8; 0x2C0];
        reader.seek(SeekFrom::Start(entry.offset)).await?;
        reader.read(&mut buf).await?;
        let part = WiiPartition {
            part_offset: entry.offset,
            part_type: entry.part_type,
            header: PartHeader::try_from(&buf)?,
        };
        if part.part_type == 0 && data_idx.is_none() {
            data_idx = Some(i);
        }
        ret_vec.push(part);
    }
    #[cfg(feature = "log")]
    debug!("{:} partitions found", ret_vec.len());
    if !ret_vec.is_empty() {
        if let Some(data_idx) = data_idx {
            return Ok(WiiPartitions {
                data_idx,
                part_info: part_info.clone(),
                partitions: ret_vec,
            });
        }
    }
    #[cfg(feature = "log")]
    warn!("No Game Partition found!");
    Err(WiiCryptoError::NoGamePartition.into())
}

impl<R> WiiDiscReader<R>
where
    R: AsyncRead + AsyncSeek,
{
    pub async fn try_parse(reader: R) -> Result<Self> {
        #[cfg(feature = "log")]
        debug!("Trying to parse a Wii Disc from the reader");
        let mut this = Self {
            disc_info: WiiDisc {
                disc_header: Default::default(),
                partitions: Default::default(),
            },
            cursor: 0,
            state: WiiDiscReaderState::Seeking,
            reader: Box::pin(reader),
        };
        let reader = &mut this.reader;
        let mut buf = vec![0u8; WiiDiscHeader::BLOCK_SIZE];
        reader.seek(SeekFrom::Start(0)).await?;
        reader.read_exact(&mut buf).await?;
        this.disc_info.disc_header = disc_get_header(&buf);
        #[cfg(feature = "log")]
        trace!("{:?}", this.disc_info.disc_header);
        if this.disc_info.disc_header.wii_magic != 0x5D1C9EA3 {
            return Err(WiiCryptoError::NotWiiDisc {
                magic: this.disc_info.disc_header.wii_magic,
            }
            .into());
        }
        let part_info = disc_get_part_info_async(&mut reader.as_mut()).await?;
        this.disc_info.partitions = get_partitions(&mut reader.as_mut(), &part_info).await?;
        Ok(this)
    }
}

impl<R> Clone for WiiDiscReader<R>
where
    R: AsyncRead + AsyncSeek + Clone,
{
    fn clone(&self) -> Self {
        Self {
            disc_info: self.disc_info.clone(),
            cursor: self.cursor,
            state: self.state.clone(),
            reader: self.reader.clone(),
        }
    }
}

impl<R> AsyncSeek for WiiDiscReader<R>
where
    R: AsyncRead + AsyncSeek,
{
    fn poll_seek(
        self: Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
        pos: std::io::SeekFrom,
    ) -> std::task::Poll<std::io::Result<u64>> {
        let this = self.project();
        match pos {
            SeekFrom::Current(pos) => {
                if *this.cursor as i64 + pos < 0i64
                    || *this.cursor as i64 + pos
                        > this.disc_info.partitions.partitions[this.disc_info.partitions.data_idx].header.data_size as i64
                {
                    return Poll::Ready(Err(std::io::Error::new(
                        std::io::ErrorKind::Other,
                        "Invalid argument",
                    )));
                }
                *this.cursor = (*this.cursor as i64 + pos) as u64;
            }
            SeekFrom::End(pos) => {
                if *this.cursor as i64 + pos < 0i64
                    || *this.cursor as i64 + pos
                        > this.disc_info.partitions.partitions[this.disc_info.partitions.data_idx].header.data_size as i64
                {
                    return Poll::Ready(Err(std::io::Error::new(
                        std::io::ErrorKind::Other,
                        "Invalid argument",
                    )));
                }
                *this.cursor =
                    (this.disc_info.partitions.partitions[this.disc_info.partitions.data_idx].header.data_size as i64 + pos) as u64;
            }
            SeekFrom::Start(pos) => {
                if pos > this.disc_info.partitions.partitions[this.disc_info.partitions.data_idx].header.data_size {
                    return Poll::Ready(Err(std::io::Error::new(
                        std::io::ErrorKind::Other,
                        "Invalid argument",
                    )));
                }
                *this.cursor = pos;
            }
        }
        std::task::Poll::Ready(Ok(*this.cursor))
    }
}

impl<R: AsyncRead + AsyncSeek> AsyncRead for WiiDiscReader<R> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut [u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        let this = self.project();
        #[cfg(feature = "log")]
        debug!("Pooling WiiDiscReader for read ({} byte(s))", buf.len());
        // If the requested size is 0, or if we are done reading, return without changing buf.
        let decrypted_size = (this.disc_info.partitions.partitions[this.disc_info.partitions.data_idx].header.data_size
            / consts::WII_SECTOR_SIZE as u64)
            * consts::WII_SECTOR_DATA_SIZE as u64;
        if buf.is_empty() || *this.cursor >= decrypted_size {
            return Poll::Ready(Ok(0));
        }
        let part = this.disc_info.partitions.partitions[this.disc_info.partitions.data_idx];
        // Calculate the size and bounds of what has to be read.
        let read_size = std::cmp::min(buf.len(), (decrypted_size - *this.cursor) as usize);
        // The "virtual" start and end, in the sense that they are the positions within the decrypted partition.
        let vstart = *this.cursor;
        let vend = vstart + read_size as u64;
        let start_blk_idx = (vstart / consts::WII_SECTOR_DATA_SIZE as u64) as usize;
        let end_blk_idx = ((vend - 1) / consts::WII_SECTOR_DATA_SIZE as u64) as usize;
        #[cfg(feature = "log")]
        debug!(
            "Loading data from 0x{:08X} to 0x{:08X} (spanning {} block(s))",
            vstart,
            vend,
            end_blk_idx - start_blk_idx + 1
        );

        match this.state {
            WiiDiscReaderState::Seeking => {
                let start_blk_addr = part.part_offset
                    + part.header.data_offset
                    + (start_blk_idx * consts::WII_SECTOR_SIZE) as u64;
                #[cfg(feature = "log")]
                debug!("Seeking to 0x{:08X}", start_blk_addr);
                ready!(this.reader.poll_seek(cx, SeekFrom::Start(start_blk_addr)))?;
                #[cfg(feature = "log")]
                debug!("Seeking succeeded");
                let n_blk = end_blk_idx - start_blk_idx + 1;
                let buf = vec![0u8; n_blk * consts::WII_SECTOR_SIZE];
                *this.state = WiiDiscReaderState::Reading(buf);
                cx.waker().wake_by_ref();
                Poll::Pending
            }
            WiiDiscReaderState::Reading(buf2) => {
                #[cfg(feature = "log")]
                debug!("Reading...");
                ready!(this.reader.poll_read(cx, buf2))?;
                #[cfg(feature = "log")]
                debug!("Reading successful");
                let part_key = decrypt_title_key(&part.header.ticket);
                #[cfg(feature = "log")]
                trace!("Partition key: {:?}", part_key);
                let mut data_pool: Vec<&mut [u8]> =
                    buf2.chunks_mut(consts::WII_SECTOR_SIZE).collect();
                #[cfg(feature = "log")]
                trace!("data_pool size: {}", data_pool.len());
                let decrypt_process = move |data: &mut &mut [u8]| {
                    let mut iv = [0_u8; consts::WII_KEY_SIZE];
                    iv[..consts::WII_KEY_SIZE].copy_from_slice(
                        &data[consts::WII_SECTOR_IV_OFF..][..consts::WII_KEY_SIZE],
                    );
                    #[cfg(feature = "log")]
                    trace!("iv: {:?}", iv);
                    #[cfg(feature = "log")]
                    trace!("before: {:?}", &data[consts::WII_SECTOR_HASH_SIZE..][..6]);
                    // Decrypt the hash to check if valid (not required here)
                    aes_decrypt_inplace(
                        &mut data[..consts::WII_SECTOR_HASH_SIZE],
                        &[0_u8; consts::WII_KEY_SIZE],
                        &part_key,
                    )
                    .unwrap();
                    aes_decrypt_inplace(
                        &mut data[consts::WII_SECTOR_HASH_SIZE..][..consts::WII_SECTOR_DATA_SIZE],
                        &iv,
                        &part_key,
                    )
                    .unwrap();
                    #[cfg(feature = "log")]
                    trace!("after: {:?}", &data[consts::WII_SECTOR_HASH_SIZE..][..6]);
                };
                #[cfg(feature = "log")]
                debug!("Decrypting blocks");
                #[cfg(not(target_arch = "wasm32"))]
                data_pool.par_iter_mut().for_each(decrypt_process);
                #[cfg(target_arch = "wasm32")]
                data_pool.iter_mut().for_each(decrypt_process);
                #[cfg(feature = "log")]
                debug!("Decryption done");
                for (i, block) in data_pool.iter().enumerate() {
                    let block_pos =
                        (start_blk_idx + i) as u64 * consts::WII_SECTOR_DATA_SIZE as u64;
                    let buf_write_start =
                        std::cmp::max(0, block_pos as i64 - vstart as i64) as usize;
                    let buf_write_end: usize = std::cmp::min(
                        read_size,
                        ((block_pos + consts::WII_SECTOR_DATA_SIZE as u64) as i64 - vstart as i64)
                            as usize,
                    );
                    let block_read_start =
                        std::cmp::max(0, vstart as i64 - block_pos as i64) as usize;
                    let block_read_end = std::cmp::min(
                        consts::WII_SECTOR_DATA_SIZE as u64,
                        (vstart + read_size as u64) - block_pos,
                    ) as usize;
                    buf[buf_write_start..buf_write_end].copy_from_slice(
                        &block[consts::WII_SECTOR_HASH_SIZE..][block_read_start..block_read_end],
                    );
                }
                *this.cursor += buf.len() as u64;
                *this.state = WiiDiscReaderState::Seeking;
                Poll::Ready(Ok(buf.len()))
            }
        }
    }
}

#[derive(Debug)]
#[pin_project(project = DiskReaderProj)]
pub enum DiscReader<R: AsyncRead + AsyncSeek> {
    Gamecube(#[pin] GCDiscReader<Pin<Box<R>>>),
    Wii(#[pin] Box<WiiDiscReader<Pin<Box<R>>>>),
}

impl<R> AsyncSeek for DiscReader<R>
where
    R: AsyncRead + AsyncSeek,
{
    fn poll_seek(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        pos: SeekFrom,
    ) -> Poll<std::io::Result<u64>> {
        match self.project() {
            DiskReaderProj::Gamecube(reader) => reader.poll_seek(cx, pos),
            DiskReaderProj::Wii(reader) => reader.poll_seek(cx, pos),
        }
    }
}

impl<R> AsyncRead for DiscReader<R>
where
    R: AsyncRead + AsyncSeek,
{
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut [u8],
    ) -> Poll<std::io::Result<usize>> {
        match self.project() {
            DiskReaderProj::Gamecube(reader) => reader.poll_read(cx, buf),
            DiskReaderProj::Wii(reader) => reader.poll_read(cx, buf),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum DiscType {
    Gamecube = 0,
    Wii,
}

impl<R> DiscReader<R>
where
    R: AsyncRead + AsyncSeek,
{
    pub async fn new(reader: R) -> Result<Self> {
        let mut reader = Box::pin(reader);
        reader.seek(SeekFrom::Start(0x18)).await?;
        let mut buf = [0u8; 8];
        reader.read(&mut buf).await?;
        #[cfg(feature = "log")]
        debug!("Magics: {:?}", buf);
        if BE::read_u32(&buf[4..][..4]) == 0xC2339F3D {
            Ok(Self::Gamecube(GCDiscReader::new(reader)))
        } else if BE::read_u32(&buf[..][..4]) == 0x5D1C9EA3 {
            Ok(Self::Wii(Box::new(WiiDiscReader::try_parse(reader).await?)))
        } else {
            Err(eyre::eyre!("Not a game disc"))
        }
    }

    pub fn get_type(&self) -> DiscType {
        match self {
            DiscReader::Gamecube(_) => DiscType::Gamecube,
            DiscReader::Wii(_) => DiscType::Wii,
        }
    }
}
