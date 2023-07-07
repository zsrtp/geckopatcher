use std::io::SeekFrom;
use std::pin::Pin;
use std::task::Poll;

use async_std::io::prelude::SeekExt;
use async_std::io::ReadExt;
use async_std::task::ready;
use eyre::Result;
#[cfg(feature = "log")]
use log::{debug, warn, trace};
#[cfg(not(target = "wasm32"))]
use rayon::prelude::*;

use crate::crypto::{consts, Unpackable};
use crate::crypto::Aes128Cbc;
use crate::crypto::AesKey;
use crate::crypto::WiiCryptoError;
use crate::crypto::COMMON_KEY;
use crate::crypto::{aes_decrypt_inplace, aes_encrypt_inplace};
use crate::declare_tryfrom;
use async_std::io::{Read as AsyncRead, Seek as AsyncSeek};
use block_modes::BlockMode;
use byteorder::{ByteOrder, BE};
use pin_project::pin_project;
use sha1_smol::Sha1;
use std::convert::TryFrom;

#[derive(Copy, Clone, Debug)]
pub struct WiiDiscHeader {
    pub disc_id: char,
    pub game_code: [char; 2],
    pub region_code: char,
    pub maker_code: [char; 2],
    pub disc_number: u8,
    pub disc_version: u8,
    pub audio_streaming: bool,
    pub streaming_buffer_size: u8,
    pub unk1: [u8; 14],
    pub wii_magic: u32,
    pub gc_magic: u32,
    pub game_title: [char; 64],
    pub disable_hash_verif: u8,
    pub disable_disc_encrypt: u8,
}

impl Default for WiiDiscHeader {
    fn default() -> Self {
        Self {
            disc_id: Default::default(),
            game_code: Default::default(),
            region_code: Default::default(),
            maker_code: Default::default(),
            disc_number: Default::default(),
            disc_version: Default::default(),
            audio_streaming: Default::default(),
            streaming_buffer_size: Default::default(),
            unk1: Default::default(),
            wii_magic: Default::default(),
            gc_magic: Default::default(),
            game_title: ['\0'; 64],
            disable_hash_verif: Default::default(),
            disable_disc_encrypt: Default::default(),
        }
    }
}

impl Unpackable for WiiDiscHeader {
    const BLOCK_SIZE: usize = 0x62;
}

pub fn disc_get_header(raw: &[u8]) -> WiiDiscHeader {
    let mut unk1 = [0 as u8; 14];
    let mut game_title = [0 as char; 64];
    unsafe {
        unk1.copy_from_slice(&raw[0xA..0x18]);
        game_title.copy_from_slice(std::mem::transmute(&raw[0x20..0x60]));
    };
    WiiDiscHeader {
        disc_id: raw[0] as char,
        game_code: [raw[1] as char, raw[2] as char],
        region_code: raw[3] as char,
        maker_code: [raw[4] as char, raw[5] as char],
        disc_number: raw[6],
        disc_version: raw[7],
        audio_streaming: raw[8] != 0,
        streaming_buffer_size: raw[9],
        unk1: unk1,
        wii_magic: BE::read_u32(&raw[0x18..0x1C]),
        gc_magic: BE::read_u32(&raw[0x1C..0x20]),
        game_title: game_title,
        disable_hash_verif: raw[0x60],
        disable_disc_encrypt: raw[0x61],
    }
}

pub fn disc_set_header(buffer: &mut [u8], dh: &WiiDiscHeader) {
    unsafe {
        buffer[0x00] = dh.disc_id as u8;
        buffer[0x01] = dh.game_code[0] as u8;
        buffer[0x02] = dh.game_code[1] as u8;
        buffer[0x03] = dh.region_code as u8;
        buffer[0x04] = dh.maker_code[0] as u8;
        buffer[0x05] = dh.maker_code[1] as u8;
        buffer[0x06] = dh.disc_number;
        buffer[0x07] = dh.disc_version;
        buffer[0x08] = dh.audio_streaming as u8;
        buffer[0x09] = dh.streaming_buffer_size;
        (&mut buffer[0x0A..0x18]).copy_from_slice(&dh.unk1[..]);
        BE::write_u32(&mut buffer[0x18..], dh.wii_magic);
        BE::write_u32(&mut buffer[0x1C..], dh.gc_magic);
        (&mut buffer[0x20..0x60]).copy_from_slice(std::mem::transmute(&dh.game_title[..]));
        buffer[0x60] = dh.disable_hash_verif;
        buffer[0x61] = dh.disable_disc_encrypt;
    };
}

#[derive(Copy, Clone, Debug, Default)]
pub struct PartInfoEntry {
    pub part_type: u32,
    pub offset: u64,
}

#[derive(Clone, Debug, Default)]
pub struct PartInfo {
    pub offset: u64,
    pub entries: Vec<PartInfoEntry>,
}

pub fn disc_get_part_info(buf: &[u8]) -> PartInfo {
    #[cfg(feature = "log")]
    debug!("Parsing partition info");
    let mut entries: Vec<PartInfoEntry> = Vec::new();
    let n_part = BE::read_u32(&buf[consts::WII_PARTITION_INFO_OFF..]) as usize;
    let part_info_offset = (BE::read_u32(&buf[consts::WII_PARTITION_INFO_OFF + 4..]) as u64) << 2;
    #[cfg(feature = "log")]
    debug!(
        "Found {:} entries, partition info at offset 0x{:08X}",
        n_part, part_info_offset
    );
    for i in 0..n_part {
        entries.push(PartInfoEntry {
            offset: (BE::read_u32(&buf[part_info_offset as usize + (8 * i)..]) as u64) << 2,
            part_type: BE::read_u32(&buf[part_info_offset as usize + (8 * i) + 4..]),
        });
    }
    PartInfo {
        offset: part_info_offset,
        entries: entries,
    }
}

pub async fn disc_get_part_info_async<R: AsyncRead + AsyncSeek>(
    reader: &mut Pin<&mut R>,
) -> Result<PartInfo> {
    #[cfg(feature = "log")]
    debug!("Parsing partition info (async)");
    let mut entries: Vec<PartInfoEntry> = Vec::new();
    let mut buf: [u8; 8] = [0u8; 8];
    reader
        .seek(SeekFrom::Start(consts::WII_PARTITION_INFO_OFF as u64))
        .await?;
    reader.read(&mut buf).await?;
    let n_part: usize = BE::read_u32(&buf[..]) as usize;
    let part_info_offset = (BE::read_u32(&buf[4..]) as u64) << 2;
    #[cfg(feature = "log")]
    debug!(
        "Found {:} entries, partition info at offset 0x{:08X}",
        n_part, part_info_offset
    );
    for i in 0..n_part as u64 {
        reader
            .seek(SeekFrom::Start(part_info_offset + (8 * i)))
            .await?;
        reader.read(&mut buf).await?;
        entries.push(PartInfoEntry {
            offset: (BE::read_u32(&buf[..]) as u64) << 2,
            part_type: BE::read_u32(&buf[4..]),
        });
    }
    Ok(PartInfo {
        offset: part_info_offset,
        entries: entries,
    })
}

pub fn disc_set_part_info(buffer: &mut [u8], pi: &PartInfo) {
    BE::write_u32(&mut buffer[0x40000..], pi.entries.len() as u32);
    BE::write_u32(&mut buffer[0x40004..], (pi.offset >> 2) as u32);
    for (i, entry) in pi.entries.iter().enumerate() {
        BE::write_u32(
            &mut buffer[pi.offset as usize + (8 * i)..],
            (entry.offset >> 2) as u32,
        );
        BE::write_u32(
            &mut buffer[pi.offset as usize + (8 * i) + 4..],
            entry.part_type as u32,
        );
    }
}

#[derive(Copy, Clone, Debug)]
pub enum PartitionType {
    Data = 0,
    Update = 1,
    ChannelInstaller = 2,
    Unknown = -1,
}

impl Default for PartitionType {
    fn default() -> Self {
        PartitionType::Unknown
    }
}

impl From<PartitionType> for u32 {
    fn from(t: PartitionType) -> Self {
        match t {
            PartitionType::Data => 0u32,
            PartitionType::Update => 1u32,
            PartitionType::ChannelInstaller => 2u32,
            PartitionType::Unknown => 0xFFFFFFFFu32,
        }
    }
}

impl From<u32> for PartitionType {
    fn from(t: u32) -> Self {
        match t {
            0 => PartitionType::Data,
            1 => PartitionType::Update,
            2 => PartitionType::ChannelInstaller,
            _ => PartitionType::Unknown,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Ticket {
    pub sig_type: u32,
    pub sig: [u8; 0x100],
    pub sig_padding: [u8; 0x3C],
    pub sig_issuer: [char; 0x40],
    pub unk1: [u8; 0x3F],
    pub title_key: [u8; 0x10],
    pub unk2: u8,
    pub ticket_id: [u8; 8],
    pub console_id: u32,
    pub title_id: [u8; 8],
    pub unk3: u16,
    pub n_dlc: u16,
    pub unk4: [u8; 0x09],
    pub common_key_index: u8,
    pub unk5: [u8; 0x50],
    pub padding2: u16,
    pub enable_time_limit: u32,
    pub time_limit: u32,
    pub fake_sign: [u8; 0x58],
}
declare_tryfrom!(Ticket);

impl Unpackable for Ticket {
    const BLOCK_SIZE: usize = 0x2A4;
}

impl From<&[u8; Ticket::BLOCK_SIZE]> for Ticket {
    fn from(buf: &[u8; Ticket::BLOCK_SIZE]) -> Self {
        let mut sig = [0 as u8; 0x100];
        let mut sig_padding = [0 as u8; 0x3C];
        let mut sig_issuer = [0 as char; 0x40];
        let mut unk1 = [0 as u8; 0x3F];
        let mut title_key = [0 as u8; 0x10];
        let mut ticket_id = [0 as u8; 0x08];
        let mut title_id = [0 as u8; 0x08];
        let mut unk4 = [0 as u8; 0x09];
        let mut unk5 = [0 as u8; 0x50];
        let mut fake_sign = [0 as u8; 0x58];
        unsafe {
            sig.copy_from_slice(&buf[0x4..0x104]);
            sig_padding.copy_from_slice(&buf[0x104..0x140]);
            sig_issuer.copy_from_slice(std::mem::transmute(&buf[0x140..0x180]));
            unk1.copy_from_slice(&buf[0x180..0x1BF]);
            title_key.copy_from_slice(&buf[0x1BF..0x1CF]);
            ticket_id.copy_from_slice(&buf[0x1D0..0x1D8]);
            title_id.copy_from_slice(&buf[0x1DC..0x1E4]);
            unk4.copy_from_slice(&buf[0x1E8..0x1F1]);
            unk5.copy_from_slice(&buf[0x1F2..0x242]);
            fake_sign.copy_from_slice(&buf[0x24C..0x2A4]);
        };
        Ticket {
            sig_type: BE::read_u32(&buf[0x00..]),
            sig: sig,
            sig_padding: sig_padding,
            sig_issuer: sig_issuer,
            unk1: unk1,
            title_key: title_key,
            unk2: buf[0x1CF],
            ticket_id: ticket_id,
            console_id: BE::read_u32(&buf[0x1D8..]),
            title_id: title_id,
            unk3: BE::read_u16(&buf[0x1E4..]),
            n_dlc: BE::read_u16(&buf[0x1E6..]),
            unk4: unk4,
            common_key_index: buf[0x1F1],
            unk5: unk5,
            padding2: BE::read_u16(&buf[0x242..]),
            enable_time_limit: BE::read_u32(&buf[0x244..]),
            time_limit: BE::read_u32(&buf[0x248..]),
            fake_sign: fake_sign,
        }
    }
}

impl From<&Ticket> for [u8; Ticket::BLOCK_SIZE] {
    fn from(t: &Ticket) -> Self {
        let mut buf = [0 as u8; Ticket::BLOCK_SIZE];

        BE::write_u32(&mut buf[0x00..], t.sig_type);
        (&mut buf[0x04..0x104]).copy_from_slice(&t.sig[..]);
        (&mut buf[0x104..0x140]).copy_from_slice(&t.sig_padding[..]);
        unsafe {
            (&mut buf[0x140..0x180]).copy_from_slice(std::mem::transmute(&t.sig_issuer[..]));
        }
        (&mut buf[0x180..0x1BF]).copy_from_slice(&t.unk1[..]);
        (&mut buf[0x1BF..0x1CF]).copy_from_slice(&t.title_key[..]);
        buf[0x1CF] = t.unk2;
        (&mut buf[0x1D0..0x1D8]).copy_from_slice(&t.ticket_id[..]);
        BE::write_u32(&mut buf[0x1D8..], t.console_id);
        (&mut buf[0x1DC..0x1E4]).copy_from_slice(&t.title_id[..]);
        BE::write_u16(&mut buf[0x1E4..], t.unk3);
        BE::write_u16(&mut buf[0x1E6..], t.n_dlc);
        (&mut buf[0x1E8..0x1F1]).copy_from_slice(&t.unk4[..]);
        buf[0x1F1] = t.common_key_index;
        (&mut buf[0x1F2..0x242]).copy_from_slice(&t.unk5[..]);
        BE::write_u16(&mut buf[0x242..], t.padding2);
        BE::write_u32(&mut buf[0x244..], t.enable_time_limit);
        BE::write_u32(&mut buf[0x248..], t.time_limit);
        (&mut buf[0x24C..0x2A4]).copy_from_slice(&t.fake_sign[..]);
        buf
    }
}

impl Default for Ticket {
    fn default() -> Self {
        Ticket::try_from(&[0 as u8; Ticket::BLOCK_SIZE]).unwrap()
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct PartHeader {
    pub ticket: Ticket,
    pub tmd_size: usize,
    pub tmd_offset: u64,
    pub cert_size: usize,
    pub cert_offset: u64,
    pub h3_offset: u64,
    pub data_offset: u64,
    pub data_size: u64,
}
declare_tryfrom!(PartHeader);

impl Unpackable for PartHeader {
    const BLOCK_SIZE: usize = 0x2C0;
}

impl From<&[u8; PartHeader::BLOCK_SIZE]> for PartHeader {
    fn from(buf: &[u8; PartHeader::BLOCK_SIZE]) -> Self {
        PartHeader {
            ticket: Ticket::try_from(&buf[..0x2A4]).unwrap(),
            tmd_size: (BE::read_u32(&buf[0x2A4..]) as usize),
            tmd_offset: ((BE::read_u32(&buf[0x2A8..]) as u64) << 2),
            cert_size: (BE::read_u32(&buf[0x2AC..]) as usize),
            cert_offset: ((BE::read_u32(&buf[0x2B0..]) as u64) << 2),
            h3_offset: ((BE::read_u32(&buf[0x2B4..]) as u64) << 2),
            data_offset: ((BE::read_u32(&buf[0x2B8..]) as u64) << 2),
            data_size: ((BE::read_u32(&buf[0x2BC..]) as u64) << 2),
        }
    }
}

impl From<&PartHeader> for [u8; PartHeader::BLOCK_SIZE] {
    fn from(ph: &PartHeader) -> Self {
        let mut buf = [0 as u8; PartHeader::BLOCK_SIZE];
        (&mut buf[..0x2A4]).copy_from_slice(&<[u8; 0x2A4]>::from(&ph.ticket));
        BE::write_u32(&mut buf[0x2A4..], ph.tmd_size as u32);
        BE::write_u32(&mut buf[0x2A8..], (ph.tmd_offset >> 2) as u32);
        BE::write_u32(&mut buf[0x2AC..], ph.cert_size as u32);
        BE::write_u32(&mut buf[0x2B0..], (ph.cert_offset >> 2) as u32);
        BE::write_u32(&mut buf[0x2B4..], (ph.h3_offset >> 2) as u32);
        BE::write_u32(&mut buf[0x2B8..], (ph.data_offset >> 2) as u32);
        BE::write_u32(&mut buf[0x2BC..], (ph.data_size >> 2) as u32);
        buf
    }
}

#[derive(Debug, Clone, Copy)]
pub struct TMDContent {
    pub content_id: u32,
    pub index: u16,
    pub content_type: u16,
    pub size: u64,
    pub hash: [u8; consts::WII_HASH_SIZE],
}

#[derive(Debug, Clone)]
pub struct TitleMetaData {
    pub sig_type: u32,
    pub signature: [u8; 0x100],
    pub padding: [u8; 60],
    pub issuer: [u8; 0x40],
    pub version: u8,
    pub ca_crl_version: u8,
    pub signer_crl_version: u8,
    pub is_v_wii: bool,
    pub system_version: u64,
    pub title_id: [u8; 8],
    pub title_type: u32,
    pub group_id: u16,
    pub fake_sign: [u8; 0x3e],
    pub access_rights: u32,
    pub title_version: u16,
    pub n_content: u16,
    pub boot_index: u16,
    pub padding3: [u8; 2],
    pub contents: Vec<TMDContent>,
}

impl Default for TitleMetaData {
    fn default() -> Self {
        partition_get_tmd(&[0u8; 0x1E4], 0)
    }
}

pub fn partition_get_tmd(partition: &[u8], tmd_offset: usize) -> TitleMetaData {
    #[cfg(feature = "log")]
    debug!("Parsing TitleMetadata");
    let mut tmd = TitleMetaData {
        sig_type: BE::read_u32(&partition[tmd_offset..]),
        signature: [0u8; 0x100],
        padding: [0u8; 60],
        issuer: [0u8; 0x40],
        version: partition[tmd_offset + 0x180],
        ca_crl_version: partition[tmd_offset + 0x181],
        signer_crl_version: partition[tmd_offset + 0x182],
        is_v_wii: partition[tmd_offset + 0x183] != 0,
        system_version: BE::read_u64(&partition[tmd_offset + 0x184..]),
        title_id: [0u8; 8],
        title_type: BE::read_u32(&partition[tmd_offset + 0x194..]),
        group_id: BE::read_u16(&partition[tmd_offset + 0x198..]),
        fake_sign: [0u8; 0x3e],
        access_rights: BE::read_u32(&partition[tmd_offset + 0x1d8..]),
        title_version: BE::read_u16(&partition[tmd_offset + 0x1dc..]),
        n_content: BE::read_u16(&partition[tmd_offset + 0x1de..]),
        boot_index: BE::read_u16(&partition[tmd_offset + 0x1e0..]),
        padding3: [0u8; 2],
        contents: Vec::with_capacity(BE::read_u16(&partition[tmd_offset + 0x1de..]) as usize),
    };
    #[cfg(feature = "log")]
    debug!("TMD has {:} content entries", tmd.n_content);
    for i in 0..tmd.n_content as usize {
        let mut content = TMDContent {
            content_id: BE::read_u32(&partition[tmd_offset + 0x1E4 + i * 36..]),
            index: BE::read_u16(&partition[tmd_offset + 0x1E8 + i * 36..]),
            content_type: BE::read_u16(&partition[tmd_offset + 0x1EA + i * 36..]),
            size: BE::read_u64(&partition[tmd_offset + 0x1EC + i * 36..]),
            hash: [0u8; consts::WII_HASH_SIZE],
        };
        content
            .hash
            .copy_from_slice(&partition[tmd_offset + 0x1F4 + i * 36..][..consts::WII_HASH_SIZE]);
        tmd.contents.push(content);
    }
    tmd.signature
        .copy_from_slice(&partition[tmd_offset + 0x4..][..0x100]);
    tmd.padding
        .copy_from_slice(&partition[tmd_offset + 0x104..][..60]);
    tmd.issuer
        .copy_from_slice(&partition[tmd_offset + 0x140..][..0x40]);
    tmd.title_id
        .copy_from_slice(&partition[tmd_offset + 0x18c..][..8]);
    tmd.fake_sign
        .copy_from_slice(&partition[tmd_offset + 0x19a..][..0x3e]);
    tmd.padding3
        .copy_from_slice(&partition[tmd_offset + 0x1e2..][..2]);
    tmd
}

pub fn partition_set_tmd(partition: &mut [u8], tmd_offset: usize, tmd: &TitleMetaData) -> usize {
    BE::write_u32(&mut partition[tmd_offset..], tmd.sig_type);
    partition[tmd_offset + 0x4..][..0x100].copy_from_slice(&tmd.signature);
    partition[tmd_offset + 0x104..][..60].copy_from_slice(&tmd.padding);
    partition[tmd_offset + 0x140..][..0x40].copy_from_slice(&tmd.issuer);
    partition[tmd_offset + 0x180] = tmd.version;
    partition[tmd_offset + 0x181] = tmd.ca_crl_version;
    partition[tmd_offset + 0x182] = tmd.signer_crl_version;
    partition[tmd_offset + 0x183] = if tmd.is_v_wii { 1 } else { 0 };
    BE::write_u64(&mut partition[tmd_offset + 0x184..], tmd.system_version);
    partition[tmd_offset + 0x18c..][..8].copy_from_slice(&tmd.title_id);
    BE::write_u32(&mut partition[tmd_offset + 0x194..], tmd.title_type);
    BE::write_u16(&mut partition[tmd_offset + 0x198..], tmd.group_id);
    partition[tmd_offset + 0x19a..][..0x3e].copy_from_slice(&tmd.fake_sign);
    BE::write_u32(&mut partition[tmd_offset + 0x1d8..], tmd.access_rights);
    BE::write_u16(&mut partition[tmd_offset + 0x1dc..], tmd.title_version);
    BE::write_u16(
        &mut partition[tmd_offset + 0x1de..],
        tmd.contents.len() as u16,
    );
    BE::write_u16(&mut partition[tmd_offset + 0x1e0..], tmd.boot_index);
    partition[tmd_offset + 0x1e2..][..2].copy_from_slice(&tmd.padding3);
    for (i, content) in tmd.contents.iter().enumerate() {
        BE::write_u32(
            &mut partition[tmd_offset + 0x1E4 + i * 36..],
            content.content_id,
        );
        BE::write_u16(&mut partition[tmd_offset + 0x1E8 + i * 36..], content.index);
        BE::write_u16(
            &mut partition[tmd_offset + 0x1EA + i * 36..],
            content.content_type,
        );
        BE::write_u64(&mut partition[tmd_offset + 0x1EC + i * 36..], content.size);
        partition[tmd_offset + 0x1F4 + i * 36..][..consts::WII_HASH_SIZE]
            .copy_from_slice(&content.hash);
    }

    0x1E4 + 36 * tmd.contents.len()
}

#[derive(Debug, Clone, Copy)]
pub struct WiiPartition {
    pub part_type: u32,
    pub part_offset: u64,
    pub header: PartHeader,
}

#[derive(Debug, Clone, Default)]
pub struct WiiPartitions {
    pub data_idx: usize,
    pub part_info: PartInfo,
    pub partitions: Vec<WiiPartition>,
}

pub fn decrypt_title_key(tik: &Ticket) -> AesKey {
    #[cfg(feature = "log")]
    debug!("decrypting title key");
    let mut buf: AesKey = Default::default();
    let key = &COMMON_KEY[tik.common_key_index as usize];
    let mut iv: AesKey = Default::default();
    iv[0..tik.title_id.len()].copy_from_slice(&tik.title_id);
    let cipher = Aes128Cbc::new_from_slices(&key[..], &iv[..]).unwrap();
    let mut block = [0 as u8; 256];
    block[0..tik.title_key.len()].copy_from_slice(&tik.title_key);

    buf.copy_from_slice(&cipher.decrypt(&mut block).unwrap()[..tik.title_key.len()]);
    buf
}

fn decrypt_partition_inplace<'a>(
    buf: &'a mut [u8],
    part: &WiiPartition,
) -> Result<(), WiiCryptoError> {
    #[cfg(feature = "log")]
    debug!("decrypting partition inplace");
    let part_key = decrypt_title_key(&part.header.ticket);
    let sector_count = (part.header.data_size / 0x8000) as usize;
    let (_, data_slice) = buf.split_at_mut((part.part_offset + part.header.data_offset) as usize);
    let mut data_pool: Vec<&mut [u8]> = data_slice.chunks_mut(consts::WII_SECTOR_SIZE).collect();
    data_pool.par_iter_mut().for_each(|data| {
        let mut iv = [0 as u8; consts::WII_KEY_SIZE];
        iv[..consts::WII_KEY_SIZE]
            .copy_from_slice(&data[consts::WII_SECTOR_IV_OFF..][..consts::WII_KEY_SIZE]);
        // aes_decrypt_inplace(&mut data[..consts::WII_SECTOR_HASH_SIZE], &[0 as u8; consts::WII_KEY_SIZE], &part_key).unwrap();
        aes_decrypt_inplace(
            &mut data[consts::WII_SECTOR_HASH_SIZE..][..consts::WII_SECTOR_DATA_SIZE],
            &iv,
            &part_key,
        )
        .unwrap();
    });
    for i in 0..sector_count {
        let mut data = [0u8; consts::WII_SECTOR_DATA_SIZE];
        let data_offset = (part.part_offset + part.header.data_offset) as usize;
        data.copy_from_slice(
            &buf[data_offset + i * consts::WII_SECTOR_SIZE + consts::WII_SECTOR_HASH_SIZE
                ..data_offset + (i + 1) * consts::WII_SECTOR_SIZE],
        );
        buf[data_offset + i * consts::WII_SECTOR_DATA_SIZE..][..consts::WII_SECTOR_DATA_SIZE]
            .copy_from_slice(&data);
    }
    // println!("");
    // println!("title code: {}", String::from_utf8_lossy(&buf[part.part_offset + part.header.data_offset..][..0x06]));
    // println!("title code of the partition: {}", String::from_utf8_lossy(&buf[part.part_offset + part.header.data_offset..][0x20..0x46]));
    return Ok(());
}

pub fn parse_disc(buf: &mut [u8]) -> Result<Option<WiiPartitions>, WiiCryptoError> {
    let header = disc_get_header(buf);
    if header.wii_magic != 0x5D1C9EA3 {
        return Err(WiiCryptoError::NotWiiDisc {
            magic: header.wii_magic,
        });
    }
    // if header.disable_disc_encrypt == 1 || header.disable_hash_verif == 1 {
    //     return Ok(None);
    // } else {
    let part_info = disc_get_part_info(buf);
    let mut ret_vec: Vec<WiiPartition> = Vec::new();
    let mut ret: Option<WiiPartitions> = None;
    let mut data_idx: Option<usize> = None;
    for (i, entry) in part_info.entries.iter().enumerate() {
        let part = WiiPartition {
            part_offset: entry.offset,
            part_type: entry.part_type,
            header: PartHeader::try_from(&buf[entry.offset as usize..][..0x2C0])?,
        };
        // println!("part{}: part type={}, part offset={:08X}; tmd size={}, tmd offset={:08X}; cert size={}, cert offset={:08X}; h3 offset={:08X}; data size={}, data offset={:08X}", i, part.part_type, part.part_offset, part.header.tmd_size, part.header.tmd_offset, part.header.cert_size, part.header.cert_offset, part.header.h3_offset, part.header.data_size, part.header.data_offset);
        if part.part_type == 0 && data_idx.is_none() {
            data_idx = Some(i);
            decrypt_partition_inplace(buf, &part)?;
        }
        ret_vec.push(part);
    }
    if ret_vec.len() > 0 {
        if let Some(data_idx) = data_idx {
            ret = Some(WiiPartitions {
                data_idx,
                part_info,
                partitions: ret_vec,
            });
        }
    }
    return Ok(ret);
    // }
}

pub fn finalize_iso(
    patched_partition: &[u8],
    original_iso: &mut [u8],
) -> Result<()> {
    // We use the original iso as a buffer to work on inplace.
    let mut part_opt: Option<WiiPartition> = None;

    let part_info = disc_get_part_info(&original_iso[..]);
    for entry in part_info.entries.iter() {
        let part = WiiPartition {
            part_offset: entry.offset,
            part_type: entry.part_type,
            header: PartHeader::try_from(&original_iso[entry.offset as usize..][..0x2C0])?,
        };
        if entry.part_type == 0 && part_opt.is_none() {
            part_opt = Some(part);
        }
    }
    let mut part_info = part_info;
    part_info.entries.retain(|&e| e.part_type == 0);
    disc_set_part_info(&mut original_iso[..], &part_info);

    if let Some(part) = part_opt {
        let part_data_offset = part.part_offset + part.header.data_offset;
        let _n_sectors = (part.header.data_size / 0x8000) as usize;

        // reset partition to 0
        for i in part_data_offset..part_data_offset + part.header.data_size {
            original_iso[i as usize] = 0;
        }

        // Put the data in place
        for i in 0..(patched_partition.len() / consts::WII_SECTOR_DATA_SIZE) {
            original_iso[part_data_offset as usize
                + i * consts::WII_SECTOR_SIZE
                + consts::WII_SECTOR_HASH_SIZE..][..consts::WII_SECTOR_DATA_SIZE]
                .copy_from_slice(
                    &patched_partition[i * consts::WII_SECTOR_DATA_SIZE..]
                        [..consts::WII_SECTOR_DATA_SIZE],
                );
        }

        hash_partition(
            &mut original_iso[part.part_offset as usize..]
                [..(part.header.data_offset + part.header.data_size) as usize],
        );
        // original_iso[0x60] = 1u8;

        // set partition data size
        let part_header = part.header;

        // encrypt everything
        let part_key = decrypt_title_key(&part_header.ticket);

        // let mut data_pool: Vec<&mut [u8]> = Vec::with_capacity(n_sectors);
        let (_, data_slice) =
            original_iso.split_at_mut((part.part_offset + part.header.data_offset) as usize);
        let mut data_pool: Vec<&mut [u8]> =
            data_slice.chunks_mut(consts::WII_SECTOR_SIZE).collect();
        // for _ in 0..n_sectors {
        //     let (section, new_data_slice) = data_slice.split_at_mut(consts::WII_SECTOR_SIZE);
        //     data_slice = new_data_slice;
        //     data_pool.push(section);
        // }
        data_pool.par_iter_mut().for_each(|data| {
            let mut iv = [0 as u8; consts::WII_KEY_SIZE];
            aes_encrypt_inplace(
                &mut data[..consts::WII_SECTOR_HASH_SIZE],
                &iv,
                &part_key,
                consts::WII_SECTOR_HASH_SIZE,
            )
            .expect("Could not encrypt hash sector");
            iv[..consts::WII_KEY_SIZE]
                .copy_from_slice(&data[consts::WII_SECTOR_IV_OFF..][..consts::WII_KEY_SIZE]);
            aes_encrypt_inplace(
                &mut data[consts::WII_SECTOR_HASH_SIZE..][..consts::WII_SECTOR_DATA_SIZE],
                &iv,
                &part_key,
                consts::WII_SECTOR_DATA_SIZE,
            )
            .expect("Could not encrypt data sector");
        });
    }

    Ok(())
}

fn hash_partition(partition: &mut [u8]) {
    let header = PartHeader::try_from(&partition[..0x2C0]).expect("Invalid partition header.");
    let n_sectors = (header.data_size / 0x8000) as usize;
    let n_clusters = n_sectors / 8;
    let n_groups = n_clusters / 8;
    // println!("part offset: {:#010X}; sectors: {}; clusters: {}; groups: {}", header.data_offset, n_sectors, n_clusters, n_groups);

    // h0
    // println!("h0");
    let mut data_pool: Vec<&mut [u8]> = Vec::with_capacity(n_sectors);
    let (_, mut data_slice) = partition.split_at_mut(header.data_offset as usize);
    for _ in 0..n_sectors {
        let (section, new_data_slice) = data_slice.split_at_mut(consts::WII_SECTOR_SIZE);
        data_slice = new_data_slice;
        data_pool.push(section);
    }
    data_pool.par_iter_mut().for_each(|data| {
        let mut hash = [0u8; 0x400];
        for j in 0..31 {
            Sha1::from(&data[..]);
            hash[j * 20..(j + 1) * 20].copy_from_slice(
                &Sha1::from(&data[consts::WII_SECTOR_HASH_SIZE + j * 0x400..][..0x400])
                    .digest()
                    .bytes()[..],
            );
        }
        data[..0x400].copy_from_slice(&hash);
    });
    // h1
    // println!("h1");
    let mut data_pool: Vec<&mut [u8]> = Vec::with_capacity(n_clusters);
    let (_, mut data_slice) = partition.split_at_mut(header.data_offset as usize);
    for _ in 0..n_clusters {
        let (section, new_data_slice) = data_slice.split_at_mut(consts::WII_SECTOR_SIZE * 8);
        data_slice = new_data_slice;
        data_pool.push(section);
    }
    data_pool.par_iter_mut().for_each(|data| {
        let mut hash = [0u8; 0x0a0];
        for j in 0..8 {
            hash[j * 20..(j + 1) * 20]
                .copy_from_slice(&Sha1::from(&data[j * 0x8000..][..0x26c]).digest().bytes()[..]);
        }
        for j in 0..8 {
            data[j * 0x8000 + 0x280..][..0xa0].copy_from_slice(&hash);
        }
    });
    // h2
    // println!("h2");
    let mut data_pool: Vec<&mut [u8]> = Vec::with_capacity(n_groups);
    let (_, mut data_slice) = partition.split_at_mut(header.data_offset as usize);
    for _ in 0..n_groups {
        let (section, new_data_slice) = data_slice.split_at_mut(consts::WII_SECTOR_SIZE * 64);
        data_slice = new_data_slice;
        data_pool.push(section);
    }
    data_pool.par_iter_mut().for_each(|data| {
        let mut hash = [0u8; 0x0a0];
        for j in 0..8 {
            hash[j * 20..(j + 1) * 20].copy_from_slice(
                &Sha1::from(&data[j * 8 * 0x8000 + 0x280..][..0xa0])
                    .digest()
                    .bytes()[..],
            );
        }
        for j in 0..64 {
            data[j * 0x8000 + 0x340..][..0xa0].copy_from_slice(&hash);
        }
    });
    // h3
    // println!("h3");
    let h3_offset = header.h3_offset as usize;
    // zero the h3 table
    partition[h3_offset..][..0x18000].copy_from_slice(&[0u8; 0x18000]);
    // divide and conquer
    let mut data_pool: Vec<(&mut [u8], &mut [u8])> = Vec::with_capacity(n_groups);
    let (h3, mut data_slice) = partition.split_at_mut(header.data_offset as usize);
    let (_, mut h3) = h3.split_at_mut(h3_offset);
    for _ in 0..n_groups {
        let (section, new_data_slice) = data_slice.split_at_mut(consts::WII_SECTOR_SIZE * 64);
        let (hash_section, new_h3) = h3.split_at_mut(20);
        data_slice = new_data_slice;
        h3 = new_h3;
        data_pool.push((hash_section, section));
    }
    data_pool.par_iter_mut().for_each(|(hash, sector)| {
        hash[..20].copy_from_slice(&Sha1::from(&sector[0x340..][..0xa0]).digest().bytes()[..]);
    });
    // h4 / TMD
    let mut tmd = partition_get_tmd(partition, header.tmd_offset as usize);
    let mut tmd_size = 0x1e4 + 36 * tmd.contents.len();
    if tmd.contents.len() > 0 {
        let content = &mut tmd.contents[0];
        content.hash.copy_from_slice(
            &Sha1::from(&partition[h3_offset..][..0x18000])
                .digest()
                .bytes()[..],
        );
        tmd_size = partition_set_tmd(partition, header.tmd_offset as usize, &tmd);
    }
    tmd_fake_sign(&mut partition[header.tmd_offset as usize..][..tmd_size]);
    ticket_fake_sign(&mut partition[..Ticket::BLOCK_SIZE]);
    // println!("hashing done.");
}

pub fn tmd_fake_sign(tmd_buf: &mut [u8]) {
    let mut tmd = partition_get_tmd(tmd_buf, 0);
    tmd.signature.copy_from_slice(&[0u8; 0x100]);
    tmd.padding.copy_from_slice(&[0u8; 60]);
    tmd.fake_sign.copy_from_slice(&[0u8; 0x3e]);
    let tmd_size = partition_set_tmd(tmd_buf, 0, &tmd);
    // start brute force
    // println!("TMD fake singing; starting brute force...");
    let mut val = 0u32;
    let mut hash = [0u8; consts::WII_HASH_SIZE];
    loop {
        BE::write_u32(&mut tmd_buf[0x19a..][..4], val);
        hash.copy_from_slice(
            &Sha1::from(&tmd_buf[0x140..][..tmd_size - 0x140])
                .digest()
                .bytes()[..],
        );
        if hash[0] == 0 {
            break;
        }

        if val == std::u32::MAX {
            break;
        }
        val += 1;
    }
    // println!("TMD fake signing status: hash[0]={}, attempt(s)={}", hash[0], val);
}

pub fn ticket_fake_sign(tik_buf: &mut [u8]) {
    let mut tik = Ticket::try_from(&tik_buf[..Ticket::BLOCK_SIZE]).unwrap();
    tik.sig.copy_from_slice(&[0u8; 0x100]);
    tik.sig_padding.copy_from_slice(&[0u8; 0x3c]);
    tik.fake_sign.copy_from_slice(&[0u8; 0x58]);
    tik_buf[..Ticket::BLOCK_SIZE]
        .copy_from_slice(&<[u8; Ticket::BLOCK_SIZE]>::try_from(&tik).unwrap());
    // start brute force
    // println!("Ticket fake singing; starting brute force...");
    let mut val = 0u32;
    let mut hash = [0u8; consts::WII_HASH_SIZE];
    loop {
        BE::write_u32(&mut tik_buf[0x248..][..4], val);
        hash.copy_from_slice(
            &Sha1::from(&tik_buf[0x140..][..Ticket::BLOCK_SIZE - 0x140])
                .digest()
                .bytes()[..],
        );
        if hash[0] == 0 {
            break;
        }

        if val == std::u32::MAX {
            break;
        }
        val += 1;
    }
    // println!("Ticket fake signing status: hash[0]={}, attempt(s)={}", hash[0], val);
}

// Disc Data classes

#[derive(Debug, Clone)]
pub struct WiiDisc {
    disc_header: WiiDiscHeader,
    partitions: WiiPartitions,
}

// Readers

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
    if ret_vec.len() > 0 {
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
    return Err(WiiCryptoError::NoGamePartition.into());
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
        reader.read(&mut buf).await?;
        this.disc_info.disc_header = disc_get_header(&mut buf);
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
            cursor: self.cursor.clone(),
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
                    + (start_blk_idx * consts::WII_SECTOR_SIZE as usize) as u64;
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
                    let mut iv = [0 as u8; consts::WII_KEY_SIZE];
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
                        &[0 as u8; consts::WII_KEY_SIZE],
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
                        (vstart + read_size as u64) - block_pos as u64,
                    ) as usize;
                    buf[buf_write_start..buf_write_end].copy_from_slice(
                        &block[consts::WII_SECTOR_HASH_SIZE..][block_read_start..block_read_end],
                    );
                }
                *this.cursor = *this.cursor + buf.len() as u64;
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
    Wii(#[pin] WiiDiscReader<Pin<Box<R>>>),
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
            Ok(Self::Wii(WiiDiscReader::try_parse(reader).await?))
        } else {
            Err(eyre::eyre!("Not a game disc"))
        }
    }

    pub fn is_wii(&self) -> bool {
        match self {
            DiscReader::Gamecube(_) => false,
            DiscReader::Wii(_) => true,
        }
    }
}
