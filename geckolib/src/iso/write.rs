#[cfg(feature = "progress")]
use crate::UPDATER;
use async_std::{
    io::{prelude::*, Seek as AsyncSeek, Write as AsyncWrite},
    sync::Mutex,
};
use byteorder::{ByteOrder, BE};
use eyre::Result;
#[cfg(feature = "parallel")]
use rayon::{iter::IntoParallelRefMutIterator, prelude::ParallelIterator};
use sha1_smol::Sha1;
use std::io::SeekFrom;
use std::pin::{pin, Pin};
use std::{sync::Arc, task::Poll};

use crate::{
    crypto::{aes_encrypt_inplace, consts, AesKey, Unpackable},
    iso::disc::{
        align_addr, disc_set_header, to_raw_addr, PartHeader, TMDContent, TitleMetaData,
        WiiDiscHeader,
    },
};

use super::{disc::{
    decrypt_title_key, DiscType, WiiDisc, WiiGroup, WiiPartition, WiiSector, WiiSectorHash,
}, read::DiscReader};

#[derive(Debug, Clone, Default)]
enum WiiDiscWriterState {
    #[default]
    /// Setups the data to be written
    Init,
    /// Parse the data to be written
    Parse(u64, u64, Vec<u8>),
    /// Writing the accumulated data
    Writing(u64, u64, Vec<u8>, Vec<u8>),

    // States for the closing process
    SeekToLastGroup(u64, Vec<u8>),
    WriteLastGroup(u64, Vec<u8>),

    SeekToPartHeader(Vec<u8>),
    WritePartHeader(Vec<u8>),

    CloseWriter,
    Done,
}

#[derive(Debug)]
struct WiiDiscWriterStatus {
    initialized: bool,
    pub disc: WiiDisc,
    hashes: Vec<[u8; consts::WII_HASH_SIZE]>,
    // Virtual cursor which tracks where in the decrypted partition we are writing from.
    cursor: u64,
    state: WiiDiscWriterState,
    group: Box<WiiGroup>,
}

#[derive(Debug)]
pub struct WiiDiscWriter<W> {
    writer: W,
    status: Arc<Mutex<WiiDiscWriterStatus>>,
}

impl<W> Clone for WiiDiscWriter<W>
where
    W: Clone,
{
    fn clone(&self) -> Self {
        Self {
            writer: self.writer.clone(),
            status: self.status.clone(),
        }
    }
}

fn h0_process(sector: &mut WiiSector) {
    let hash = &mut sector.hash;
    let data = &sector.data;
    for j in 0..consts::WII_SECTOR_DATA_HASH_COUNT {
        hash.h0[j as usize].copy_from_slice(
            &Sha1::from(
                &data[(j * consts::WII_SECTOR_DATA_HASH_SIZE) as usize
                    ..((j + 1) * consts::WII_SECTOR_DATA_HASH_SIZE) as usize],
            )
            .digest()
            .bytes(),
        );
    }
}

fn h1_process(sectors: &mut [WiiSector]) {
    let mut hash = [0u8; consts::WII_HASH_SIZE * 8];
    for j in 0..8 {
        hash[j * consts::WII_HASH_SIZE..(j + 1) * consts::WII_HASH_SIZE]
            .copy_from_slice(&Sha1::from(sectors[j].hash.get_h0_ref()).digest().bytes()[..]);
    }
    #[cfg(feature = "parallel")]
    let pool = sectors.par_iter_mut();
    #[cfg(not(feature = "parallel"))]
    let pool = sectors.iter_mut();
    pool.map(|s: &mut WiiSector| &mut s.hash)
        .for_each(|h: &mut WiiSectorHash| {
            h.get_h1_mut().copy_from_slice(&hash);
        });
}

fn h2_process(sectors: &mut [&mut WiiSector]) -> [u8; consts::WII_HASH_SIZE * 8] {
    let mut hash = [0u8; consts::WII_HASH_SIZE * 8];
    for i in 0..8 {
        hash[i * consts::WII_HASH_SIZE..(i + 1) * consts::WII_HASH_SIZE].copy_from_slice(
            &Sha1::from(sectors[i * 8].hash.get_h1_ref())
                .digest()
                .bytes()[..],
        );
    }
    #[cfg(feature = "parallel")]
    let pool = sectors.par_iter_mut();
    #[cfg(not(feature = "parallel"))]
    let pool = sectors.iter_mut();
    pool.map(|s| &mut s.hash).for_each(|h: &mut WiiSectorHash| {
        h.get_h2_mut().copy_from_slice(&hash);
    });
    hash
}

fn hash_group(group: &mut WiiGroup) -> [u8; consts::WII_HASH_SIZE] {
    // h0
    #[cfg(feature = "parallel")]
    group
        .as_sectors_mut()
        .par_iter_mut()
        .for_each(|s| h0_process(s));
    #[cfg(not(feature = "parallel"))]
    group
        .as_sectors_mut()
        .iter_mut()
        .for_each(|s| h0_process(*s));
    // h1
    #[cfg(feature = "parallel")]
    group
        .sub_groups
        .par_iter_mut()
        .map(|sb| &mut sb.sectors[..])
        .for_each(h1_process);
    #[cfg(not(feature = "parallel"))]
    group
        .sub_groups
        .iter_mut()
        .map(|sb| &mut sb.sectors[..])
        .for_each(h1_process);
    // h2
    let hash = h2_process(&mut group.as_sectors_mut());
    // single H3
    let mut ret_buf = [0u8; consts::WII_HASH_SIZE];
    ret_buf.copy_from_slice(&Sha1::from(hash).digest().bytes());
    ret_buf
}

/// Implementation of the Segher's fake signing algorithm
fn fake_sign(part: &mut WiiPartition, hashes: &[[u8; consts::WII_HASH_SIZE]]) {
    let content = &mut part.tmd.contents[0];
    let mut hashes_ = Vec::with_capacity(consts::WII_H3_SIZE as usize);
    hashes_.extend(hashes.iter().flatten());
    hashes_.resize(consts::WII_H3_SIZE as usize, 0);
    crate::debug!(
        "[fake_sign] Hashes size: 0x{:08X}; Hashes padding size: 0x{:08X}; H3 size: 0x{:08X}",
        hashes.len() * consts::WII_HASH_SIZE,
        consts::WII_H3_SIZE - hashes.len() as u64 * consts::WII_HASH_SIZE as u64,
        hashes_.len()
    );
    content
        .hash
        .copy_from_slice(&Sha1::from(&hashes_).digest().bytes());

    // Fake sign tmd
    if let Err(err) = part.tmd.fake_sign() {
        crate::warn!("Error while signing TMD: {}", err);
    }
    if let Err(err) = part.header.ticket.fake_sign() {
        crate::warn!("Error while signing Ticket: {}", err);
    }
}

fn encrypt_group(group: &mut WiiGroup, part_key: AesKey) {
    crate::trace!("Encrypting group");
    #[cfg(feature = "parallel")]
    let data_pool = group
        .sub_groups
        .par_iter_mut()
        .flat_map(|sb| sb.sectors.par_iter_mut());
    #[cfg(not(feature = "parallel"))]
    let data_pool = group
        .sub_groups
        .iter_mut()
        .flat_map(|sb| sb.sectors.iter_mut());
    let encrypt_process = |sector: &mut WiiSector| {
        let mut iv = [0u8; consts::WII_KEY_SIZE];
        aes_encrypt_inplace(sector.hash.as_array_mut(), &iv, &part_key);
        iv[..consts::WII_KEY_SIZE].copy_from_slice(
            &sector.hash.as_array_mut()[consts::WII_SECTOR_IV_OFF as usize..][..consts::WII_KEY_SIZE],
        );
        aes_encrypt_inplace(&mut sector.data, &iv, &part_key);
    };
    data_pool.for_each(encrypt_process);
}

impl<W> WiiDiscWriter<W>
where
    W: AsyncWrite + AsyncSeek + Unpin,
{
    pub fn new(disc: WiiDisc, writer: W) -> Self {
        Self {
            writer,
            status: Arc::new(Mutex::new(WiiDiscWriterStatus {
                initialized: false,
                disc,
                cursor: 0,
                state: WiiDiscWriterState::default(),
                group: Box::new(WiiGroup::default()),
                hashes: Vec::new(),
            })),
        }
    }

    pub async fn init(self: &mut Pin<&mut Self>) -> Result<()> {
        crate::trace!("Writing Wii Disc and Partition headers");
        let this = self;
        let mut state = this.status.lock_arc().await;

        if state.initialized {
            return Ok(());
        }

        // Write ISO header
        let mut buf = vec![0u8; WiiDiscHeader::BLOCK_SIZE];
        let disc = &mut state.disc;
        disc_set_header(&mut buf, &disc.disc_header);
        this.writer.seek(SeekFrom::Start(0)).await?;
        this.writer.write_all(&buf).await?;

        // Get to the Partition Info
        this.writer
            .write_all(&vec![0u8; 0x40000 - WiiDiscHeader::BLOCK_SIZE])
            .await?;

        // Write Partition Info
        let part_idx = disc.partitions.data_idx;
        let mut buf = [0u8; 0x28];
        BE::write_u32(&mut buf[..], 1);
        BE::write_u32(&mut buf[4..], 0x40020 >> 2);
        let offset: u64 = 0x50000;
        let i = 0;
        let part_type: u32 = disc.partitions.partitions[part_idx].part_type.into();
        crate::debug!("part_type: {}", part_type);
        disc.partitions.partitions[part_idx].part_offset = offset;
        BE::write_u32(&mut buf[0x20 + (8 * i)..], (offset >> 2) as u32);
        BE::write_u32(&mut buf[0x20 + (8 * i) + 4..], part_type);
        this.writer.write_all(&buf).await?;

        // Get to Region area
        const REGION_OFFSET: usize = 0x4E000;
        let buf = vec![0u8; REGION_OFFSET - 0x40028];
        this.writer.write_all(&buf).await?;

        // Write Region area
        let mut buf = [0u8; 0x20];
        disc.disc_region.compose_into(&mut buf);
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
        if disc.partitions.partitions[part_idx]
            .tmd
            .contents
            .is_empty()
        {
            crate::warn!("TMD has no content value. Generating new value");
            disc.partitions.partitions[part_idx]
                .tmd
                .contents
                .push(TMDContent {
                    content_id: 0,
                    index: 0,
                    content_type: 1,
                    size: 0x1E0000,
                    hash: [0u8; consts::WII_HASH_SIZE],
                });
        }

        // Write (partial) partition header
        {
            let part = &mut disc.partitions.partitions[part_idx];
            part.header.data_size = 0;
            part.header.tmd_size = part.tmd.get_size();
            part.header.tmd_offset = PartHeader::BLOCK_SIZE as u64;
            part.header.cert_offset = part.header.tmd_offset + part.header.tmd_size as u64;
            part.header.h3_offset = std::cmp::max(
                consts::WII_H3_OFFSET,
                part.header.cert_offset + part.header.cert_size as u64,
            );
            part.header.data_offset =
                align_addr(part.header.cert_offset + part.header.cert_size as u64, 17);
        }
        let buf =
            <[u8; PartHeader::BLOCK_SIZE]>::from(&disc.partitions.partitions[part_idx].header);
        this.writer.write_all(&buf).await?;
        let mut buf = vec![0u8; disc.partitions.partitions[part_idx].header.tmd_size];
        TitleMetaData::set_partition(&mut buf, 0, &disc.partitions.partitions[part_idx].tmd);
        this.writer.write_all(&buf).await?;
        // Write certificate
        let cert = disc.partitions.partitions[part_idx].cert.clone();
        this.writer.write_all(&cert).await?;
        let padding_size = std::cmp::max(
            0,
            disc.partitions.partitions[part_idx].header.data_offset as i64
                - (disc.partitions.partitions[part_idx].header.h3_offset as i64),
        );
        if padding_size > 0 {
            let buf = vec![0u8; padding_size as usize];
            this.writer.write_all(&buf).await?;
        }
        let pos = this.writer.seek(SeekFrom::Current(0)).await?;
        let data_offset = disc.partitions.partitions[part_idx].part_offset
            + disc.partitions.partitions[part_idx].header.data_offset;
        if data_offset > pos {
            this.writer
                .write_all(&vec![0u8; (data_offset - pos) as usize])
                .await?;
        }
        state.initialized = true;
        Ok(())
    }
}

fn prepare_header(part: &mut WiiPartition, hashes: &[[u8; consts::WII_HASH_SIZE]]) -> Vec<u8> {
    // Hash the whole table and return the partition header
    fake_sign(part, hashes);
    #[cfg(feature = "log")]
    let part_offset = part.part_offset;
    crate::debug!("Partition offset: 0x{part_offset:08X?}");
    crate::trace!("Partition Header: {:?}", part.header);
    let mut buf = Vec::with_capacity((part.header.h3_offset + consts::WII_H3_SIZE) as usize);
    let h3_padding =
        part.header.h3_offset as usize - (PartHeader::BLOCK_SIZE + part.tmd.get_size());
    //let mut buf = vec![0u8; PartHeader::BLOCK_SIZE + part.tmd.get_size()];
    buf.extend_from_slice(&<[u8; PartHeader::BLOCK_SIZE]>::from(&part.header));
    buf.extend(std::iter::repeat(0).take(part.tmd.get_size() + h3_padding));
    buf.extend(hashes.iter().flatten());
    buf.extend(
        std::iter::repeat(0).take((consts::WII_H3_SIZE - hashes.len() as u64 * consts::WII_HASH_SIZE as u64) as usize),
    );
    TitleMetaData::set_partition(&mut buf, PartHeader::BLOCK_SIZE, &part.tmd);
    buf
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
        let this = self.get_mut();
        let mut status = match this.status.try_lock_arc() {
            Some(state) => state,
            None => {
                cx.waker().wake_by_ref();
                return Poll::Pending;
            }
        };
        let part_idx = status.disc.partitions.data_idx;
        if let WiiDiscWriterState::Init = status.state {
            crate::trace!("Pooling WiiDiscWriter for write ({} byte(s))", buf.len());
        }
        // If the requested size is 0, or if we are done reading, return without changing buf.
        if buf.is_empty() {
            return Poll::Ready(Ok(0));
        }
        // The "virtual" start and end, in the sense that they are the positions within the decrypted partition.
        let vstart = status.cursor;
        let vend = vstart + buf.len() as u64;
        let start_blk_idx = vstart / consts::WII_SECTOR_DATA_SIZE;
        let end_blk_idx = (vend - 1) / consts::WII_SECTOR_DATA_SIZE;
        let start_group_idx = start_blk_idx / 64;
        let start_block_idx_in_group = start_blk_idx % 64;
        let end_group_idx = end_blk_idx / 64;
        let end_block_idx_in_group = end_blk_idx % 64;
        if let WiiDiscWriterState::Init = status.state {
            crate::trace!(
                "Writing data from 0x{:08X} to 0x{:08X} (spanning {} block(s), from {} to {})",
                vstart,
                vend,
                end_blk_idx - start_blk_idx + 1,
                start_blk_idx,
                end_blk_idx,
            );
        }

        // Virtual space
        // Existing  ----~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~
        //               ^ vstart                     ^ vend
        // Data          -----------------------------

        // Real space
        //G|~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~|-----------------------------------------------------------------------|-------
        //S|~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~|-----------------------------------|-----------------------------------|-------
        //B|G~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~|BSG--------------|BSG--------------|BSG--------------|BSG--------------|___----
        //                                           ^
        //Buffer                             |___----~~~~~~~~~~ ___~~~~~~~~~~~~~~ ___~~~~~~~~~~~~~~ ___~~~~~~~~~~~~~~|
        //Data                                       ----------    --------------    -----
        //                                           ^ start                              ^ end

        let state = std::mem::take(&mut status.state);
        match state {
            WiiDiscWriterState::Init => {
                status.state =
                    WiiDiscWriterState::Parse(status.cursor, start_group_idx, buf.to_vec());
                cx.waker().wake_by_ref();
                Poll::Pending
            }
            WiiDiscWriterState::Parse(mut cursor, group_idx, in_buf) => {
                let start_blk = std::cmp::max(
                    start_group_idx * 64 + start_block_idx_in_group,
                    group_idx * 64,
                );
                let end_blk = std::cmp::min(
                    end_group_idx * 64 + end_block_idx_in_group,
                    group_idx * 64 + 63,
                );
                let mut curr_buf = &in_buf[..];
                for i in start_blk..=end_blk {
                    // Offsets in the group buffer (decrypted address)
                    let buffer_start =
                        std::cmp::max(cursor, i * consts::WII_SECTOR_DATA_SIZE)
                            % consts::WII_SECTOR_DATA_SIZE;
                    let buffer_end = std::cmp::min(
                        (cursor + in_buf.len() as u64) - 1,
                        (i + 1) * consts::WII_SECTOR_DATA_SIZE - 1,
                    ) % consts::WII_SECTOR_DATA_SIZE
                        + 1;
                    let size = (buffer_end - buffer_start) as usize;
                    assert!(size <= 0x7C00);
                    crate::trace!(
                        "Caching block #{} (0x{:08X} to 0x{:08X}) (remaining: 0x{:X} byte(s))",
                        i,
                        buffer_start,
                        buffer_end,
                        curr_buf.len() as u64 - (buffer_end - buffer_start),
                    );
                    let data;
                    (data, curr_buf) = curr_buf.split_at(size);
                    status.group.sub_groups[((i / 8) % 8) as usize].sectors[(i % 8) as usize].data
                        [buffer_start as usize..buffer_end as usize]
                        .copy_from_slice(data);
                    if buffer_end == consts::WII_SECTOR_DATA_SIZE * 64 {
                        crate::trace!("Reached end of group #{}", group_idx);
                    }
                }
                if (status.cursor + (buf.len() - curr_buf.len()) as u64)
                    % (consts::WII_SECTOR_DATA_SIZE * 64)
                    == 0
                {
                    // We are at the start of a group. We can hash and encrypt the group and write it.
                    crate::trace!("Hashing and encrypting group #{}", group_idx);
                    if status.hashes.len() as u64 <= group_idx {
                        status
                            .hashes
                            .resize(group_idx as usize + 1, [0u8; consts::WII_HASH_SIZE]);
                    }
                    let group_hash = hash_group(&mut status.group);
                    status.hashes[group_idx as usize].copy_from_slice(&group_hash);
                    let part_key =
                        decrypt_title_key(&status.disc.partitions.partitions[part_idx].header.ticket);
                    if !status.disc.disc_header.disable_disc_encrypt {
                        encrypt_group(&mut status.group, part_key);
                    }

                    status.state = WiiDiscWriterState::Writing(
                        cursor,
                        group_idx,
                        status.group.to_vec(),
                        curr_buf.to_vec(),
                    );
                    status.group.reset();

                    cx.waker().wake_by_ref();
                    Poll::Pending
                } else {
                    // We are in the middle of a group. We need to read the rest of the group.
                    cursor += in_buf.len() as u64;
                    status.cursor = cursor;
                    status.state = WiiDiscWriterState::Init;
                    Poll::Ready(Ok(buf.len()))
                }
            }
            WiiDiscWriterState::Writing(mut cursor, group_idx, group_buf, curr_buf) => {
                crate::trace!("Writing group #{}", group_idx);
                let n_written = match pin!(&mut this.writer).poll_write(cx, &group_buf) {
                    Poll::Ready(result) => result?,
                    Poll::Pending => {
                        status.state =
                            WiiDiscWriterState::Writing(cursor, group_idx, group_buf, curr_buf);
                        return Poll::Pending;
                    }
                };
                crate::trace!("Writing succeeded");
                if n_written < group_buf.len() {
                    status.state = WiiDiscWriterState::Writing(
                        cursor,
                        group_idx,
                        group_buf.split_at(n_written).1.to_vec(),
                        curr_buf,
                    );
                    cx.waker().wake_by_ref();
                    Poll::Pending
                } else {
                    if cursor % (consts::WII_SECTOR_DATA_SIZE * 64) == 0 {
                        cursor += consts::WII_SECTOR_DATA_SIZE * 64;
                    } else {
                        cursor = (cursor / (consts::WII_SECTOR_DATA_SIZE * 64) + 1)
                            * consts::WII_SECTOR_DATA_SIZE
                            * 64;
                    }
                    if curr_buf.is_empty() {
                        // The write is done.
                        crate::trace!("Write done at block #{}", group_idx);
                        status.cursor = cursor;
                        status.state = WiiDiscWriterState::Init;
                        Poll::Ready(Ok(buf.len()))
                    } else {
                        // We need to write the rest of the buffer
                        status.state =
                            WiiDiscWriterState::Parse(cursor, group_idx + 1, curr_buf);
                        cx.waker().wake_by_ref();
                        Poll::Pending
                    }
                }
            }
            _ => {
                crate::error!("Unexpected state: {:?}", state);
                Poll::Ready(Err(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    "Unexpected state",
                )))
            }
        }
    }

    fn poll_flush(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<std::io::Result<()>> {
        pin!(&mut self.get_mut().writer).poll_flush(cx)
    }

    fn poll_close(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<std::io::Result<()>> {
        crate::debug!("poll_close of WiiDiscWriter");
        let this = self.get_mut();
        let mut status = match this.status.try_lock_arc() {
            Some(status) => status,
            None => {
                cx.waker().wake_by_ref();
                return Poll::Pending;
            }
        };
        let part_idx = status.disc.partitions.data_idx;
        let state = std::mem::take(&mut status.state);
        #[cfg(feature = "progress")]
        if let Ok(mut updater) = UPDATER.lock() {
            updater.tick();
        }
        match state {
            WiiDiscWriterState::Init => {
                crate::trace!("WiiDiscWriterFinalizeState::Init");
                // Align the encrypted data size to 21 bits
                status.disc.partitions.partitions[part_idx].header.data_size =
                    align_addr(to_raw_addr(status.cursor), 21);

                // Hash and encrypt the last group
                let n_group = status.disc.partitions.partitions[part_idx].header.data_size
                    / consts::WII_SECTOR_SIZE as u64
                    / 64;
                let group_idx = (status.disc.partitions.partitions[part_idx].header.data_size - 1)
                    / consts::WII_SECTOR_SIZE as u64
                    / 64;
                crate::trace!("Hashing and encrypting group #{}", group_idx);
                if status.hashes.len() <= group_idx as usize {
                    status
                        .hashes
                        .resize(group_idx as usize + 1, [0u8; consts::WII_HASH_SIZE]);
                }
                let group_hash = hash_group(&mut status.group);
                status.hashes[group_idx as usize].copy_from_slice(&group_hash);
                let part_key =
                    decrypt_title_key(&status.disc.partitions.partitions[part_idx].header.ticket);
                if !status.disc.disc_header.disable_disc_encrypt {
                    encrypt_group(&mut status.group, part_key);
                }

                status.state = if status.cursor % (consts::WII_SECTOR_DATA_SIZE * 64) != 0 {
                    WiiDiscWriterState::SeekToLastGroup(n_group - 1, status.group.to_vec())
                } else {
                    let hashes = status.hashes.clone();
                    WiiDiscWriterState::SeekToPartHeader(prepare_header(
                        &mut status.disc.partitions.partitions[part_idx],
                        &hashes,
                    ))
                };
                cx.waker().wake_by_ref();
                Poll::Pending
            }
            WiiDiscWriterState::SeekToLastGroup(group_idx, group_buf) => {
                crate::trace!(
                    "WiiDiscWriterFinalizeState::SeekToLastGroup(group_idx=0x{:08X})",
                    group_idx
                );
                let pos = status.disc.partitions.partitions[part_idx].part_offset
                    + status.disc.partitions.partitions[part_idx].header.data_offset
                    + group_idx * consts::WII_SECTOR_SIZE as u64 * 64;
                if pin!(&mut this.writer)
                    .poll_seek(cx, SeekFrom::Start(pos))
                    .is_pending()
                {
                    crate::trace!("Pending...");
                    status.state = WiiDiscWriterState::SeekToLastGroup(group_idx, group_buf);
                    return Poll::Pending;
                }
                status.state = WiiDiscWriterState::WriteLastGroup(group_idx, group_buf);
                cx.waker().wake_by_ref();
                Poll::Pending
            }
            WiiDiscWriterState::WriteLastGroup(group_idx, group_buf) => {
                crate::trace!(
                    "WiiDiscWriterFinalizeState::WriteLastGroup(group_idx=0x{:08X})",
                    group_idx
                );
                let n_written = match pin!(&mut this.writer).poll_write(cx, &group_buf) {
                    Poll::Ready(result) => result?,
                    Poll::Pending => {
                        crate::trace!("Pending...");
                        status.state = WiiDiscWriterState::WriteLastGroup(group_idx, group_buf);
                        return Poll::Pending;
                    }
                };
                status.state = if n_written < group_buf.len() {
                    WiiDiscWriterState::WriteLastGroup(
                        group_idx,
                        group_buf.split_at(n_written).1.to_vec(),
                    )
                } else {
                    let hashes = status.hashes.clone();
                    WiiDiscWriterState::SeekToPartHeader(prepare_header(
                        &mut status.disc.partitions.partitions[part_idx],
                        &hashes,
                    ))
                };
                cx.waker().wake_by_ref();
                Poll::Pending
            }
            WiiDiscWriterState::SeekToPartHeader(buf) => {
                crate::trace!("WiiDiscWriterFinalizeState::SeekToPartHeader");
                if let Poll::Ready(result) = pin!(&mut this.writer)
                    .poll_seek(
                        cx,
                        SeekFrom::Start(status.disc.partitions.partitions[part_idx].part_offset),
                    )
                {
                    match result {
                        Ok(new_pos) => {crate::trace!("Seeked to 0x{:08X}", new_pos);},
                        Err(err) => return Poll::Ready(Err(err)),
                    }
                } else {
                    crate::trace!("Pending...");
                    status.state = WiiDiscWriterState::SeekToPartHeader(buf);
                    return Poll::Pending;
                }
                status.state = WiiDiscWriterState::WritePartHeader(buf);
                cx.waker().wake_by_ref();
                Poll::Pending
            }
            WiiDiscWriterState::WritePartHeader(buf) => {
                crate::trace!("WiiDiscWriterFinalizeState::WritePartHeader");
                match pin!(&mut this.writer).poll_write(cx, &buf) {
                    Poll::Ready(result) => match result {
                        Ok(n_written) => {
                            if n_written < buf.len() {
                                status.state = WiiDiscWriterState::WritePartHeader(
                                    buf.split_at(n_written).1.to_vec(),
                                );
                                cx.waker().wake_by_ref();
                                Poll::Pending
                            } else {
                                status.state = WiiDiscWriterState::CloseWriter;
                                cx.waker().wake_by_ref();
                                Poll::Pending
                            }
                        }
                        Err(err) => {
                            status.state = WiiDiscWriterState::Init;
                            Poll::Ready(Err(err))
                        }
                    },
                    Poll::Pending => {
                        crate::trace!("Pending...");
                        status.state = WiiDiscWriterState::WritePartHeader(buf);
                        Poll::Pending
                    }
                }
            }
            WiiDiscWriterState::CloseWriter => {
                crate::trace!("WiiDiscWriterFinalizeState::CloseWriter");
                match pin!(&mut this.writer).poll_close(cx) {
                    Poll::Ready(result) => {
                        status.state = WiiDiscWriterState::Done;
                        Poll::Ready(result)
                    }
                    Poll::Pending => {
                        status.state = WiiDiscWriterState::CloseWriter;
                        Poll::Pending
                    }
                }
            }
            WiiDiscWriterState::Done => Poll::Ready(Ok(())),
            _ => {
                crate::error!("Unexpected state: {:?}", state);
                Poll::Ready(Err(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    "Unexpected state",
                )))
            }
        }
    }
}

// ---

#[derive(Debug, Clone)]
pub enum DiscWriter<W> {
    Gamecube(W),
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
            DiscWriter::Gamecube(writer) => pin!(writer).poll_write(cx, buf),
            DiscWriter::Wii(writer) => pin!(writer).poll_write(cx, buf),
        }
    }

    fn poll_flush(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<std::io::Result<()>> {
        match self.get_mut() {
            DiscWriter::Gamecube(writer) => pin!(writer).poll_flush(cx),
            DiscWriter::Wii(writer) => pin!(writer).poll_flush(cx),
        }
    }

    fn poll_close(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<std::io::Result<()>> {
        match self.get_mut() {
            DiscWriter::Gamecube(writer) => pin!(writer).poll_close(cx),
            DiscWriter::Wii(writer) => pin!(writer).poll_close(cx),
        }
    }
}

impl<W> DiscWriter<W> {
    pub fn new_gc(writer: W) -> Self {
        Self::Gamecube(writer)
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
    pub fn new_wii(writer: W, disc: WiiDisc) -> Self {
        Self::Wii(WiiDiscWriter::new(disc, writer))
    }

    pub fn new(writer: W, disc_info: Option<WiiDisc>) -> Self {
        match disc_info {
            None => DiscWriter::new_gc(writer),
            Some(disc_info) => DiscWriter::new_wii(writer, disc_info),
        }
    }

    pub fn from_reader<R>(writer: W, reader: &DiscReader<R>) -> Self {
        match reader {
            DiscReader::Gamecube(_) => DiscWriter::new_gc(writer),
            DiscReader::Wii(reader) => DiscWriter::new_wii(writer, reader.disc.to_owned()),
        }
    }
}

impl<W> DiscWriter<W>
where
    W: AsyncWrite + AsyncSeek + Clone + Unpin,
{
    pub async fn init(self: &mut Pin<&mut Self>) -> Result<()> {
        match self.as_mut().as_wii_disc_mut() {
            Some(writer) => std::pin::pin!(writer.clone()).init().await,
            None => Ok(()),
        }
    }
}

impl<W> DiscWriter<W>
{
    fn as_wii_disc_mut(&mut self) -> Option<&mut WiiDiscWriter<W>> {
        match self {
            DiscWriter::Wii(writer) => Some(writer),
            _ => None,
        }
    }
}
