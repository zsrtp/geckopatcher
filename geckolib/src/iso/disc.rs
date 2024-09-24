use std::error::Error;
use std::io::SeekFrom;
use std::mem::offset_of;
use std::pin::Pin;

use async_std::io::prelude::SeekExt;
use async_std::io::ReadExt;
use num::Unsigned;

use crate::crypto::aes_decrypt_inplace;
use crate::crypto::AesKey;
use crate::crypto::WiiCryptoError;
use crate::crypto::COMMON_KEY;
use crate::crypto::{consts, Unpackable};
use crate::declare_tryfrom;
use async_std::io::{Read as AsyncRead, Seek as AsyncSeek};
use byteorder::{ByteOrder, BE};
use sha1_smol::Sha1;
use std::convert::TryFrom;

#[derive(Debug)]
pub enum DiscError {
    NoGamePartition,
    EncryptionError(WiiCryptoError),
    Io(std::io::Error),
}

impl Error for DiscError {}

impl std::fmt::Display for DiscError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DiscError::NoGamePartition => write!(f, "There is no game parition in this disc"),
            DiscError::EncryptionError(e) => write!(f, "Encryption error: {}", e),
            DiscError::Io(e) => write!(f, "IO error: {}", e),
        }
    }
}

impl From<WiiCryptoError> for DiscError {
    fn from(e: WiiCryptoError) -> Self {
        DiscError::EncryptionError(e)
    }
}

impl From<std::io::Error> for DiscError {
    fn from(e: std::io::Error) -> Self {
        DiscError::Io(e)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum DiscType {
    Gamecube = 0,
    Wii,
}

#[derive(Copy, Clone, Debug)]
pub struct WiiDiscHeader {
    pub disc_id: u8,
    pub game_code: [u8; 2],
    pub region_code: u8,
    pub maker_code: [u8; 2],
    pub disc_number: u8,
    pub disc_version: u8,
    pub audio_streaming: bool,
    pub streaming_buffer_size: u8,
    pub unk1: [u8; 14],
    pub wii_magic: u32,
    pub gc_magic: u32,
    pub game_title: [u8; 64],
    pub disable_hash_verif: bool,
    pub disable_disc_encrypt: bool,
    pub padding: [u8; 0x39e],
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
            game_title: [0; 64],
            disable_hash_verif: Default::default(),
            disable_disc_encrypt: Default::default(),
            padding: [0; 0x39e],
        }
    }
}

impl Unpackable for WiiDiscHeader {
    const BLOCK_SIZE: usize = 0x400;
}

pub fn disc_get_header(raw: &[u8]) -> WiiDiscHeader {
    let mut unk1 = [0_u8; 14];
    let mut game_title = [0u8; 64];
    let mut padding = [0u8; 0x39e];
    unk1.copy_from_slice(&raw[0xA..0x18]);
    game_title.copy_from_slice(&raw[0x20..0x60]);
    padding.copy_from_slice(&raw[0x62..0x400]);
    WiiDiscHeader {
        disc_id: raw[0],
        game_code: [raw[1], raw[2]],
        region_code: raw[3],
        maker_code: [raw[4], raw[5]],
        disc_number: raw[6],
        disc_version: raw[7],
        audio_streaming: raw[8] != 0,
        streaming_buffer_size: raw[9],
        unk1,
        wii_magic: BE::read_u32(&raw[0x18..0x1C]),
        gc_magic: BE::read_u32(&raw[0x1C..0x20]),
        game_title,
        disable_hash_verif: raw[0x60] != 0,
        disable_disc_encrypt: raw[0x61] != 0,
        padding,
    }
}

pub fn disc_set_header(buffer: &mut [u8], dh: &WiiDiscHeader) {
    buffer[0x00] = dh.disc_id;
    buffer[0x01] = dh.game_code[0];
    buffer[0x02] = dh.game_code[1];
    buffer[0x03] = dh.region_code;
    buffer[0x04] = dh.maker_code[0];
    buffer[0x05] = dh.maker_code[1];
    buffer[0x06] = dh.disc_number;
    buffer[0x07] = dh.disc_version;
    buffer[0x08] = dh.audio_streaming as u8;
    buffer[0x09] = dh.streaming_buffer_size;
    buffer[0x0A..0x18].copy_from_slice(&dh.unk1[..]);
    BE::write_u32(&mut buffer[0x18..], dh.wii_magic);
    BE::write_u32(&mut buffer[0x1C..], dh.gc_magic);
    buffer[0x20..0x60].copy_from_slice(&dh.game_title[..]);
    buffer[0x60] = dh.disable_hash_verif as u8;
    buffer[0x61] = dh.disable_disc_encrypt as u8;
    buffer[0x62..0x400].copy_from_slice(&dh.padding);
}

#[derive(Debug, Clone, Copy, Default)]
pub struct WiiDiscRegionAgeRating {
    jp: u8,
    us: u8,
    unknown1: u8,
    de: u8,
    pegi: u8,
    fi: u8,
    pt: u8,
    gb: u8,
    au: u8,
    kr: u8,
}

#[derive(Debug, Clone, Copy, Default)]
pub enum WiiDiscRegions {
    #[default]
    NTSCJ,
    NTSCU,
    PAL,
    KOR,
}

impl From<u32> for WiiDiscRegions {
    fn from(r: u32) -> Self {
        match r {
            0 => WiiDiscRegions::NTSCJ,
            1 => WiiDiscRegions::NTSCU,
            2 => WiiDiscRegions::PAL,
            3 => WiiDiscRegions::KOR,
            _ => WiiDiscRegions::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct WiiDiscRegion {
    pub region: WiiDiscRegions,
    pub age_rating: WiiDiscRegionAgeRating,
}

impl Unpackable for WiiDiscRegion {
    const BLOCK_SIZE: usize = 0x20;
}

impl WiiDiscRegion {
    pub fn parse(raw: &[u8]) -> Self {
        Self {
            region: BE::read_u32(&raw[..4]).into(),
            age_rating: WiiDiscRegionAgeRating {
                jp: raw[0x10],
                us: raw[0x11],
                unknown1: raw[0x12],
                de: raw[0x13],
                pegi: raw[0x14],
                fi: raw[0x15],
                pt: raw[0x16],
                gb: raw[0x17],
                au: raw[0x18],
                kr: raw[0x19],
            },
        }
    }

    pub fn compose_into(&self, buf: &mut [u8]) {
        buf[..4].copy_from_slice((self.region as u32).to_be_bytes().as_ref());
        buf[0x10] = self.age_rating.jp;
        buf[0x11] = self.age_rating.us;
        buf[0x12] = self.age_rating.unknown1;
        buf[0x13] = self.age_rating.de;
        buf[0x14] = self.age_rating.pegi;
        buf[0x15] = self.age_rating.fi;
        buf[0x16] = self.age_rating.pt;
        buf[0x17] = self.age_rating.gb;
        buf[0x18] = self.age_rating.au;
        buf[0x19] = self.age_rating.kr;
    }
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

pub async fn disc_get_part_info_async<R: AsyncRead + AsyncSeek>(
    reader: &mut Pin<&mut R>,
) -> Result<PartInfo, DiscError> {
    crate::debug!("Parsing partition info (async)");
    let mut entries: Vec<PartInfoEntry> = Vec::new();
    let mut buf: [u8; 8] = [0u8; 8];
    reader
        .seek(SeekFrom::Start(consts::WII_PARTITION_INFO_OFFSET))
        .await?;
    reader.read(&mut buf).await?;
    let n_part = BE::read_u32(&buf[..]) as u64;
    let part_info_offset = (BE::read_u32(&buf[4..]) as u64) << 2;
    crate::debug!(
        "Found {:} entries, partition info at offset 0x{:08X}",
        n_part,
        part_info_offset
    );
    for i in 0..n_part {
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
        entries,
    })
}

pub fn disc_set_part_info(buffer: &mut [u8], pi: &PartInfo) {
    BE::write_u32(&mut buffer[0x40000..], pi.entries.len() as u32);
    BE::write_u32(&mut buffer[0x40004..], (pi.offset >> 2) as u32);
    for (i, entry) in pi.entries.iter().enumerate() {
        BE::write_u32(
            &mut buffer[(pi.offset + (8 * i as u64)) as usize..],
            (entry.offset >> 2) as u32,
        );
        BE::write_u32(
            &mut buffer[(pi.offset + (8 * i as u64) + 4) as usize..],
            entry.part_type,
        );
    }
}

#[derive(Copy, Clone, Debug, Default, Eq, PartialEq, PartialOrd, Ord)]
pub enum PartitionType {
    Data = 0,
    Update = 1,
    ChannelInstaller = 2,
    #[default]
    Unknown = -1,
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
pub struct SignatureRSA2048 {
    pub sig_type: u32,
    pub sig: [u8; 0x100],
    pub sig_padding: [u8; 0x3C],
    pub sig_issuer: [u8; 0x40],
}

#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct Ticket {
    pub sig: SignatureRSA2048,
    pub server_public_key: [u8; 0x3C],
    pub version: u8,
    pub ca_crl_version: u8,
    pub signer_crl_version: u8,
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
assert_eq_size!(Ticket, [u8; <Ticket as Unpackable>::BLOCK_SIZE]);
declare_tryfrom!(Ticket);

impl Unpackable for Ticket {
    const BLOCK_SIZE: usize = 0x2A4;
}

impl Ticket {
    pub fn fake_sign(&mut self) -> Result<(), eyre::Report> {
        let mut tik_buf = [0u8; <Ticket as Unpackable>::BLOCK_SIZE];
        self.sig.sig.fill(0);
        self.sig.sig_padding.fill(0);
        self.fake_sign.fill(0);
        tik_buf[..<Ticket as Unpackable>::BLOCK_SIZE].copy_from_slice(&<[u8; <Ticket as Unpackable>::BLOCK_SIZE]>::from(&*self));
        // start brute force
        crate::trace!("Ticket fake signing; starting brute force...");
        let mut val = 0u32;
        let mut hash_0;
        let mut sha1 = Sha1::new();
        loop {
            BE::write_u32(&mut tik_buf[offset_of!(Self, time_limit)/* 0x248 */..][..size_of_val(&self.time_limit)], val);
            sha1.reset();
            sha1.update(&tik_buf[0x140..]);
            hash_0 = sha1.digest().bytes()[0];
            if hash_0 == 0 {
                break;
            }

            if val == u32::MAX {
                return Err(eyre::eyre!("Could not re-sign the partition ticket"));
            }
            val += 1;
        }
        self.time_limit = val;
        crate::debug!(
            "Ticket ({}) fake signing status: hash[0]={}, attempt(s)={}",
            if self.version == 1 { "v1" } else { "v0" },
            hash_0,
            self.time_limit
        );
        Ok(())
    }
}

impl From<&[u8; <Ticket as Unpackable>::BLOCK_SIZE]> for Ticket {
    fn from(buf: &[u8; <Ticket as Unpackable>::BLOCK_SIZE]) -> Self {
        let mut sig = [0_u8; 0x100];
        let mut sig_padding = [0_u8; 0x3C];
        let mut sig_issuer = [0_u8; 0x40];
        let mut server_public_key = [0_u8; 60];
        let mut title_key = [0_u8; 0x10];
        let mut ticket_id = [0_u8; 0x08];
        let mut title_id = [0_u8; 0x08];
        let mut unk4 = [0_u8; 0x09];
        let mut unk5 = [0_u8; 0x50];
        let mut fake_sign = [0_u8; 0x58];
        sig.copy_from_slice(&buf[0x4..0x104]);
        sig_padding.copy_from_slice(&buf[0x104..0x140]);
        sig_issuer.copy_from_slice(&buf[0x140..0x180]);
        server_public_key.copy_from_slice(&buf[0x180..0x1BC]);
        title_key.copy_from_slice(&buf[0x1BF..0x1CF]);
        ticket_id.copy_from_slice(&buf[0x1D0..0x1D8]);
        title_id.copy_from_slice(&buf[0x1DC..0x1E4]);
        unk4.copy_from_slice(&buf[0x1E8..0x1F1]);
        unk5.copy_from_slice(&buf[0x1F2..0x242]);
        fake_sign.copy_from_slice(&buf[0x24C..0x2A4]);
        Ticket {
            sig: SignatureRSA2048 {
                sig_type: BE::read_u32(&buf[0x00..]),
                sig,
                sig_padding,
                sig_issuer,
            },
            server_public_key,
            version: buf[0x1BC],
            ca_crl_version: buf[0x1BD],
            signer_crl_version: buf[0x1BE],
            title_key,
            unk2: buf[0x1CF],
            ticket_id,
            console_id: BE::read_u32(&buf[0x1D8..]),
            title_id,
            unk3: BE::read_u16(&buf[0x1E4..]),
            n_dlc: BE::read_u16(&buf[0x1E6..]),
            unk4,
            common_key_index: buf[0x1F1],
            unk5,
            padding2: BE::read_u16(&buf[0x242..]),
            enable_time_limit: BE::read_u32(&buf[0x244..]),
            time_limit: BE::read_u32(&buf[0x248..]),
            fake_sign,
        }
    }
}

impl From<&Ticket> for [u8; <Ticket as Unpackable>::BLOCK_SIZE] {
    fn from(t: &Ticket) -> Self {
        let mut buf = [0_u8; <Ticket as Unpackable>::BLOCK_SIZE];

        BE::write_u32(&mut buf[0x00..], t.sig.sig_type);
        buf[0x04..0x104].copy_from_slice(&t.sig.sig[..]);
        buf[0x104..0x140].copy_from_slice(&t.sig.sig_padding[..]);
        buf[0x140..0x180].copy_from_slice(&t.sig.sig_issuer[..]);
        buf[0x180..0x1BC].copy_from_slice(&t.server_public_key[..]);
        buf[0x1BC] = t.version;
        buf[0x1BD] = t.ca_crl_version;
        buf[0x1BE] = t.signer_crl_version;
        buf[0x1BF..0x1CF].copy_from_slice(&t.title_key[..]);
        buf[0x1CF] = t.unk2;
        buf[0x1D0..0x1D8].copy_from_slice(&t.ticket_id[..]);
        BE::write_u32(&mut buf[0x1D8..], t.console_id);
        buf[0x1DC..0x1E4].copy_from_slice(&t.title_id[..]);
        BE::write_u16(&mut buf[0x1E4..], t.unk3);
        BE::write_u16(&mut buf[0x1E6..], t.n_dlc);
        buf[0x1E8..0x1F1].copy_from_slice(&t.unk4[..]);
        buf[0x1F1] = t.common_key_index;
        buf[0x1F2..0x242].copy_from_slice(&t.unk5[..]);
        BE::write_u16(&mut buf[0x242..], t.padding2);
        BE::write_u32(&mut buf[0x244..], t.enable_time_limit);
        BE::write_u32(&mut buf[0x248..], t.time_limit);
        buf[0x24C..0x2A4].copy_from_slice(&t.fake_sign[..]);
        buf
    }
}

impl Default for Ticket {
    fn default() -> Self {
        Ticket::from(&[0_u8; <Ticket as Unpackable>::BLOCK_SIZE])
    }
}

#[derive(Debug, Clone, Copy, Default)]
#[repr(C)]
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
        let mut buf = [0_u8; PartHeader::BLOCK_SIZE];
        buf[..0x2A4].copy_from_slice(&<[u8; <Ticket as Unpackable>::BLOCK_SIZE]>::from(&ph.ticket));
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

impl TitleMetaData {
    pub fn get_size_n(n_content: u16) -> usize {
        0x1E4 + 36 * n_content as usize
    }

    pub fn get_size(&self) -> usize {
        0x1E4 + 36 * self.contents.len()
    }

    pub fn from_partition(partition: &[u8], tmd_offset: usize) -> TitleMetaData {
        crate::trace!("Parsing TitleMetadata");
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
        crate::trace!("TMD has {:} content entries", tmd.n_content);
        for i in 0..tmd.n_content as usize {
            let mut content = TMDContent {
                content_id: BE::read_u32(&partition[tmd_offset + 0x1E4 + i * 36..]),
                index: BE::read_u16(&partition[tmd_offset + 0x1E8 + i * 36..]),
                content_type: BE::read_u16(&partition[tmd_offset + 0x1EA + i * 36..]),
                size: BE::read_u64(&partition[tmd_offset + 0x1EC + i * 36..]),
                hash: [0u8; consts::WII_HASH_SIZE],
            };
            content.hash.copy_from_slice(
                &partition[tmd_offset + 0x1F4 + i * 36..][..consts::WII_HASH_SIZE],
            );
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
        crate::trace!("TMD: {:?}", tmd);
        tmd
    }

    pub fn set_partition(partition: &mut [u8], tmd_offset: usize, tmd: &TitleMetaData) {
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
    }

    pub fn fake_sign(&mut self) -> Result<(), eyre::Report> {
        let mut tmd_buf = vec![0u8; self.get_size()];
        self.signature.fill(0);
        self.padding.fill(0);
        self.fake_sign.fill(0);
        let tmd_size = self.get_size();
        TitleMetaData::set_partition(&mut tmd_buf, 0, self);
        // start brute force
        crate::debug!("TMD fake signing; starting brute force...");
        let mut val = 0u32;
        let mut hash_0;
        loop {
            BE::write_u32(&mut tmd_buf[0x19a..][..4], val);
            hash_0 = Sha1::from(&tmd_buf[0x140..][..tmd_size - 0x140])
                .digest()
                .bytes()[0];
            if hash_0 == 0 {
                break;
            }

            if val == u32::MAX {
                return Err(eyre::eyre!("Could not re-sign the partition's TMD"));
            }
            val += 1;
        }
        *self = TitleMetaData::from_partition(&tmd_buf, 0);
        // BE::write_u32(&mut self.fake_sign, val);
        crate::debug!(
            "TMD fake signing status: hash[0]={}, attempt(s)={}",
            hash_0,
            val
        );
        Ok(())
    }
}

impl Default for TitleMetaData {
    fn default() -> Self {
        TitleMetaData::from_partition(&[0u8; 0x1E4], 0)
    }
}

#[derive(Debug, Clone)]
pub struct WiiPartition {
    pub part_type: PartitionType,
    pub part_offset: u64,
    pub header: PartHeader,
    pub tmd: TitleMetaData,
    pub cert: Box<[u8]>,
}

#[derive(Debug, Clone, Default)]
pub struct WiiPartitions {
    pub data_idx: usize,
    pub part_info: PartInfo,
    pub partitions: Vec<WiiPartition>,
}

pub fn decrypt_title_key(tik: &Ticket) -> AesKey {
    crate::trace!("decrypting title key");
    let mut buf: AesKey = Default::default();
    let key: AesKey = COMMON_KEY[tik.common_key_index as usize].into();
    let mut iv: AesKey = Default::default();
    iv[0..tik.title_id.len()].copy_from_slice(&tik.title_id);
    let mut block = [0u8; 256];
    block[0..tik.title_key.len()].copy_from_slice(&tik.title_key);
    aes_decrypt_inplace(&mut block, iv, key);
    buf.copy_from_slice(&block[..tik.title_key.len()]);
    buf
}

// Disc Data classes

#[derive(Debug, Clone)]
pub struct WiiDisc {
    pub disc_header: WiiDiscHeader,
    pub disc_region: WiiDiscRegion,
    pub partitions: WiiPartitions,
}

// Disc Data/Hash Structs

#[derive(Debug, Copy, Clone)]
#[repr(C)]
pub struct WiiSectorHash {
    pub(crate) h0: [[u8; consts::WII_HASH_SIZE]; consts::WII_SECTOR_DATA_HASH_COUNT as usize],
    _h0_padding: [u8; 20],
    pub(crate) h1: [[u8; consts::WII_HASH_SIZE]; 8],
    _h1_padding: [u8; 32],
    pub(crate) h2: [[u8; consts::WII_HASH_SIZE]; 8],
    _h2_padding: [u8; 32],
}
const_assert_eq!(
    std::mem::size_of::<WiiSectorHash>(),
    consts::WII_SECTOR_HASH_SIZE
);

impl Default for WiiSectorHash {
    fn default() -> Self {
        WiiSectorHash {
            h0: [[0u8; consts::WII_HASH_SIZE]; consts::WII_SECTOR_DATA_HASH_COUNT as usize],
            _h0_padding: [0u8; 20],
            h1: [[0u8; consts::WII_HASH_SIZE]; 8],
            _h1_padding: [0u8; 32],
            h2: [[0u8; consts::WII_HASH_SIZE]; 8],
            _h2_padding: [0u8; 32],
        }
    }
}

impl WiiSectorHash {
    pub fn as_array(self) -> [u8; consts::WII_SECTOR_HASH_SIZE] {
        unsafe { std::mem::transmute(self) }
    }

    pub fn as_array_ref(&self) -> &[u8; consts::WII_SECTOR_HASH_SIZE] {
        unsafe { std::mem::transmute(self) }
    }

    pub fn as_array_mut(&mut self) -> &mut [u8; consts::WII_SECTOR_HASH_SIZE] {
        unsafe { std::mem::transmute(self) }
    }

    pub fn get_h0_ref(&self) -> &[u8; consts::WII_HASH_SIZE * consts::WII_SECTOR_DATA_HASH_COUNT as usize] {
        unsafe { std::mem::transmute(&self.h0) }
    }

    pub fn get_h1_ref(&self) -> &[u8; consts::WII_HASH_SIZE * 8] {
        unsafe { std::mem::transmute(&self.h1) }
    }

    pub fn get_h2_ref(&self) -> &[u8; consts::WII_HASH_SIZE * 8] {
        unsafe { std::mem::transmute(&self.h2) }
    }

    pub fn get_h0_mut(
        &mut self,
    ) -> &mut [u8; consts::WII_HASH_SIZE * consts::WII_SECTOR_DATA_HASH_COUNT as usize] {
        unsafe { std::mem::transmute(&mut self.h0) }
    }

    pub fn get_h1_mut(&mut self) -> &mut [u8; consts::WII_HASH_SIZE * 8] {
        unsafe { std::mem::transmute(&mut self.h1) }
    }

    pub fn get_h2_mut(&mut self) -> &mut [u8; consts::WII_HASH_SIZE * 8] {
        unsafe { std::mem::transmute(&mut self.h2) }
    }
}

#[derive(Debug, Copy, Clone)]
#[repr(C)]
pub struct WiiSector {
    pub(crate) hash: WiiSectorHash,
    pub(crate) data: [u8; consts::WII_SECTOR_DATA_SIZE as usize],
}
const_assert_eq!(std::mem::size_of::<WiiSector>(), consts::WII_SECTOR_SIZE);

impl Default for WiiSector {
    fn default() -> Self {
        WiiSector {
            hash: WiiSectorHash::default(),
            data: [0u8; consts::WII_SECTOR_DATA_SIZE as usize],
        }
    }
}

#[derive(Debug, Clone)]
#[repr(C)]
pub struct WiiSubGroup {
    pub(crate) sectors: Box<[WiiSector; 8]>,
}

impl Default for WiiSubGroup {
    fn default() -> Self {
        WiiSubGroup {
            sectors: Box::new([WiiSector::default(); 8]),
        }
    }
}

#[derive(Debug, Clone)]
#[repr(C)]
pub struct WiiGroup {
    pub(crate) sub_groups: Box<[WiiSubGroup; 8]>,
}

impl Default for WiiGroup {
    fn default() -> Self {
        WiiGroup {
            sub_groups: Box::new(core::array::from_fn(|_| WiiSubGroup::default())),
        }
    }
}

impl WiiGroup {
    pub fn reset(&mut self) {
        for sub_group in self.sub_groups.iter_mut() {
            for sector in sub_group.sectors.iter_mut() {
                sector.hash = WiiSectorHash::default();
                sector.data.fill(0);
            }
        }
    }

    pub fn to_vec(&self) -> Vec<u8> {
        self.as_sectors_ref()
            .iter()
            .flat_map(|sector| sector.hash.as_array_ref().iter().chain(sector.data.iter()))
            .copied()
            .collect()
    }

    pub fn as_sectors(self) -> Vec<WiiSector> {
        self.sub_groups
            .iter()
            .flat_map(|sub_group| sub_group.sectors.iter())
            .copied()
            .collect()
    }

    pub fn as_sectors_ref(&self) -> Vec<&WiiSector> {
        self.sub_groups
            .iter()
            .flat_map(|sub_group| sub_group.sectors.iter())
            .collect()
    }

    pub fn as_sectors_mut(&mut self) -> Vec<&mut WiiSector> {
        self.sub_groups
            .iter_mut()
            .flat_map(|sub_group| sub_group.sectors.iter_mut())
            .collect()
    }
}

// Numeric helper functions for address handling

/// Aligns the `addr` up so that every bits before the `bit`-th bit are 0.
pub fn align_addr<T>(addr: T, bit: usize) -> T
where
    T: std::ops::Add<Output = T>
        + std::ops::Sub<Output = T>
        + std::ops::Shl<usize, Output = T>
        + std::ops::BitAnd<Output = T>
        + std::ops::Not<Output = T>
        + From<u8>,
{
    ((addr - T::from(1)) & !((T::from(1) << bit) - T::from(1))) + (T::from(1) << bit)
}

pub fn to_raw_addr<T>(size: T) -> T
where
    T: std::ops::Div<Output = T>
        + std::ops::Mul<Output = T>
        + std::ops::Rem<Output = T>
        + std::ops::Add<Output = T>
        + From<u16>
        + Copy,
{
    (size / T::from(consts::WII_SECTOR_DATA_SIZE as u16)) * T::from(consts::WII_SECTOR_SIZE as u16)
        + (size % T::from(consts::WII_SECTOR_DATA_SIZE as u16))
        + T::from(consts::WII_SECTOR_HASH_SIZE as u16)
}

pub fn to_virtual_addr<T>(size: T) -> T
where
    T: std::ops::Div<Output = T>
        + std::ops::Mul<Output = T>
        + std::ops::Rem<Output = T>
        + std::ops::Add<Output = T>
        + num::traits::SaturatingSub<Output = T>
        + From<u16>
        + Copy
        + Unsigned,
{
    (size / T::from(consts::WII_SECTOR_SIZE as u16)) * T::from(consts::WII_SECTOR_DATA_SIZE as u16)
        + T::saturating_sub(
            &(size % T::from(consts::WII_SECTOR_SIZE as u16)),
            &T::from(consts::WII_SECTOR_HASH_SIZE as u16),
        )
}

#[cfg(test)]
mod test {
    use crate::iso::disc::{to_raw_addr, to_virtual_addr};

    #[test]
    fn works_normally() {
        assert_eq!(to_virtual_addr(0x400u16), 0x0);
        assert_eq!(to_virtual_addr(0x400u32), 0x0);
        assert_eq!(to_virtual_addr(0x400u64), 0x0);
        assert_eq!(to_virtual_addr(0x400u128), 0x0);
        assert_eq!(to_virtual_addr(0x400usize), 0x0);

        assert_eq!(to_virtual_addr(0x600u16), 0x200);
        assert_eq!(to_virtual_addr(0x600u32), 0x200);
        assert_eq!(to_virtual_addr(0x600u64), 0x200);
        assert_eq!(to_virtual_addr(0x600u128), 0x200);
        assert_eq!(to_virtual_addr(0x600usize), 0x200);

        assert_eq!(to_virtual_addr(0x7fffu16), 0x7BFF);
        assert_eq!(to_virtual_addr(0x7fffu32), 0x7BFF);
        assert_eq!(to_virtual_addr(0x7fffu64), 0x7BFF);
        assert_eq!(to_virtual_addr(0x7fffu128), 0x7BFF);
        assert_eq!(to_virtual_addr(0x7fffusize), 0x7BFF);

        assert_eq!(to_virtual_addr(0x8401u16), 0x7C01);
        assert_eq!(to_virtual_addr(0x8401u32), 0x7C01);
        assert_eq!(to_virtual_addr(0x8401u64), 0x7C01);
        assert_eq!(to_virtual_addr(0x8401u128), 0x7C01);
        assert_eq!(to_virtual_addr(0x8401usize), 0x7C01);
    }

    #[test]
    fn hash_address_goes_to_start_of_data() {
        assert_eq!(to_virtual_addr(0x0u16), 0x0);
        assert_eq!(to_virtual_addr(0x0u32), 0x0);
        assert_eq!(to_virtual_addr(0x0u64), 0x0);
        assert_eq!(to_virtual_addr(0x0u128), 0x0);
        assert_eq!(to_virtual_addr(0x0usize), 0x0);

        assert_eq!(to_virtual_addr(0x1u16), 0x0);
        assert_eq!(to_virtual_addr(0x1u32), 0x0);
        assert_eq!(to_virtual_addr(0x1u64), 0x0);
        assert_eq!(to_virtual_addr(0x1u128), 0x0);
        assert_eq!(to_virtual_addr(0x1usize), 0x0);

        assert_eq!(to_virtual_addr(0x3FFu16), 0x0);
        assert_eq!(to_virtual_addr(0x3FFu32), 0x0);
        assert_eq!(to_virtual_addr(0x3FFu64), 0x0);
        assert_eq!(to_virtual_addr(0x3FFu128), 0x0);
        assert_eq!(to_virtual_addr(0x3FFusize), 0x0);

        assert_eq!(to_virtual_addr(0x8000u16), 0x7C00);
        assert_eq!(to_virtual_addr(0x8000u32), 0x7C00);
        assert_eq!(to_virtual_addr(0x8000u64), 0x7C00);
        assert_eq!(to_virtual_addr(0x8000u128), 0x7C00);
        assert_eq!(to_virtual_addr(0x8000usize), 0x7C00);

        assert_eq!(to_virtual_addr(0x83ffu16), 0x7C00);
        assert_eq!(to_virtual_addr(0x83ffu32), 0x7C00);
        assert_eq!(to_virtual_addr(0x83ffu64), 0x7C00);
        assert_eq!(to_virtual_addr(0x83ffu128), 0x7C00);
        assert_eq!(to_virtual_addr(0x83ffusize), 0x7C00);
    }

    #[test]
    fn decrypted_address_idempotent() {
        assert_eq!(to_virtual_addr(to_raw_addr(0x1u16)), 0x1);
        assert_eq!(to_virtual_addr(to_raw_addr(0x1u32)), 0x1);
        assert_eq!(to_virtual_addr(to_raw_addr(0x1u64)), 0x1);
        assert_eq!(to_virtual_addr(to_raw_addr(0x1u128)), 0x1);
        assert_eq!(to_virtual_addr(to_raw_addr(0x1usize)), 0x1);

        assert_eq!(to_virtual_addr(to_raw_addr(0x0u16)), 0x0);
        assert_eq!(to_virtual_addr(to_raw_addr(0x0u32)), 0x0);
        assert_eq!(to_virtual_addr(to_raw_addr(0x0u64)), 0x0);
        assert_eq!(to_virtual_addr(to_raw_addr(0x0u128)), 0x0);
        assert_eq!(to_virtual_addr(to_raw_addr(0x0usize)), 0x0);

        assert_eq!(to_virtual_addr(to_raw_addr(0x7C00u16)), 0x7C00);
        assert_eq!(to_virtual_addr(to_raw_addr(0x7C00u32)), 0x7C00);
        assert_eq!(to_virtual_addr(to_raw_addr(0x7C00u64)), 0x7C00);
        assert_eq!(to_virtual_addr(to_raw_addr(0x7C00u128)), 0x7C00);
        assert_eq!(to_virtual_addr(to_raw_addr(0x7C00usize)), 0x7C00);
    }
}
