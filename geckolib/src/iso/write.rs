/* use async_std::io::{Seek as AsyncSeek, Write as AsyncWrite};
use byteorder::BE;
use eyre::Result;
use pin_project::pin_project;
use std::io::SeekFrom;
use std::pin::Pin;
use std::task::Poll;

use super::disc::{PartInfo, WiiDisc, WiiPartitions};

#[derive(Debug)]
#[pin_project]
pub struct GCDiscWriter<W: AsyncWrite + AsyncSeek> {
    #[pin]
    writer: W,
}

impl<W> GCDiscWriter<W>
where
    W: AsyncWrite + AsyncSeek,
{
    pub fn new(writer: W) -> Self {
        Self { writer }
    }
}

impl<W> Clone for GCDiscWriter<W>
where
    W: AsyncWrite + AsyncSeek + Clone,
{
    fn clone(&self) -> Self {
        Self {
            writer: self.writer.clone(),
        }
    }
}

impl<W> AsyncSeek for GCDiscWriter<W>
where
    W: AsyncWrite + AsyncSeek,
{
    fn poll_seek(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        pos: SeekFrom,
    ) -> Poll<Result<u64>> {
        self.project().reader.poll_seek(cx, pos)
    }
}

impl<W: AsyncWrite + AsyncSeek> AsyncWrite for GCDiscWriter<W> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> Poll<Result<usize>> {
        self.project().reader.poll_write(cx, buf)
    }
}

// ---

#[derive(Debug, Clone)]
enum WiiDiscWriterState {
    Seeking,
    Writing(Vec<u8>),
}

#[derive(Debug)]
#[pin_project]
pub struct WiiDiscWriter<W: AsyncWrite + AsyncSeek> {
    pub disc_info: WiiDisc,
    // Virtual cursor which tracks where in the decrypted partition we are writing from.
    cursor: u64,
    state: WiiDiscWriterState,
    #[pin]
    writer: Pin<Box<W>>,
}

impl<W> WiiDiscWriter<W>
where
    W: AsyncWrite + AsyncSeek,
{
    pub async fn init(writer: W) -> Result<Self> {
        crate::debug!("Trying to parse a Wii Disc from the reader");
        let mut this = Self {
            disc_info: WiiDisc {
                disc_header: Default::default(),
                partitions: Default::default(),
            },
            cursor: 0,
            state: WiiDiscWriterState::Seeking,
            writer: Box::pin(writer),
        };
        let reader = &mut this.writer;
        let mut buf = vec![0u8; WiiDiscHeader::BLOCK_SIZE];
        reader.seek(SeekFrom::Start(0)).await?;
        reader.read(&mut buf).await?;
        this.disc_info.disc_header = disc_get_header(&mut buf);
        crate::trace!("{:?}", this.disc_info.disc_header);
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

impl<W> Clone for WiiDiscWriter<W>
where
    W: AsyncWrite + AsyncSeek + Clone,
{
    fn clone(&self) -> Self {
        Self {
            disc_info: self.disc_info.clone(),
            cursor: self.cursor.clone(),
            state: self.state.clone(),
            writer: self.writer.clone(),
        }
    }
}

impl<W> AsyncSeek for WiiDiscWriter<W>
where
    W: AsyncWrite + AsyncSeek,
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
                        > this.disc_info.partitions.partitions[0].header.data_size as i64
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
                        > this.disc_info.partitions.partitions[0].header.data_size as i64
                {
                    return Poll::Ready(Err(std::io::Error::new(
                        std::io::ErrorKind::Other,
                        "Invalid argument",
                    )));
                }
                *this.cursor =
                    (this.disc_info.partitions.partitions[0].header.data_size as i64 + pos) as u64;
            }
            SeekFrom::Start(pos) => {
                if pos > this.disc_info.partitions.partitions[0].header.data_size {
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

impl<R: AsyncRead + AsyncSeek> AsyncRead for WiiDiscWriter<R> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut [u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        let this = self.project();
        crate::debug!("Pooling WiiDiscReader for read ({} byte(s))", buf.len());
        // If the requested size is 0, or if we are done reading, return without changing buf.
        let decrypted_size = (this.disc_info.partitions.partitions[0].header.data_size
            / consts::WII_SECTOR_SIZE as u64)
            * consts::WII_SECTOR_DATA_SIZE as u64;
        if buf.len() == 0 || *this.cursor >= decrypted_size {
            return Poll::Ready(Ok(0));
        }
        let part = this.disc_info.partitions.partitions[0];
        // Calculate the size and bounds of what has to be read.
        let read_size = std::cmp::min(buf.len(), (decrypted_size - *this.cursor) as usize);
        // The "virtual" start and end, in the sense that they are the positions within the decrypted partition.
        let vstart = *this.cursor;
        let vend = vstart + read_size as u64;
        let start_blk_idx = (vstart / consts::WII_SECTOR_DATA_SIZE as u64) as usize;
        let end_blk_idx = ((vend - 1) / consts::WII_SECTOR_DATA_SIZE as u64) as usize;
        crate::debug!(
            "Loading data from 0x{:08X} to 0x{:08X} (spanning {} block(s))",
            vstart,
            vend,
            end_blk_idx - start_blk_idx + 1
        );

        match this.state {
            WiiDiscWriterState::Seeking => {
                let start_blk_addr = part.part_offset
                    + part.header.data_offset
                    + (start_blk_idx * consts::WII_SECTOR_SIZE as usize) as u64;
                crate::debug!("Seeking to 0x{:08X}", start_blk_addr);
                ready!(this.reader.poll_seek(cx, SeekFrom::Start(start_blk_addr)))?;
                crate::debug!("Seeking succeeded");
                let n_blk = end_blk_idx - start_blk_idx + 1;
                let buf = vec![0u8; n_blk * consts::WII_SECTOR_SIZE];
                *this.state = WiiDiscWriterState::Writing(buf);
                cx.waker().wake_by_ref();
                Poll::Pending
            }
            WiiDiscWriterState::Writing(buf2) => {
                crate::debug!("Reading...");
                ready!(this.reader.poll_read(cx, buf2))?;
                crate::debug!("Reading successful");
                let part_key = decrypt_title_key(&part.header.ticket);
                crate::trace!("Partition key: {:?}", part_key);
                let mut data_pool: Vec<&mut [u8]> =
                    buf2.chunks_mut(consts::WII_SECTOR_SIZE).collect();
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
                    )
                    .unwrap();
                    aes_decrypt_inplace(
                        &mut data[consts::WII_SECTOR_HASH_SIZE..][..consts::WII_SECTOR_DATA_SIZE],
                        &iv,
                        &part_key,
                    )
                    .unwrap();
                    crate::trace!("after: {:?}", &data[consts::WII_SECTOR_HASH_SIZE..][..6]);
                };
                crate::debug!("Decrypting blocks");
                #[cfg(not(target_arch = "wasm32"))]
                data_pool.par_iter_mut().for_each(decrypt_process);
                #[cfg(target_arch = "wasm32")]
                data_pool.iter_mut().for_each(decrypt_process);
                crate::debug!("Decryption done");
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
                        (vstart + read_size as u64) - block_pos as u64,
                    ) as usize;
                    buf[buf_write_start..buf_write_end].copy_from_slice(
                        &block[consts::WII_SECTOR_HASH_SIZE..][block_read_start..block_read_end],
                    );
                }
                *this.cursor = *this.cursor + buf.len() as u64;
                *this.state = WiiDiscWriterState::Seeking;
                Poll::Ready(Ok(buf.len()))
            }
        }
    }
}

// ---

#[derive(Debug)]
#[pin_project(project = DiskWriterProj)]
pub enum DiscWriter<R: AsyncWrite + AsyncSeek> {
    Gamecube(#[pin] GCDiscWriter<Pin<Box<R>>>),
    Wii(#[pin] WiiDiscWriter<Pin<Box<R>>>),
}

impl<W> AsyncSeek for DiscWriter<W>
where
    W: AsyncWrite + AsyncSeek,
{
    fn poll_seek(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        pos: SeekFrom,
    ) -> Poll<std::io::Result<u64>> {
        match self.project() {
            DiscWriterProj::Gamecube(reader) => reader.poll_seek(cx, pos),
            DiscWriterProj::Wii(reader) => reader.poll_seek(cx, pos),
        }
    }
}

impl<W> AsyncWrite for DiscWriter<W>
where
    W: AsyncWrite + AsyncSeek,
{
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut [u8],
    ) -> Poll<std::io::Result<usize>> {
        match self.project() {
            DiscWriterProj::Gamecube(reader) => reader.poll_read(cx, buf),
            DiscWriterProj::Wii(reader) => reader.poll_read(cx, buf),
        }
    }
}

impl<W> DiscWriter<W>
where
    W: AsyncWrite + AsyncSeek,
{
    pub async fn new(reader: W) -> Result<Self> {
        let mut reader = Box::pin(reader);
        reader.seek(SeekFrom::Start(0x18)).await?;
        let mut buf = [0u8; 8];
        reader.read(&mut buf).await?;
        crate::debug!("Magics: {:?}", buf);
        if BE::read_u32(&buf[4..][..4]) == 0xC2339F3D {
            Ok(Self::Gamecube(GCDiscWriter::new(reader)))
        } else if BE::read_u32(&buf[..][..4]) == 0x5D1C9EA3 {
            Ok(Self::Wii(WiiDiscWriter::try_parse(reader).await?))
        } else {
            Err(eyre::eyre!("Not a game disc"))
        }
    }

    pub fn is_wii(&self) -> bool {
        match self {
            DiscWriter::Gamecube(_) => false,
            DiscWriter::Wii(_) => true,
        }
    }
}
 */