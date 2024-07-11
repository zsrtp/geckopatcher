#[cfg(feature = "progress")]
use crate::UPDATER;
use async_std::{
    io::{self, prelude::*, Seek as AsyncSeek, Write as AsyncWrite},
    task::ready,
};
use byteorder::{ByteOrder, BE};
use eyre::Result;
use pin_project::pin_project;
#[cfg(feature = "parallel")]
use rayon::{prelude::ParallelIterator, slice::ParallelSliceMut};
use sha1_smol::Sha1;
use std::task::Poll;
use std::{future::Future, io::SeekFrom};
use std::{pin::Pin, task::Context};

use crate::{
    crypto::{aes_encrypt_inplace, consts, AesKey, Unpackable},
    iso::disc::{
        align_addr, disc_set_header, to_encrypted_addr, PartHeader, TMDContent,
        TitleMetaData, WiiDiscHeader,
    },
};

use super::disc::{decrypt_title_key, DiscType, WiiDisc, WiiPartition};

#[derive(Debug)]
pub struct GCDiscWriter<W> {
    writer: W,
}

impl<W> GCDiscWriter<W> {
    pub fn new(writer: W) -> Self {
        Self { writer }
    }
}

impl<W> Clone for GCDiscWriter<W>
where
    W: Clone,
{
    fn clone(&self) -> Self {
        Self {
            writer: self.writer.clone(),
        }
    }
}

impl<W> AsyncSeek for GCDiscWriter<W>
where
    W: AsyncSeek + Unpin,
{
    fn poll_seek(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        pos: SeekFrom,
    ) -> Poll<Result<u64, std::io::Error>> {
        std::pin::pin!(&mut self.get_mut().writer).poll_seek(cx, pos)
    }
}

impl<W> AsyncWrite for GCDiscWriter<W>
where
    W: AsyncWrite + Unpin,
{
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> Poll<Result<usize, std::io::Error>> {
        std::pin::pin!(&mut self.get_mut().writer).poll_write(cx, buf)
    }

    fn poll_flush(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<std::io::Result<()>> {
        std::pin::pin!(&mut self.get_mut().writer).poll_flush(cx)
    }

    fn poll_close(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<std::io::Result<()>> {
        std::pin::pin!(&mut self.get_mut().writer).poll_close(cx)
    }
}

// ---

#[derive(Debug, Clone, Default)]
enum WiiDiscWriterState {
    #[default]
    /// Setups the data to be written
    Setup,
    /// Parse the data to be written
    Parse(Vec<u8>),
    /// Seek for writing accumulated data
    SeekToGroup(usize, Vec<u8>),
    /// Writing the data
    Writing(usize, Vec<u8>, Vec<u8>),
}

#[derive(Debug, Clone, Default)]
enum WiiDiscWriterFinalizeState {
    #[default]
    Init,
    SeekToLastGroup(u64, Vec<u8>),
    WriteLastGroup(u64, Vec<u8>),

    SeekToPartHeader(Vec<u8>),
    WritePartHeader(Vec<u8>),

    Done,
}

#[derive(Debug)]
#[pin_project]
pub struct WiiDiscWriter<W> {
    pub disc: WiiDisc,
    hashes: Vec<[u8; consts::WII_HASH_SIZE]>,
    // Virtual cursor which tracks where in the decrypted partition we are writing from.
    cursor: u64,
    state: WiiDiscWriterState,
    group: [u8; consts::WII_SECTOR_SIZE * 64],
    finalize_state: WiiDiscWriterFinalizeState,
    #[pin]
    writer: W,
}

#[cfg(feature = "parallel")]
fn get_data_pool(data: &mut [u8], chunk_size: usize) -> impl ParallelIterator<Item = &mut [u8]> {
    data.par_chunks_exact_mut(chunk_size)
}

#[cfg(not(feature = "parallel"))]
fn get_data_pool(data: &mut [u8], chunk_size: usize) -> impl Iterator<Item = &mut [u8]> {
    data.chunks_exact_mut(chunk_size)
}

fn h0_process(data: &mut [u8]) {
    let (hash, data) = data.split_at_mut(consts::WII_SECTOR_HASH_SIZE);
    for j in 0..consts::WII_SECTOR_DATA_HASH_COUNT {
        hash[j * consts::WII_HASH_SIZE..(j + 1) * consts::WII_HASH_SIZE].copy_from_slice(
            &Sha1::from(
                &data[j * consts::WII_SECTOR_DATA_HASH_SIZE
                    ..(j + 1) * consts::WII_SECTOR_DATA_HASH_SIZE],
            )
            .digest()
            .bytes(),
        );
    }
}

fn h1_process(data: &mut [u8]) {
    let mut hash = [0u8; consts::WII_HASH_SIZE * 8];
    for j in 0..8 {
        hash[j * consts::WII_HASH_SIZE..(j + 1) * consts::WII_HASH_SIZE].copy_from_slice(
            &Sha1::from(
                &data[j * consts::WII_SECTOR_SIZE..]
                    [..consts::WII_HASH_SIZE * (consts::WII_SECTOR_DATA_HASH_COUNT)],
            )
            .digest()
            .bytes()[..],
        );
    }
    // let pool = get_data_pool(data, consts::WII_SECTOR_SIZE);
    // pool.for_each(|d| {
    //     d[0x280..][..consts::WII_HASH_SIZE * 8]
    //         .copy_from_slice(&hash);
    // });
    for j in 0..8 {
        data[j * consts::WII_SECTOR_SIZE + 0x280..][..consts::WII_HASH_SIZE * 8]
            .copy_from_slice(&hash);
    }
}

fn h2_process(h: &mut [u8]) {
    let mut hash = [0u8; consts::WII_HASH_SIZE * 8];
    for i in 0..8 {
        hash[i * consts::WII_HASH_SIZE..(i + 1) * consts::WII_HASH_SIZE].copy_from_slice(
            &Sha1::from(
                &h[i * 8 * consts::WII_SECTOR_SIZE + 0x280..][..consts::WII_HASH_SIZE * 8],
            )
                .digest()
                .bytes()[..],
        );
    }
    // let pool = get_data_pool(h, consts::WII_SECTOR_SIZE);
    // pool.for_each(|d| {
    //     d[0x340..][..consts::WII_HASH_SIZE * 8]
    //         .copy_from_slice(&hash);
    // });
    for i in 0..8 * 8 {
        h[i * consts::WII_SECTOR_SIZE + 0x340..][..consts::WII_HASH_SIZE * 8]
            .copy_from_slice(&hash);
    }
}

fn hash_group(buf: &mut [u8]) -> [u8; consts::WII_HASH_SIZE] {
    // h0
    let data_pool = get_data_pool(buf, consts::WII_SECTOR_SIZE);
    data_pool.for_each(h0_process);
    // h1
    let data_pool = get_data_pool(buf, consts::WII_SECTOR_SIZE * 8);
    data_pool.for_each(h1_process);
    // h2
    let data_pool = get_data_pool(buf, consts::WII_SECTOR_SIZE * 8 * 8);
    data_pool.for_each(h2_process);
    // single H3
    let mut ret_buf = [0u8; consts::WII_HASH_SIZE];
    ret_buf.copy_from_slice(
        &Sha1::from(&buf[0x340..][..consts::WII_HASH_SIZE * 8])
            .digest()
            .bytes(),
    );
    ret_buf
}

/// Implementation of the Segher's fake signing algorithm
fn fake_sign(part: &mut WiiPartition, hashes: &[[u8; consts::WII_HASH_SIZE]]) {
    let content = &mut part.tmd.contents[0];
    let mut hashes_ = Vec::with_capacity(consts::WII_H3_SIZE);
    hashes_.extend(hashes.iter().flatten());
    hashes_.resize(consts::WII_H3_SIZE, 0);
    crate::debug!(
        "[fake_sign] Hashes size: 0x{:08X}; Hashes padding size: 0x{:08X}; H3 size: 0x{:08X}",
        hashes.len() * consts::WII_HASH_SIZE,
        consts::WII_H3_SIZE - hashes.len() * consts::WII_HASH_SIZE,
        hashes_.len()
    );
    content
        .hash
        .copy_from_slice(&Sha1::from(&hashes_).digest().bytes());

    // Fake sign tmd
    let _ = part.tmd.fake_sign();
    let _ = part.header.ticket.fake_sign();
}

fn encrypt_group(group: &mut [u8], part_key: AesKey) {
    crate::trace!("Encrypting group");
    #[cfg(feature = "parallel")]
    let data_pool = group.par_chunks_exact_mut(consts::WII_SECTOR_SIZE);
    #[cfg(not(feature = "parallel"))]
    let data_pool = group.chunks_exact_mut(consts::WII_SECTOR_SIZE);
    let encrypt_process = |data: &mut [u8]| {
        let mut iv = [0u8; consts::WII_KEY_SIZE];
        aes_encrypt_inplace(&mut data[..consts::WII_SECTOR_HASH_SIZE], &iv, &part_key);
        iv[..consts::WII_KEY_SIZE]
            .copy_from_slice(&data[consts::WII_SECTOR_IV_OFF..][..consts::WII_KEY_SIZE]);
        aes_encrypt_inplace(
            &mut data[consts::WII_SECTOR_HASH_SIZE..][..consts::WII_SECTOR_DATA_SIZE],
            &iv,
            &part_key,
        );
    };
    data_pool.for_each(encrypt_process);
}

impl<W> WiiDiscWriter<W>
where
    W: AsyncWrite + AsyncSeek + Unpin,
{
    pub async fn init(disc: WiiDisc, writer: W) -> Result<Self> {
        crate::trace!("Writing Wii Disc and Partition headers");
        let mut this = Self {
            disc,
            cursor: 0,
            state: WiiDiscWriterState::default(),
            group: [0u8; consts::WII_SECTOR_SIZE * 64],
            finalize_state: WiiDiscWriterFinalizeState::default(),
            hashes: Vec::new(),
            writer,
        };

        // Write ISO header
        let mut buf = vec![0u8; WiiDiscHeader::BLOCK_SIZE];
        disc_set_header(&mut buf, &this.disc.disc_header);
        this.writer.seek(SeekFrom::Start(0)).await?;
        this.writer.write_all(&buf).await?;

        // Get to the Partition Info
        this.writer
            .write_all(&vec![0u8; 0x40000 - WiiDiscHeader::BLOCK_SIZE])
            .await?;

        // Write Partition Info
        let part = &mut this.disc.partitions.partitions[this.disc.partitions.data_idx];
        let mut buf = [0u8; 0x28];
        BE::write_u32(&mut buf[..], 1);
        BE::write_u32(&mut buf[4..], 0x40020 >> 2);
        let offset: u64 = 0x50000;
        let i = 0;
        let part_type: u32 = part.part_type.into();
        crate::debug!("part_type: {}", part_type);
        part.part_offset = offset;
        BE::write_u32(&mut buf[0x20 + (8 * i)..], (offset >> 2) as u32);
        BE::write_u32(&mut buf[0x20 + (8 * i) + 4..], part_type);
        this.writer.write_all(&buf).await?;

        // Get to Region area
        const REGION_OFFSET: usize = 0x4E000;
        let buf = vec![0u8; REGION_OFFSET - 0x40028];
        this.writer.write_all(&buf).await?;

        // Write Region area
        let mut buf = [0u8; 0x20];
        this.disc.disc_region.compose_into(&mut buf);
        this.writer.write_all(&buf).await?;

        // Get to Magic
        const MAGIC_OFFSET: usize = 0x4FFFC;
        let buf = vec![0u8; MAGIC_OFFSET - 0x4E020];
        this.writer.write_all(&buf).await?;

        // Write Magic
        const WII_END_MAGIC: u32 = 0xC3F81A8E;
        let mut buf = [0u8; 4];
        BE::write_u32(&mut buf, WII_END_MAGIC);
        this.writer.write_all(&buf).await?;

        // Make sure there is at least one content in the TitleMetaData
        if part.tmd.contents.is_empty() {
            crate::warn!("TMD has no content value. Generating new value");
            part.tmd.contents.push(TMDContent {
                content_id: 0,
                index: 0,
                content_type: 1,
                size: 0x1E0000,
                hash: [0u8; consts::WII_HASH_SIZE],
            });
        }

        // Write (partial) partition header
        part.header.data_size = 0;
        part.header.tmd_size = part.tmd.get_size();
        part.header.tmd_offset = PartHeader::BLOCK_SIZE as u64;
        part.header.cert_offset = part.header.tmd_offset + part.header.tmd_size as u64;
        part.header.h3_offset = std::cmp::max(
            consts::WII_H3_OFFSET as u64,
            part.header.cert_offset + part.header.cert_size as u64,
        );
        part.header.data_offset =
            align_addr(part.header.cert_offset + part.header.cert_size as u64, 17);
        let buf = <[u8; PartHeader::BLOCK_SIZE]>::from(&part.header);
        this.writer.write_all(&buf).await?;
        let mut buf = vec![0u8; part.header.tmd_size];
        TitleMetaData::set_partition(&mut buf, 0, &part.tmd);
        this.writer.write_all(&buf).await?;
        // Write certificate
        this.writer.write_all(&part.cert).await?;
        let padding_size = std::cmp::max(
            0,
            part.header.data_offset as i64 - (part.header.h3_offset as i64),
        );
        if padding_size > 0 {
            let buf = vec![0u8; padding_size as usize];
            this.writer.write_all(&buf).await?;
        }

        Ok(this)
    }
}

fn prepare_header(part: &mut WiiPartition, hashes: &[[u8; consts::WII_HASH_SIZE]]) -> Vec<u8> {
    // Hash the whole table and return the partition header
    fake_sign(part, hashes);
    let part_offset = part.part_offset;
    crate::debug!("Partition offset: 0x{part_offset:08X?}");
    crate::trace!("Partition Header: {:?}", part.header);
    let mut buf = Vec::with_capacity(part.header.h3_offset as usize + consts::WII_H3_SIZE);
    let h3_padding =
        part.header.h3_offset as usize - (PartHeader::BLOCK_SIZE + part.tmd.get_size());
    //let mut buf = vec![0u8; PartHeader::BLOCK_SIZE + part.tmd.get_size()];
    buf.extend_from_slice(&<[u8; PartHeader::BLOCK_SIZE]>::from(&part.header));
    buf.extend(std::iter::repeat(0).take(part.tmd.get_size() + h3_padding));
    buf.extend(hashes.iter().flatten());
    buf.extend(
        std::iter::repeat(0)
            .take(consts::WII_H3_SIZE - hashes.len() * consts::WII_HASH_SIZE),
    );
    TitleMetaData::set_partition(&mut buf, PartHeader::BLOCK_SIZE, &part.tmd);
    buf
}

impl<W> WiiDiscWriter<W>
where
    W: AsyncWrite + AsyncSeek + Unpin,
{
    fn poll_close_disc(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.project();
        let part = &mut this.disc.partitions.partitions[this.disc.partitions.data_idx];
        let finalize_state = std::mem::take(this.finalize_state);
        #[cfg(feature = "progress")]
        if let Ok(mut updater) = UPDATER.lock() {
            updater.tick();
        }
        match finalize_state {
            WiiDiscWriterFinalizeState::Init => {
                crate::trace!("WiiDiscWriterFinalizeState::Init");
                // Align the encrypted data size to 21 bits
                part.header.data_size = align_addr(part.header.data_size, 21);

                // Hash and encrypt the last group
                let n_group = part.header.data_size / consts::WII_SECTOR_SIZE as u64 / 64;
                let group_idx = n_group - 1;
                crate::trace!("Hashing and encrypting group #{}", group_idx);
                if this.hashes.len() <= group_idx as usize {
                    this.hashes
                        .resize(group_idx as usize + 1, [0u8; consts::WII_HASH_SIZE]);
                }
                this.hashes[group_idx as usize].copy_from_slice(&hash_group(this.group));
                let part_key = decrypt_title_key(&part.header.ticket);
                encrypt_group(this.group, part_key);

                *this.finalize_state =
                    if *this.cursor % (consts::WII_SECTOR_DATA_SIZE as u64 * 64) != 0 {
                        WiiDiscWriterFinalizeState::SeekToLastGroup(n_group - 1, this.group.to_vec())
                    } else {
                        WiiDiscWriterFinalizeState::SeekToPartHeader(prepare_header(part, &this.hashes))
                    };
                cx.waker().wake_by_ref();
                Poll::Pending
            }
            WiiDiscWriterFinalizeState::SeekToLastGroup(group_idx, group_buf) => {
                crate::trace!(
                    "WiiDiscWriterFinalizeState::SeekToLastGroup(group_idx=0x{:08X})",
                    group_idx
                );
                let pos = part.part_offset
                    + part.header.data_offset
                    + group_idx as u64 * consts::WII_SECTOR_SIZE as u64 * 64;
                if this.writer.poll_seek(cx, SeekFrom::Start(pos)).is_pending() {
                    crate::trace!("Pending...");
                    *this.finalize_state = WiiDiscWriterFinalizeState::SeekToLastGroup(group_idx, group_buf);
                    return Poll::Pending;
                }
                *this.finalize_state = WiiDiscWriterFinalizeState::WriteLastGroup(group_idx, group_buf);
                cx.waker().wake_by_ref();
                Poll::Pending
            }
            WiiDiscWriterFinalizeState::WriteLastGroup(group_idx, group_buf) => {
                crate::trace!(
                    "WiiDiscWriterFinalizeState::WriteLastGroup(group_idx=0x{:08X})",
                    group_idx
                );
                let n_written = match this.writer.poll_write(cx, &group_buf) {
                    Poll::Ready(result) => result?,
                    Poll::Pending => {
                        crate::trace!("Pending...");
                        *this.finalize_state =
                            WiiDiscWriterFinalizeState::WriteLastGroup(group_idx, group_buf);
                        return Poll::Pending;
                    }
                };
                *this.finalize_state = if n_written < group_buf.len() {
                    WiiDiscWriterFinalizeState::WriteLastGroup(group_idx, group_buf.split_at(n_written).1.to_vec())
                } else {
                    WiiDiscWriterFinalizeState::SeekToPartHeader(prepare_header(part, &this.hashes))
                };
                cx.waker().wake_by_ref();
                Poll::Pending
            }
            WiiDiscWriterFinalizeState::SeekToPartHeader(buf) => {
                crate::trace!("WiiDiscWriterFinalizeState::SeekToPartHeader");
                if this
                    .writer
                    .poll_seek(cx, SeekFrom::Start(part.part_offset))
                    .is_pending()
                {
                    crate::trace!("Pending...");
                    *this.finalize_state = WiiDiscWriterFinalizeState::SeekToPartHeader(buf);
                    return Poll::Pending;
                }
                *this.finalize_state = WiiDiscWriterFinalizeState::WritePartHeader(buf);
                cx.waker().wake_by_ref();
                Poll::Pending
            }
            WiiDiscWriterFinalizeState::WritePartHeader(buf) => {
                crate::trace!("WiiDiscWriterFinalizeState::WritePartHeader");
                match this.writer.poll_write(cx, &buf) {
                    Poll::Ready(result) => match result {
                        Ok(n_written) => {
                            if n_written < buf.len() {
                                *this.finalize_state = WiiDiscWriterFinalizeState::WritePartHeader(
                                    buf.split_at(n_written).1.to_vec(),
                                );
                                cx.waker().wake_by_ref();
                                Poll::Pending
                            } else {
                                *this.finalize_state = WiiDiscWriterFinalizeState::Done;
                                Poll::Ready(Ok(()))
                            }
                        }
                        Err(err) => {
                            *this.finalize_state = WiiDiscWriterFinalizeState::Init;
                            Poll::Ready(Err(err))
                        }
                    },
                    Poll::Pending => {
                        crate::trace!("Pending...");
                        *this.finalize_state =
                            WiiDiscWriterFinalizeState::WritePartHeader(buf);
                        Poll::Pending
                    }
                }
            }
            WiiDiscWriterFinalizeState::Done => Poll::Ready(Ok(())),
        }
    }

    pub fn finalize(&mut self) -> FinalizeFuture<'_, W> {
        crate::trace!("Finalizing the ISO");
        FinalizeFuture { writer: self, state: Default::default() }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Default)]
enum FinalizeFutureState {
    #[default]
    Init,
    Flush,
    Close,
    Done,
}

pub struct FinalizeFuture<'a, W> {
    pub(crate) writer: &'a mut WiiDiscWriter<W>,
    state: FinalizeFutureState,
}

impl<'a, W: AsyncWrite + AsyncSeek + Unpin> Future for FinalizeFuture<'a, W> {
    type Output = io::Result<()>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let Self { writer, state } = &mut *self;
        let mut pinned = Pin::new(&mut **writer);
        match state {
            FinalizeFutureState::Init => {
                ready!(pinned.as_mut().poll_close_disc(cx))?;
                *state = FinalizeFutureState::Flush;
                cx.waker().wake_by_ref();
                Poll::Pending
            }
            FinalizeFutureState::Flush => {
                let this = pinned.as_mut().project();
                ready!(this.writer.poll_flush(cx))?;
                *state = FinalizeFutureState::Close;
                cx.waker().wake_by_ref();
                Poll::Pending
            }
            FinalizeFutureState::Close => {
                let this = pinned.as_mut().project();
                ready!(this.writer.poll_close(cx))?;
                *state = FinalizeFutureState::Done;
                cx.waker().wake_by_ref();
                Poll::Pending
            }
            FinalizeFutureState::Done => Poll::Ready(Ok(())),
        }
    }
}

impl<W> Clone for WiiDiscWriter<W>
where
    W: Clone,
{
    fn clone(&self) -> Self {
        Self {
            disc: self.disc.clone(),
            cursor: self.cursor,
            state: self.state.clone(),
            group: self.group.clone(),
            finalize_state: self.finalize_state.clone(),
            hashes: self.hashes.clone(),
            writer: self.writer.clone(),
        }
    }
}

impl<W> AsyncWrite for WiiDiscWriter<W>
where
    W: AsyncWrite + AsyncSeek + Unpin,
{
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        let this = self.project();
        let part = &mut this.disc.partitions.partitions[this.disc.partitions.data_idx];
        if let WiiDiscWriterState::Setup = this.state {
            crate::trace!("Pooling WiiDiscWriter for write ({} byte(s))", buf.len());
        }
        // If the requested size is 0, or if we are done reading, return without changing buf.
        if buf.is_empty() {
            return Poll::Ready(Ok(0));
        }
        let write_size = buf.len();
        // The "virtual" start and end, in the sense that they are the positions within the decrypted partition.
        let vstart = *this.cursor;
        let vend = vstart + write_size as u64;
        let start_blk_idx = (vstart / consts::WII_SECTOR_DATA_SIZE as u64) as usize;
        let end_blk_idx = ((vend - 1) / consts::WII_SECTOR_DATA_SIZE as u64) as usize;
        let start_group_idx = start_blk_idx / 64;
        let start_block_idx_in_group = start_blk_idx % 64;
        let end_group_idx = end_blk_idx / 64;
        let end_block_idx_in_group = end_blk_idx % 64;
        if let WiiDiscWriterState::Setup = this.state {
            crate::trace!(
                "Writing data from 0x{:08X} to 0x{:08X} (spanning {} block(s), from {} to {})",
                vstart,
                vend,
                end_blk_idx - start_blk_idx + 1,
                start_blk_idx,
                end_blk_idx,
            );
        }

        let state = std::mem::take(this.state);
        match state {
            WiiDiscWriterState::Setup => {
                *this.state = WiiDiscWriterState::Parse(buf.to_vec());
                cx.waker().wake_by_ref();
                Poll::Pending
            }
            WiiDiscWriterState::Parse(in_buf) => {
                let end_blk = std::cmp::min(
                    end_group_idx * 64 + end_block_idx_in_group,
                    start_group_idx * 64 + 64,
                );
                let mut curr_buf = &in_buf[..];
                let mut rem = write_size;
                for i in start_blk_idx..end_blk {
                    // Offsets in the group buffer (decrypted address)
                    let buffer_start = std::cmp::max(
                        vstart % consts::WII_SECTOR_DATA_SIZE as u64
                            + consts::WII_SECTOR_HASH_SIZE as u64
                            + start_block_idx_in_group as u64 * consts::WII_SECTOR_SIZE as u64,
                        (i % 64) as u64 * consts::WII_SECTOR_SIZE as u64,
                    );
                    let buffer_end = std::cmp::min(
                        vend % consts::WII_SECTOR_DATA_SIZE as u64
                            + consts::WII_SECTOR_HASH_SIZE as u64
                            + end_block_idx_in_group as u64 * consts::WII_SECTOR_SIZE as u64,
                        (i % 64) as u64 * consts::WII_SECTOR_SIZE as u64
                            + consts::WII_SECTOR_SIZE as u64,
                    );
                    let data;
                    (data, curr_buf) = curr_buf.split_at((buffer_end - buffer_start) as usize);
                    this.group[buffer_start as usize..buffer_end as usize].copy_from_slice(data);
                    rem -= data.len();
                }
                if *this.cursor
                    + (write_size - rem) as u64 % (consts::WII_SECTOR_DATA_SIZE as u64 * 64)
                    == 0
                {
                    // We are at the start of a group. We can hash and encrypt the group and write it.
                    crate::trace!("Hashing and encrypting group #{}", start_group_idx);
                    if this.hashes.len() <= start_group_idx {
                        this.hashes
                            .resize(start_group_idx + 1, [0u8; consts::WII_HASH_SIZE]);
                    }
                    this.hashes[start_group_idx].copy_from_slice(&hash_group(this.group));
                    let part_key = decrypt_title_key(&part.header.ticket);
                    encrypt_group(this.group, part_key);
                    *this.state =
                        WiiDiscWriterState::SeekToGroup(start_group_idx, curr_buf.to_vec());
                    cx.waker().wake_by_ref();
                    Poll::Pending
                } else {
                    // We are in the middle of a group. We need to read the rest of the group.
                    *this.state = WiiDiscWriterState::Setup;
                    *this.cursor += write_size as u64;
                    Poll::Ready(Ok(write_size))
                }
            }
            WiiDiscWriterState::SeekToGroup(group_idx, curr_buf) => {
                let group_addr = part.part_offset
                    + part.header.data_offset
                    + to_encrypted_addr((group_idx * 64 * consts::WII_SECTOR_DATA_SIZE) as u64);
                crate::trace!("Seeking to 0x{:08X}", group_addr);
                if this
                    .writer
                    .poll_seek(cx, SeekFrom::Start(group_addr))
                    .is_pending()
                {
                    *this.state = WiiDiscWriterState::SeekToGroup(group_idx, curr_buf);
                    return Poll::Pending;
                }
                crate::trace!("Seeking succeeded");
                *this.state = WiiDiscWriterState::Writing(group_idx, this.group.to_vec(), curr_buf);
                this.group.fill(0);
                cx.waker().wake_by_ref();
                Poll::Pending
            }
            WiiDiscWriterState::Writing(group_idx, group_buf, curr_buf) => {
                crate::trace!("Writing group #{}", group_idx);
                let n_written = match this.writer.poll_write(cx, &group_buf) {
                    Poll::Ready(result) => result?,
                    Poll::Pending => {
                        *this.state = WiiDiscWriterState::Writing(group_idx, group_buf, curr_buf);
                        return Poll::Pending;
                    }
                };
                crate::trace!("Writing succeeded");
                if n_written < group_buf.len() {
                    *this.state = WiiDiscWriterState::Writing(
                        group_idx,
                        group_buf.split_at(n_written).1.to_vec(),
                        curr_buf,
                    );
                    cx.waker().wake_by_ref();
                    Poll::Pending
                } else {
                    if buf.is_empty() {
                        // The write is done.
                        *this.state = WiiDiscWriterState::Setup;
                        *this.cursor += write_size as u64;
                        Poll::Ready(Ok(write_size))
                    } else {
                        // We need to write the rest of the buffer
                        *this.state = WiiDiscWriterState::Parse(curr_buf);
                        cx.waker().wake_by_ref();
                        Poll::Ready(Ok(write_size))
                    }
                }
            }
        }
    }

    fn poll_flush(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<std::io::Result<()>> {
        self.project().writer.poll_flush(cx)
    }

    fn poll_close(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<std::io::Result<()>> {
        crate::debug!("poll_close of WiiDiscWriter");
        ready!(self.as_mut().poll_close_disc(cx))?;
        self.project().writer.poll_close(cx)
    }
}

// ---

#[derive(Debug)]
pub enum DiscWriter<W> {
    Gamecube(GCDiscWriter<W>),
    Wii(WiiDiscWriter<W>),
}

impl<W> AsyncWrite for DiscWriter<W>
where
    W: AsyncWrite + AsyncSeek + Unpin,
{
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        match self.get_mut() {
            DiscWriter::Gamecube(writer) => std::pin::pin!(writer).poll_write(cx, buf),
            DiscWriter::Wii(writer) => std::pin::pin!(writer).poll_write(cx, buf),
        }
    }

    fn poll_flush(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<std::io::Result<()>> {
        match self.get_mut() {
            DiscWriter::Gamecube(writer) => std::pin::pin!(writer).poll_flush(cx),
            DiscWriter::Wii(writer) => std::pin::pin!(writer).poll_flush(cx),
        }
    }

    fn poll_close(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<std::io::Result<()>> {
        match self.get_mut() {
            DiscWriter::Gamecube(writer) => std::pin::pin!(writer).poll_close(cx),
            DiscWriter::Wii(writer) => std::pin::pin!(writer).poll_close(cx),
        }
    }
}

impl<W> DiscWriter<W> {
    pub async fn new_gc(writer: W) -> Result<Self> {
        Ok(Self::Gamecube(GCDiscWriter {
            writer,
        }))
    }

    pub fn get_type(&self) -> DiscType {
        match self {
            DiscWriter::Gamecube(_) => DiscType::Gamecube,
            DiscWriter::Wii(_) => DiscType::Wii,
        }
    }
}

impl<W> DiscWriter<W>
where
    W: AsyncWrite + AsyncSeek + Unpin,
{
    pub async fn new_wii(writer: W, disc: WiiDisc) -> Result<Self> {
        Ok(Self::Wii(
            WiiDiscWriter::init(disc, writer).await?,
        ))
    }

    pub async fn new(writer: W, disc_info: Option<WiiDisc>) -> Result<Self> {
        match disc_info {
            None => DiscWriter::new_gc(writer).await,
            Some(disc_info) => DiscWriter::new_wii(writer, disc_info).await,
        }
    }
}

impl<W> DiscWriter<W>
where
    W: AsyncWrite + AsyncSeek + Unpin,
{
    pub async fn finalize(&mut self) -> io::Result<()> {
        match self {
            DiscWriter::Gamecube(writer) => writer.flush().await,
            DiscWriter::Wii(writer) => writer.finalize().await,
        }
    }
}
