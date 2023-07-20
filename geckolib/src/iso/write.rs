use async_std::{
    io::{self, prelude::*, Read as AsyncRead, Seek as AsyncSeek, Write as AsyncWrite},
    task::ready,
};
use byteorder::{ByteOrder, BE};
use eyre::Result;
#[cfg(feature = "progress")]
use indicatif::{ProgressBar, ProgressStyle, ProgressDrawTarget};
use pin_project::pin_project;
#[cfg(all(not(target_family = "wasm"), feature = "parallel"))]
use rayon::{prelude::ParallelIterator, slice::ParallelSliceMut};
use sha1_smol::Sha1;
use std::task::Poll;
use std::{future::Future, io::SeekFrom};
use std::{pin::Pin, task::Context};

use crate::{
    crypto::{aes_encrypt_inplace, consts, AesKey, Unpackable},
    iso::disc::{
        align_addr, disc_set_header, to_decrypted_addr, to_encrypted_addr, PartHeader, TMDContent,
        TitleMetaData, WiiDiscHeader,
    },
};

use super::disc::{decrypt_title_key, DiscType, WiiDisc, WiiPartition};

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
    ) -> Poll<Result<u64, std::io::Error>> {
        self.project().writer.poll_seek(cx, pos)
    }
}

impl<W> AsyncWrite for GCDiscWriter<W>
where
    W: AsyncWrite + AsyncRead + AsyncSeek,
{
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> Poll<Result<usize, std::io::Error>> {
        self.project().writer.poll_write(cx, buf)
    }

    fn poll_flush(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<std::io::Result<()>> {
        self.project().writer.poll_flush(cx)
    }

    fn poll_close(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<std::io::Result<()>> {
        self.project().writer.poll_close(cx)
    }
}

// ---

#[derive(Debug, Clone, Default)]
enum WiiDiscWriterState {
    /// Seek for writing
    #[default]
    SeekingToStart,
    SeekingNextBlock(usize, u64, Vec<u8>),
    Writing(usize, usize, Vec<u8>),
}

#[derive(Debug, Clone, Default)]
enum WiiDiscWriterFinalizeState {
    #[default]
    Init,
    SeekToPadding(u64),
    WritePadding(Vec<u8>),
    SeekToStart(Option<AesKey>),

    LoadGroup(Option<AesKey>, usize, usize, Vec<u8>),
    SeekBack(Option<AesKey>, usize, Vec<u8>),
    WriteGroup(Option<AesKey>, usize, Vec<u8>),

    SeekToPartHeader(AesKey, Vec<u8>),
    WritePartHeader(AesKey, Vec<u8>),

    Done,
}

#[derive(Debug)]
#[pin_project]
pub struct WiiDiscWriter<W: AsyncWrite + AsyncRead + AsyncSeek + Unpin> {
    pub disc: WiiDisc,
    hashes: Vec<[u8; consts::WII_HASH_SIZE]>,
    // Virtual cursor which tracks where in the decrypted partition we are writing from.
    cursor: u64,
    state: WiiDiscWriterState,
    finalize_state: WiiDiscWriterFinalizeState,
    #[pin]
    writer: W,
    #[cfg(feature = "progress")]
    bar: ProgressBar,
}

fn hash_group(buf: &mut [u8]) -> [u8; consts::WII_HASH_SIZE] {
    // h0
    #[cfg(all(not(target_family = "wasm"), feature = "parallel"))]
    let data_pool = buf.par_chunks_exact_mut(consts::WII_SECTOR_SIZE);
    #[cfg(any(target_family = "wasm", not(feature = "parallel")))]
    let data_pool = buf.chunks_exact_mut(consts::WII_SECTOR_SIZE);
    fn h0_process(data: &mut [u8]) {
        let (hash, data) = data.split_at_mut(consts::WII_SECTOR_HASH_SIZE);
        for j in 0..31 {
            hash[j * 20..(j + 1) * 20].copy_from_slice(
                &Sha1::from(
                    &data[j * consts::WII_SECTOR_HASH_SIZE..][..consts::WII_SECTOR_HASH_SIZE],
                )
                .digest()
                .bytes(),
            );
        }
    }
    data_pool.for_each(h0_process);
    // h1
    #[cfg(all(not(target_family = "wasm"), feature = "parallel"))]
    let data_pool = buf.par_chunks_exact_mut(consts::WII_SECTOR_SIZE * 8);
    #[cfg(any(target_family = "wasm", not(feature = "parallel")))]
    let data_pool = buf.chunks_exact_mut(consts::WII_SECTOR_SIZE * 8);
    fn h1_process(data: &mut [u8]) {
        let mut hash = [0u8; consts::WII_HASH_SIZE * 8];
        for j in 0..8 {
            hash[j * consts::WII_HASH_SIZE..(j + 1) * consts::WII_HASH_SIZE].copy_from_slice(
                &Sha1::from(&data[j * consts::WII_SECTOR_SIZE..][..0x26c])
                    .digest()
                    .bytes()[..],
            );
        }
        for j in 0..8 {
            data[j * consts::WII_SECTOR_SIZE + 0x280..][..0xa0].copy_from_slice(&hash);
        }
    }
    data_pool.for_each(h1_process);
    // h2
    #[cfg(all(not(target_family = "wasm"), feature = "parallel"))]
    let data_pool = buf.par_chunks_exact_mut(consts::WII_SECTOR_SIZE * 64);
    #[cfg(any(target_family = "wasm", not(feature = "parallel")))]
    let data_pool = buf.chunks_exact_mut(consts::WII_SECTOR_SIZE * 64);
    fn h2_process(h: &mut [u8]) {
        let mut hash = [0u8; consts::WII_HASH_SIZE * 8];
        for j in 0..8 {
            hash[j * consts::WII_HASH_SIZE..(j + 1) * consts::WII_HASH_SIZE].copy_from_slice(
                &Sha1::from(&h[j * 8 * consts::WII_SECTOR_SIZE + 0x280..][..0xa0])
                    .digest()
                    .bytes()[..],
            );
        }
        for j in 0..64 {
            h[j * consts::WII_SECTOR_SIZE + 0x340..][..0xa0].copy_from_slice(&hash);
        }
    }
    data_pool.for_each(h2_process);
    // single H3
    let mut ret_buf = [0u8; consts::WII_HASH_SIZE];
    ret_buf.copy_from_slice(&Sha1::from(&buf[0x340 + 1..][..0xa0]).digest().bytes());
    ret_buf
}

fn fake_sign(part: &mut WiiPartition, hashes: &[[u8; consts::WII_HASH_SIZE]]) {
    let content = &mut part.tmd.contents[0];
    let mut hashes_ = Vec::with_capacity(consts::WII_H3_SIZE);
    hashes_.extend(hashes.iter().flatten());
    hashes_.extend(
        std::iter::repeat(0).take(consts::WII_H3_SIZE - hashes.len() * consts::WII_HASH_SIZE),
    );
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
    part.tmd.fake_sign();
    part.header.ticket.fake_sign();
}

fn encrypt_group(group: &mut [u8], part_key: AesKey) {
    crate::trace!("Encrypting group");
    #[cfg(all(not(target_family = "wasm"), feature = "parallel"))]
    let data_pool = group.par_chunks_exact_mut(consts::WII_SECTOR_SIZE);
    #[cfg(any(target_family = "wasm", not(feature = "parallel")))]
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
    W: AsyncWrite + AsyncRead + AsyncSeek + Unpin,
{
    pub async fn init(disc: WiiDisc, writer: W) -> Result<Self> {
        crate::trace!("Writing Wii Disc and Partition headers");
        let mut this = Self {
            disc,
            cursor: 0,
            state: WiiDiscWriterState::default(),
            finalize_state: WiiDiscWriterFinalizeState::default(),
            hashes: Vec::new(),
            writer,
            #[cfg(feature = "progress")]
            bar: ProgressBar::hidden(),
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
        part.header.h3_offset = part.header.cert_offset + part.header.cert_size as u64;
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

    fn poll_close_disc(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.project();
        let mut part = &mut this.disc.partitions.partitions[this.disc.partitions.data_idx];
        let finalize_state = std::mem::take(this.finalize_state);
        #[cfg(feature = "progress")]
        {
            if !this.bar.is_hidden() {
                this.bar.tick();
            }
        }
        match finalize_state {
            WiiDiscWriterFinalizeState::Init => {
                crate::trace!("WiiDiscWriterFinalizeState::Init");
                // Align the encrypted data size to 21 bits
                let old_size = part.header.data_size;
                part.header.data_size = align_addr(part.header.data_size, 21);
                // Pad the rest of the data
                let padding_size = part.header.data_size - old_size;
                *this.finalize_state = if padding_size > 0 {
                    WiiDiscWriterFinalizeState::SeekToPadding(padding_size)
                } else {
                    WiiDiscWriterFinalizeState::SeekToStart(None)
                };
                let n_group = part.header.data_size / consts::WII_SECTOR_SIZE as u64 / 64;
                #[cfg(feature = "progress")]
                {
                    this.bar.reset();
                    this.bar.set_draw_target(ProgressDrawTarget::stderr());
                    this.bar.set_length(n_group * 2);
                    this.bar.set_style(
                        ProgressStyle::with_template(
                            "{prefix:.grey} {msg} {wide_bar} {percent}% {pos}/{len:6}",
                        ).map_err(|err| io::Error::new(io::ErrorKind::Other, err))?,
                    );
                    this.bar.set_prefix("[1/2]");
                    this.bar.set_message("Building hash table");
                }
                this.hashes.clear();
                this.hashes
                    .resize(n_group as usize, [0u8; consts::WII_HASH_SIZE]);
                cx.waker().wake_by_ref();
                Poll::Pending
            }
            WiiDiscWriterFinalizeState::SeekToPadding(padding_size) => {
                let pos = part.part_offset
                    + part.header.data_offset
                    + (part.header.data_size - padding_size);
                crate::trace!(
                    "WiiDiscWriterFinalizeState::SeekToPadding(at 0x{:08X})",
                    pos
                );
                if this.writer.poll_seek(cx, SeekFrom::Start(pos)).is_pending() {
                    crate::trace!("Pending...");
                    *this.finalize_state = WiiDiscWriterFinalizeState::SeekToPadding(padding_size);
                    return Poll::Pending;
                }
                crate::trace!("WiiDiscWriterFinalizeState::SeekToPadding succeeded");
                *this.finalize_state =
                    WiiDiscWriterFinalizeState::WritePadding(vec![0u8; padding_size as usize]);
                cx.waker().wake_by_ref();
                Poll::Pending
            }
            WiiDiscWriterFinalizeState::WritePadding(buf) => {
                crate::trace!(
                    "WiiDiscWriterFinalizeState::WritePadding(size=0x{:08X})",
                    buf.len()
                );
                let n_written = match this.writer.poll_write(cx, &buf) {
                    Poll::Ready(result) => result?,
                    Poll::Pending => {
                        crate::trace!("Pending...");
                        *this.finalize_state = WiiDiscWriterFinalizeState::WritePadding(buf);
                        return Poll::Pending;
                    }
                };
                *this.finalize_state = if n_written < buf.len() {
                    WiiDiscWriterFinalizeState::WritePadding(buf.split_at(n_written).1.to_vec())
                } else {
                    WiiDiscWriterFinalizeState::SeekToStart(None)
                };
                cx.waker().wake_by_ref();
                Poll::Pending
            }
            WiiDiscWriterFinalizeState::SeekToStart(part_key) => {
                crate::trace!(
                    "WiiDiscWriterFinalizeState::SeekToStart(has part key: {:?})",
                    part_key.is_some()
                );
                let pos = part.part_offset + part.header.data_offset;
                if this.writer.poll_seek(cx, SeekFrom::Start(pos)).is_pending() {
                    crate::trace!("Pending...");
                    *this.finalize_state = WiiDiscWriterFinalizeState::SeekToStart(part_key);
                    return Poll::Pending;
                }
                let n_group = part.header.data_size / consts::WII_SECTOR_SIZE as u64 / 64;
                if n_group > 0 {
                    *this.finalize_state = WiiDiscWriterFinalizeState::LoadGroup(
                        part_key,
                        0,
                        consts::WII_SECTOR_SIZE * 64,
                        vec![0u8; consts::WII_SECTOR_SIZE * 64],
                    );
                    cx.waker().wake_by_ref();
                    Poll::Pending
                } else {
                    *this.finalize_state = WiiDiscWriterFinalizeState::Done;
                    Poll::Ready(Ok(()))
                }
            }
            WiiDiscWriterFinalizeState::LoadGroup(part_key, group_idx, rem, mut buf) => {
                crate::trace!("WiiDiscWriterFinalizeState::LoadGroup(has part key: {:?}; group_idx=0x{:08X}; rem=0x{:08X})", part_key.is_some(), group_idx, rem);
                let offset = buf.len() - rem;
                let n_read = match this.writer.poll_read(cx, &mut buf[offset..]) {
                    Poll::Ready(result) => result?,
                    Poll::Pending => {
                        crate::trace!("Pending...");
                        *this.finalize_state =
                            WiiDiscWriterFinalizeState::LoadGroup(part_key, group_idx, rem, buf);
                        return Poll::Pending;
                    }
                };
                if n_read == 0 {
                    return Poll::Ready(Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        eyre::eyre!("Unexpected EOF while reading from the writer"),
                    )));
                }
                *this.finalize_state = if n_read < rem {
                    WiiDiscWriterFinalizeState::LoadGroup(part_key, group_idx, rem - n_read, buf)
                } else {
                    if let Some(key) = part_key {
                        encrypt_group(&mut buf, key);
                    } else {
                        // Hash the group
                        this.hashes[group_idx].copy_from_slice(&hash_group(&mut buf));
                    }
                    WiiDiscWriterFinalizeState::SeekBack(part_key, group_idx, buf)
                };
                cx.waker().wake_by_ref();
                Poll::Pending
            }
            WiiDiscWriterFinalizeState::SeekBack(part_key, group_idx, buf) => {
                crate::trace!(
                    "WiiDiscWriterFinalizeState::SeekBack(has part key: {:?}; group_idx=0x{:08X})",
                    part_key.is_some(),
                    group_idx
                );
                let pos = part.part_offset
                    + part.header.data_offset
                    + group_idx as u64 * consts::WII_SECTOR_SIZE as u64 * 64;
                if this.writer.poll_seek(cx, SeekFrom::Start(pos)).is_pending() {
                    crate::trace!("Pending...");
                    *this.finalize_state =
                        WiiDiscWriterFinalizeState::SeekBack(part_key, group_idx, buf);
                    return Poll::Pending;
                }
                *this.finalize_state =
                    WiiDiscWriterFinalizeState::WriteGroup(part_key, group_idx, buf);
                cx.waker().wake_by_ref();
                Poll::Pending
            }
            WiiDiscWriterFinalizeState::WriteGroup(part_key, group_idx, buf) => {
                crate::trace!("WiiDiscWriterFinalizeState::WriteGroup(has part key: {:?}; group_idx=0x{:08X}; size=0x{:08X})", part_key.is_some(), group_idx, buf.len());
                let n_written = match this.writer.poll_write(cx, &buf) {
                    Poll::Ready(result) => result?,
                    Poll::Pending => {
                        crate::trace!("Pending...");
                        *this.finalize_state =
                            WiiDiscWriterFinalizeState::WriteGroup(part_key, group_idx, buf);
                        return Poll::Pending;
                    }
                };
                if n_written < buf.len() {
                    *this.finalize_state = WiiDiscWriterFinalizeState::WriteGroup(
                        part_key,
                        group_idx,
                        buf.split_at(n_written).1.to_vec(),
                    );
                    cx.waker().wake_by_ref();
                    Poll::Pending
                } else {
                    let n_group = part.header.data_size / consts::WII_SECTOR_SIZE as u64 / 64;
                    if group_idx as u64 + 1 < n_group {
                        *this.finalize_state = WiiDiscWriterFinalizeState::LoadGroup(
                            part_key,
                            group_idx + 1,
                            consts::WII_SECTOR_SIZE * 64,
                            vec![0u8; consts::WII_SECTOR_SIZE * 64],
                        );
                        #[cfg(feature = "progress")]
                        {
                            this.bar.inc(1);
                        }
                        cx.waker().wake_by_ref();
                        Poll::Pending
                    } else if part_key.is_some() {
                        #[cfg(feature = "progress")]
                        {
                            this.bar.finish_and_clear();
                        }
                        *this.finalize_state = WiiDiscWriterFinalizeState::Done;
                        Poll::Ready(Ok(()))
                    } else {
                        // Hash the whole table and store the partition header again
                        fake_sign(part, this.hashes);
                        crate::trace!("Partition Header: {:?}", part.header);
                        let mut buf = Vec::with_capacity(
                            PartHeader::BLOCK_SIZE + part.tmd.get_size() + consts::WII_H3_SIZE,
                        );
                        //let mut buf = vec![0u8; PartHeader::BLOCK_SIZE + part.tmd.get_size()];
                        buf.extend_from_slice(&<[u8; PartHeader::BLOCK_SIZE]>::from(&part.header));
                        buf.extend(std::iter::repeat(0).take(part.tmd.get_size()));
                        buf.extend(this.hashes.iter().flatten());
                        buf.extend(
                            std::iter::repeat(0).take(
                                consts::WII_H3_SIZE - this.hashes.len() * consts::WII_HASH_SIZE,
                            ),
                        );
                        TitleMetaData::set_partition(&mut buf, PartHeader::BLOCK_SIZE, &part.tmd);
                        *this.finalize_state = WiiDiscWriterFinalizeState::SeekToPartHeader(
                            decrypt_title_key(&part.header.ticket),
                            buf,
                        );
                        cx.waker().wake_by_ref();
                        Poll::Pending
                    }
                }
            }
            WiiDiscWriterFinalizeState::SeekToPartHeader(part_key, buf) => {
                crate::trace!("WiiDiscWriterFinalizeState::SeekToPartHeader");
                if this
                    .writer
                    .poll_seek(cx, SeekFrom::Start(part.part_offset))
                    .is_pending()
                {
                    crate::trace!("Pending...");
                    *this.finalize_state =
                        WiiDiscWriterFinalizeState::SeekToPartHeader(part_key, buf);
                    return Poll::Pending;
                }
                *this.finalize_state = WiiDiscWriterFinalizeState::WritePartHeader(part_key, buf);
                cx.waker().wake_by_ref();
                Poll::Pending
            }
            WiiDiscWriterFinalizeState::WritePartHeader(part_key, buf) => {
                crate::trace!("WiiDiscWriterFinalizeState::WritePartHeader");
                match this.writer.poll_write(cx, &buf) {
                    Poll::Ready(result) => match result {
                        Ok(n_written) => {
                            *this.finalize_state = if n_written < buf.len() {
                                WiiDiscWriterFinalizeState::WritePartHeader(
                                    part_key,
                                    buf.split_at(n_written).1.to_vec(),
                                )
                            } else {
                                #[cfg(feature = "progress")]
                                {
                                    this.bar.set_prefix("[2/2]");
                                    this.bar.set_message("Encrypting partition");
                                }
                                WiiDiscWriterFinalizeState::SeekToStart(Some(part_key))
                            };
                            cx.waker().wake_by_ref();
                            Poll::Pending
                        }
                        Err(err) => {
                            *this.finalize_state = WiiDiscWriterFinalizeState::Init;
                            Poll::Ready(Err(err))
                        }
                    },
                    Poll::Pending => {
                        crate::trace!("Pending...");
                        *this.finalize_state =
                            WiiDiscWriterFinalizeState::WritePartHeader(part_key, buf);
                        Poll::Pending
                    }
                }
            }
            WiiDiscWriterFinalizeState::Done => Poll::Ready(Ok(())),
        }
    }

    pub fn finalize(&mut self) -> FinalizeFuture<'_, W> {
        crate::trace!("Finalizing the ISO");
        FinalizeFuture { writer: self }
    }
}

pub struct FinalizeFuture<'a, W: AsyncWrite + AsyncRead + AsyncSeek + Unpin> {
    pub(crate) writer: &'a mut WiiDiscWriter<W>,
}

impl<'a, W: AsyncWrite + AsyncRead + AsyncSeek + Unpin> Future for FinalizeFuture<'a, W> {
    type Output = io::Result<()>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let Self { writer } = &mut *self;
        let mut pinned = Pin::new(&mut **writer);
        ready!(pinned.as_mut().poll_close_disc(cx))?;
        let this = pinned.as_mut().project();
        this.writer.poll_close(cx)
    }
}

impl<W> Clone for WiiDiscWriter<W>
where
    W: AsyncWrite + AsyncRead + AsyncSeek + Unpin + Clone,
{
    fn clone(&self) -> Self {
        Self {
            disc: self.disc.clone(),
            cursor: self.cursor,
            state: self.state.clone(),
            finalize_state: self.finalize_state.clone(),
            hashes: self.hashes.clone(),
            writer: self.writer.clone(),
            #[cfg(feature = "progress")]
            bar: ProgressBar::hidden(),
        }
    }
}

impl<W> AsyncSeek for WiiDiscWriter<W>
where
    W: AsyncWrite + AsyncRead + AsyncSeek + Unpin,
{
    fn poll_seek(
        self: Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
        pos: std::io::SeekFrom,
    ) -> std::task::Poll<std::io::Result<u64>> {
        let this = self.project();
        match pos {
            SeekFrom::Current(pos) => {
                if *this.cursor as i64 + pos < 0i64 {
                    return Poll::Ready(Err(std::io::Error::new(
                        std::io::ErrorKind::Other,
                        "Invalid argument",
                    )));
                }
                *this.cursor = (*this.cursor as i64 + pos) as u64;
            }
            SeekFrom::End(pos) => {
                if *this.cursor as i64 + pos < 0i64 {
                    return Poll::Ready(Err(std::io::Error::new(
                        std::io::ErrorKind::Other,
                        "Invalid argument",
                    )));
                }
                *this.cursor = (to_decrypted_addr(
                    this.disc.partitions.partitions[this.disc.partitions.data_idx]
                        .header
                        .data_size,
                ) as i64
                    + pos) as u64;
            }
            SeekFrom::Start(pos) => {
                *this.cursor = pos;
            }
        }
        std::task::Poll::Ready(Ok(*this.cursor))
    }
}

impl<W> AsyncWrite for WiiDiscWriter<W>
where
    W: AsyncWrite + AsyncRead + AsyncSeek + Unpin,
{
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        let this = self.project();
        let part = &mut this.disc.partitions.partitions[this.disc.partitions.data_idx];
        if let WiiDiscWriterState::SeekingToStart = this.state {
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
        if let WiiDiscWriterState::SeekingToStart = this.state {
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
            WiiDiscWriterState::SeekingToStart => {
                let start_blk_addr = part.part_offset
                    + part.header.data_offset
                    + (start_blk_idx * consts::WII_SECTOR_SIZE) as u64;
                let start_blk_pos = (start_blk_idx * consts::WII_SECTOR_DATA_SIZE) as u64;
                let block_offset = std::cmp::max(0, vstart as i64 - start_blk_pos as i64) as u64;
                let seek_pos = start_blk_addr + block_offset + to_encrypted_addr(0u64);
                crate::trace!("Seeking to 0x{:08X}", seek_pos);
                if this
                    .writer
                    .poll_seek(cx, SeekFrom::Start(seek_pos))
                    .is_pending()
                {
                    *this.state = WiiDiscWriterState::SeekingToStart;
                    return Poll::Pending;
                }
                crate::trace!("Seeking succeeded");
                let buf_ = buf.to_vec();
                crate::trace!(
                    "0x{:08X}; 0x{:08X}; 0x{:08X}",
                    consts::WII_SECTOR_DATA_SIZE,
                    buf_.len(),
                    block_offset
                );
                *this.state = WiiDiscWriterState::Writing(
                    start_blk_idx,
                    std::cmp::min(
                        consts::WII_SECTOR_DATA_SIZE - block_offset as usize,
                        buf_.len(),
                    ),
                    buf_,
                );
                cx.waker().wake_by_ref();
                Poll::Pending
            }
            WiiDiscWriterState::SeekingNextBlock(block_idx, pos, buf) => {
                crate::trace!("Seeking to 0x{:08X}", pos);
                if this.writer.poll_seek(cx, SeekFrom::Start(pos)).is_pending() {
                    *this.state = WiiDiscWriterState::SeekingNextBlock(block_idx, pos, buf);
                    return Poll::Pending;
                }
                crate::trace!("Seeking succeeded");
                *this.state = WiiDiscWriterState::Writing(
                    block_idx,
                    std::cmp::min(consts::WII_SECTOR_DATA_SIZE, buf.len()),
                    buf,
                );
                cx.waker().wake_by_ref();
                Poll::Pending
            }
            WiiDiscWriterState::Writing(block_idx, size, buf_) => {
                crate::trace!("Writing 0x{:08X} bytes", size);
                let n_written = match this.writer.poll_write(cx, &buf_[..size]) {
                    Poll::Ready(result) => result?,
                    Poll::Pending => {
                        *this.state = WiiDiscWriterState::Writing(block_idx, size, buf_);
                        return Poll::Pending;
                    }
                };
                crate::trace!("Writing succeeded");
                if n_written < size {
                    *this.state = WiiDiscWriterState::Writing(
                        block_idx,
                        size - n_written,
                        buf_.split_at(n_written).1.to_vec(),
                    );
                    cx.waker().wake_by_ref();
                    Poll::Pending
                } else {
                    let buf_ = buf_.split_at(n_written).1.to_vec();
                    if buf_.is_empty() {
                        // The write is done. Update the cursors and return.
                        crate::trace!("cursor before: 0x{:08X}", *this.cursor);
                        *this.cursor = vend;
                        crate::trace!("cursor end: 0x{:08X}", *this.cursor);
                        part.header.data_size =
                            std::cmp::max(part.header.data_size, to_encrypted_addr(*this.cursor));
                        // The state is already set to the initial state. We just need to return Ready
                        Poll::Ready(Ok(buf.len()))
                    } else {
                        let blk_addr = part.part_offset
                            + part.header.data_offset
                            + to_encrypted_addr(((block_idx + 1) * consts::WII_SECTOR_DATA_SIZE) as u64);
                        *this.state =
                            WiiDiscWriterState::SeekingNextBlock(block_idx + 1, blk_addr, buf_);
                        cx.waker().wake_by_ref();
                        Poll::Pending
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
#[pin_project(project = DiscWriterProj)]
pub enum DiscWriter<W: AsyncWrite + AsyncRead + AsyncSeek> {
    Gamecube(#[pin] GCDiscWriter<Pin<Box<W>>>),
    Wii(#[pin] Box<WiiDiscWriter<Pin<Box<W>>>>),
}

impl<W> AsyncSeek for DiscWriter<W>
where
    W: AsyncWrite + AsyncRead + AsyncSeek,
{
    fn poll_seek(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        pos: SeekFrom,
    ) -> Poll<std::io::Result<u64>> {
        match self.project() {
            DiscWriterProj::Gamecube(writer) => writer.poll_seek(cx, pos),
            DiscWriterProj::Wii(writer) => writer.poll_seek(cx, pos),
        }
    }
}

impl<W> AsyncWrite for DiscWriter<W>
where
    W: AsyncWrite + AsyncRead + AsyncSeek,
{
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        match self.project() {
            DiscWriterProj::Gamecube(writer) => writer.poll_write(cx, buf),
            DiscWriterProj::Wii(writer) => writer.poll_write(cx, buf),
        }
    }

    fn poll_flush(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<std::io::Result<()>> {
        match self.project() {
            DiscWriterProj::Gamecube(writer) => writer.poll_flush(cx),
            DiscWriterProj::Wii(writer) => writer.poll_flush(cx),
        }
    }

    fn poll_close(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<std::io::Result<()>> {
        match self.project() {
            DiscWriterProj::Gamecube(writer) => writer.poll_close(cx),
            DiscWriterProj::Wii(writer) => writer.poll_close(cx),
        }
    }
}

impl<W> DiscWriter<W>
where
    W: AsyncWrite + AsyncRead + AsyncSeek,
{
    pub async fn new_gc(writer: W) -> Result<Self> {
        Ok(Self::Gamecube(GCDiscWriter {
            writer: Box::pin(writer),
        }))
    }

    pub async fn new_wii(writer: W, disc: WiiDisc) -> Result<Self> {
        Ok(Self::Wii(Box::new(
            WiiDiscWriter::init(disc, Box::pin(writer)).await?,
        )))
    }

    pub async fn new(writer: W, disc_info: Option<WiiDisc>) -> Result<Self> {
        match disc_info {
            None => DiscWriter::new_gc(writer).await,
            Some(disc_indo) => DiscWriter::new_wii(writer, disc_indo).await,
        }
    }

    pub fn get_type(&self) -> DiscType {
        match self {
            DiscWriter::Gamecube(_) => DiscType::Gamecube,
            DiscWriter::Wii(_) => DiscType::Wii,
        }
    }

    pub async fn finalize(&mut self) -> io::Result<()> {
        match self {
            DiscWriter::Gamecube(_) => Ok(()),
            DiscWriter::Wii(writer) => writer.finalize().await,
        }
    }
}
