use super::disc::*;
use crate::crypto::{aes_decrypt_inplace, consts, Unpackable, WiiCryptoError};
use crate::iso::consts as iso_consts;
use async_std::io::prelude::SeekExt;
use async_std::io::{Read as AsyncRead, ReadExt, Seek as AsyncSeek};
use async_std::sync::Mutex;
use async_std::task::ready;
use byteorder::{ByteOrder, BE};
use eyre::Result;
#[cfg(feature = "parallel")]
use rayon::prelude::*;
use std::io::SeekFrom;
use std::pin::{pin, Pin};
use std::sync::Arc;
use std::task::Poll;

#[derive(Debug, Clone)]
enum WiiDiscReaderState {
    Seeking,
    Reading(Vec<u8>),
}

#[derive(Debug)]
struct WiiDiscReaderStatus {
    // Virtual cursor which tracks where in the decrypted partition we are reading from.
    cursor: u64,
    state: WiiDiscReaderState,
}

#[derive(Debug)]
pub struct WiiDiscReader<R> {
    reader: R,
    status: Arc<Mutex<WiiDiscReaderStatus>>,
    pub disc: WiiDisc,
}

async fn get_partitions<R: AsyncRead + AsyncSeek>(
    reader: &mut Pin<&mut R>,
    part_info: &PartInfo,
) -> Result<WiiPartitions> {
    crate::debug!("Fetching partitions from reader");
    let mut ret_vec: Vec<WiiPartition> = Vec::new();
    let mut data_idx: Option<usize> = None;
    for entry in part_info.entries.iter() {
        let mut tmd_count_buf = [0u8; 2];
        reader.seek(SeekFrom::Start(entry.offset)).await?;
        reader.read_exact(&mut tmd_count_buf).await?;
        let tmd_count = BE::read_u16(&tmd_count_buf);
        let mut buf = vec![0u8; 0x2C0 + TitleMetaData::get_size_n(tmd_count)];
        reader.seek(SeekFrom::Start(entry.offset)).await?;
        reader.read_exact(&mut buf).await?;
        let header = PartHeader::try_from(&buf[..0x2C0])?;
        let tmd = TitleMetaData::from_partition(&buf[0x2C0..], 0);
        let mut buf = vec![0u8; header.cert_size];
        reader.seek(SeekFrom::Start(header.cert_offset)).await?;
        reader.read_exact(&mut buf).await?;
        let cert = buf.into_boxed_slice();
        let part = WiiPartition {
            part_offset: entry.offset,
            part_type: entry.part_type.into(),
            header,
            tmd,
            cert,
        };
        if part.part_type == PartitionType::Data && data_idx.is_none() {
            data_idx = Some(ret_vec.len());
        }
        ret_vec.push(part);
    }
    crate::debug!("{:} partitions found", ret_vec.len());
    #[cfg(feature = "log")]
    ret_vec
        .iter()
        .enumerate()
        .for_each(|(i, p)| crate::debug!("[#{}] offset: {:#X?}", i, p.part_offset));
    if !ret_vec.is_empty() {
        if let Some(data_idx) = data_idx {
            crate::trace!(
                "(cert_offset: {:#08X})",
                ret_vec[data_idx].header.cert_offset
            );
            crate::trace!(
                "(data_offset: {:#08X})",
                ret_vec[data_idx].header.data_offset
            );
            crate::trace!(
                "(data_size: {:#08X}; decrypted size: {:#08X})",
                ret_vec[data_idx].header.data_size,
                to_virtual_addr(ret_vec[data_idx].header.data_size)
            );
            return Ok(WiiPartitions {
                data_idx,
                part_info: part_info.clone(),
                partitions: ret_vec,
            });
        }
    }
    crate::warn!("No Game Partition found!");
    Err(WiiCryptoError::NoGamePartition.into())
}

impl<R> WiiDiscReader<R>
where
    R: AsyncRead + AsyncSeek + Unpin,
{
    pub async fn try_parse(reader: R) -> Result<Self> {
        crate::debug!("Trying to parse a Wii Disc from the reader");
        let mut this = Self {
            reader,
            status: Arc::new(Mutex::new(WiiDiscReaderStatus {
                cursor: 0,
                state: WiiDiscReaderState::Seeking,
            })),
            disc: WiiDisc {
                disc_header: Default::default(),
                disc_region: Default::default(),
                partitions: Default::default(),
            },
        };
        let mut buf = vec![0u8; WiiDiscHeader::BLOCK_SIZE];
        pin!(&mut this.reader).seek(SeekFrom::Start(0)).await?;
        pin!(&mut this.reader).read_exact(&mut buf).await?;
        this.disc.disc_header = disc_get_header(&buf);
        let disc_header = this.disc.disc_header;
        let mut buf = vec![0u8; WiiDiscRegion::BLOCK_SIZE];
        pin!(&mut this.reader)
            .seek(SeekFrom::Start(0x4E000))
            .await?;
        pin!(&mut this.reader).read_exact(&mut buf).await?;
        this.disc.disc_region = WiiDiscRegion::parse(&buf);
        crate::trace!("{:?}", disc_header);
        if disc_header.wii_magic != iso_consts::WII_MAGIC {
            return Err(WiiCryptoError::NotWiiDisc {
                magic: disc_header.wii_magic,
            }
            .into());
        }
        let part_info = disc_get_part_info_async(&mut pin!(&mut this.reader).as_mut()).await?;
        this.disc.partitions =
            get_partitions(&mut pin!(&mut this.reader).as_mut(), &part_info).await?;
        Ok(this)
    }
}

impl<R> Clone for WiiDiscReader<R>
where
    R: Clone,
{
    fn clone(&self) -> Self {
        Self {
            status: self.status.clone(),
            reader: self.reader.clone(),
            disc: self.disc.clone(),
        }
    }
}

impl<R> AsyncSeek for WiiDiscReader<R>
where
    R: AsyncSeek + Unpin,
{
    fn poll_seek(
        self: Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
        pos: std::io::SeekFrom,
    ) -> std::task::Poll<std::io::Result<u64>> {
        let this = self.get_mut();
        let mut state = match this.status.try_lock() {
            Some(state) => state,
            None => return Poll::Pending,
        };
        let part = &this.disc.partitions.partitions[this.disc.partitions.data_idx];
        match pos {
            SeekFrom::Current(pos) => {
                if state.cursor as i64 + pos < 0i64
                    || state.cursor as i64 + pos > to_virtual_addr(part.header.data_size) as i64
                {
                    return Poll::Ready(Err(std::io::Error::new(
                        std::io::ErrorKind::Other,
                        "Invalid argument",
                    )));
                }
                state.cursor = (state.cursor as i64 + pos) as u64;
            }
            SeekFrom::End(pos) => {
                if state.cursor as i64 + pos < 0i64
                    || state.cursor as i64 + pos > to_virtual_addr(part.header.data_size) as i64
                {
                    return Poll::Ready(Err(std::io::Error::new(
                        std::io::ErrorKind::Other,
                        "Invalid argument",
                    )));
                }
                state.cursor = (to_virtual_addr(part.header.data_size) as i64 + pos) as u64;
            }
            SeekFrom::Start(pos) => {
                if pos > to_virtual_addr(part.header.data_size) {
                    return Poll::Ready(Err(std::io::Error::new(
                        std::io::ErrorKind::Other,
                        "Invalid argument",
                    )));
                }
                state.cursor = pos;
            }
        }
        std::task::Poll::Ready(Ok(state.cursor))
    }
}

impl<R> AsyncRead for WiiDiscReader<R>
where
    R: AsyncRead + AsyncSeek + Unpin,
{
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut [u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        let this = self.get_mut();
        let mut state = match this.status.try_lock() {
            Some(state) => state,
            None => return Poll::Pending,
        };
        crate::trace!("Pooling WiiDiscReader for read ({} byte(s))", buf.len());
        let part = &this.disc.partitions.partitions[this.disc.partitions.data_idx];
        // If the requested size is 0, or if we are done reading, return without changing buf.
        let decrypted_size = to_virtual_addr(part.header.data_size);
        if buf.is_empty() || state.cursor >= decrypted_size {
            return Poll::Ready(Ok(0));
        }
        // Calculate the size and bounds of what has to be read.
        let read_size = std::cmp::min(buf.len(), (decrypted_size - state.cursor) as usize);
        // The "virtual" start and end, in the sense that they are the positions within the decrypted partition.
        let vstart = state.cursor;
        let vend = vstart + read_size as u64;
        let start_blk_idx = (vstart / consts::WII_SECTOR_DATA_SIZE as u64) as usize;
        let end_blk_idx = ((vend - 1) / consts::WII_SECTOR_DATA_SIZE as u64) as usize;
        crate::trace!(
            "Loading data from 0x{:08X} to 0x{:08X} (spanning {} block(s))",
            vstart,
            vend,
            end_blk_idx - start_blk_idx + 1
        );

        match &mut state.state {
            WiiDiscReaderState::Seeking => {
                let start_blk_addr = part.part_offset
                    + part.header.data_offset
                    + (start_blk_idx * consts::WII_SECTOR_SIZE) as u64;
                crate::trace!("Seeking to 0x{:08X}", start_blk_addr);
                ready!(pin!(&mut this.reader).poll_seek(cx, SeekFrom::Start(start_blk_addr)))?;
                crate::trace!("Seeking succeeded");
                let n_blk = end_blk_idx - start_blk_idx + 1;
                let buf = vec![0u8; n_blk * consts::WII_SECTOR_SIZE];
                state.state = WiiDiscReaderState::Reading(buf);
                cx.waker().wake_by_ref();
                Poll::Pending
            }
            WiiDiscReaderState::Reading(buf2) => {
                crate::trace!("Reading...");
                ready!(pin!(&mut this.reader).poll_read(cx, buf2))?;
                crate::trace!("Reading successful");
                let part_key = decrypt_title_key(
                    &this.disc.partitions.partitions[this.disc.partitions.data_idx]
                        .header
                        .ticket,
                );
                crate::trace!("Partition key: {:?}", part_key);
                #[cfg(feature = "parallel")]
                let mut data_pool: Vec<&mut [u8]> =
                    buf2.par_chunks_exact_mut(consts::WII_SECTOR_SIZE).collect();
                #[cfg(not(feature = "parallel"))]
                let mut data_pool: Vec<&mut [u8]> =
                    buf2.chunks_exact_mut(consts::WII_SECTOR_SIZE).collect();
                crate::trace!("data_pool size: {}", data_pool.len());
                let decrypt_process = move |data: &mut &mut [u8]| {
                    let mut iv = [0_u8; consts::WII_KEY_SIZE];
                    iv[..consts::WII_KEY_SIZE].copy_from_slice(
                        &data[consts::WII_SECTOR_IV_OFF..][..consts::WII_KEY_SIZE],
                    );
                    crate::trace!("iv: {:?}", iv);
                    crate::trace!("before: {:?}", &data[consts::WII_SECTOR_HASH_SIZE..][..6]);
                    // Decrypt the hash to check if valid (not required here)
                    aes_decrypt_inplace(
                        &mut data[..consts::WII_SECTOR_HASH_SIZE],
                        &[0_u8; consts::WII_KEY_SIZE],
                        &part_key,
                    );
                    aes_decrypt_inplace(
                        &mut data[consts::WII_SECTOR_HASH_SIZE..][..consts::WII_SECTOR_DATA_SIZE],
                        &iv,
                        &part_key,
                    );
                    crate::trace!("after: {:?}", &data[consts::WII_SECTOR_HASH_SIZE..][..6]);
                };
                crate::trace!("Decrypting blocks");
                #[cfg(feature = "parallel")]
                data_pool.par_iter_mut().for_each(decrypt_process);
                #[cfg(not(feature = "parallel"))]
                data_pool.iter_mut().for_each(decrypt_process);
                crate::trace!("Decryption done");
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
                state.cursor += buf.len() as u64;
                state.state = WiiDiscReaderState::Seeking;
                Poll::Ready(Ok(buf.len()))
            }
        }
    }
}

#[derive(Debug)]
pub enum DiscReader<R> {
    Gamecube(R),
    Wii(WiiDiscReader<R>),
}

impl<R> Clone for DiscReader<R>
where
    R: Clone,
{
    fn clone(&self) -> Self {
        match self {
            Self::Gamecube(reader) => Self::Gamecube(reader.clone()),
            Self::Wii(reader) => Self::Wii(reader.clone()),
        }
    }
}

impl<R> AsyncSeek for DiscReader<R>
where
    R: AsyncSeek + Unpin,
{
    fn poll_seek(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        pos: SeekFrom,
    ) -> Poll<std::io::Result<u64>> {
        match self.get_mut() {
            DiscReader::Gamecube(reader) => pin!(reader).poll_seek(cx, pos),
            DiscReader::Wii(reader) => pin!(reader).poll_seek(cx, pos),
        }
    }
}

impl<R> AsyncRead for DiscReader<R>
where
    R: AsyncRead + AsyncSeek + Unpin,
{
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut [u8],
    ) -> Poll<std::io::Result<usize>> {
        match self.get_mut() {
            DiscReader::Gamecube(reader) => pin!(reader).poll_read(cx, buf),
            DiscReader::Wii(reader) => pin!(reader).poll_read(cx, buf),
        }
    }
}

impl<R> DiscReader<R>
where
    R: AsyncRead + AsyncSeek + Unpin,
{
    pub async fn new(mut reader: R) -> Result<Self> {
        pin!(&mut reader).seek(SeekFrom::Start(0x18)).await?;
        let mut buf = [0u8; 8];
        pin!(&mut reader).read(&mut buf).await?;
        crate::debug!("Magics: {:?}", buf);
        if BE::read_u32(&buf[4..][..4]) == iso_consts::GC_MAGIC {
            crate::debug!("Loading Gamecube disc");
            Ok(Self::Gamecube(reader))
        } else if BE::read_u32(&buf[..][..4]) == iso_consts::WII_MAGIC {
            crate::debug!("Loading Wii disc");
            Ok(Self::Wii(WiiDiscReader::try_parse(reader).await?))
        } else {
            Err(eyre::eyre!(
                "Not a game disc (Wii: {:08X}; GC: {:08X})",
                BE::read_u32(&buf[..][..4]),
                BE::read_u32(&buf[4..][..4])
            ))
        }
    }

    pub fn get_type(&self) -> DiscType {
        match self {
            DiscReader::Gamecube(_) => DiscType::Gamecube,
            DiscReader::Wii(_) => DiscType::Wii,
        }
    }

    pub fn get_disc_info(&self) -> Option<WiiDisc> {
        match self {
            DiscReader::Gamecube(_) => None,
            DiscReader::Wii(wii) => Some(wii.disc.clone()),
        }
    }
}
