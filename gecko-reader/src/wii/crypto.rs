use std::{collections::HashMap, ffi::CString, mem::offset_of, ops::Shr};

use byteorder::ByteOrder;

use crate::{
    iosc::{
        self, CERT_ECC_SIZE, CERT_HEADER_SIZE, CERT_RSA2048_ECC_SIZE, CERT_RSA2048_RSA2048_SIZE,
        CERT_RSA4096_RSA2048_SIZE, ECC_PUBLIC_KEY_SIZE, IOSUid, PublicKeyType,
        RSA2048_PUBLIC_KEY_SIZE, SIGNATURE_ECC_SIZE, SIGNATURE_RSA2048_SIZE,
        SIGNATURE_RSA4096_SIZE, Signature, SignatureRSA2048, SignatureType,
        hle::{ConsoleType, DefaultHandle, Iosc, IoscError},
    },
    wii,
};

pub fn aes_decrypt_inplace<K: ::core::ops::Deref<Target = [u8; wii::consts::WII_KEY_SIZE]>>(
    data: &mut [u8],
    iv: K,
    key: K,
) {
    use aes::cipher::{BlockDecryptMut, KeyIvInit};
    let mut cipher = wii::keys::Aes128CbcDec::new_from_slices(key.deref(), iv.deref()).unwrap();
    data.chunks_exact_mut(wii::consts::WII_KEY_SIZE)
        .for_each(|chunk| cipher.decrypt_block_mut(aes::Block::from_mut_slice(chunk)));
}

pub fn aes_encrypt_inplace<K: ::core::ops::Deref<Target = [u8; wii::consts::WII_KEY_SIZE]>>(
    data: &mut [u8],
    iv: K,
    key: K,
) {
    use aes::cipher::{BlockEncryptMut, KeyIvInit};
    let mut cipher = wii::keys::Aes128CbcEnc::new_from_slices(key.deref(), iv.deref()).unwrap();
    data.chunks_exact_mut(wii::consts::WII_KEY_SIZE)
        .for_each(|chunk| cipher.encrypt_block_mut(aes::Block::from_mut_slice(chunk)));
}

// The next section is based off of Dolphin-emu's Core/Core/IOS/ES/Formats.h

pub mod titles {
    pub const BOOT2: u64 = 0x0000000100000001;
    pub const SYSTEM_MENU: u64 = 0x0000000100000002;
    pub const SHOP: u64 = 0x0001000248414241;
    pub const KOREAN_SHOP: u64 = 0x000100024841424b;
    pub const FORECAST_CHANNEL_NTSC_U: u64 = 0x0001000248414645;
    pub const FORECAST_CHANNEL_NTSC_J: u64 = 0x000100024841464a;
    pub const FORECAST_CHANNEL_PAL: u64 = 0x0001000248414650;
    pub const NINTENDO_CHANNEL_NTSC_U: u64 = 0x0001000148415445;
    pub const NINTENDO_CHANNEL_NTSC_J: u64 = 0x000100014841544a;
    pub const NINTENDO_CHANNEL_PAL: u64 = 0x0001000148415450;
    pub const NEWS_CHANNEL_NTSC_U: u64 = 0x0001000248414745;
    pub const NEWS_CHANNEL_NTSC_J: u64 = 0x000100024841474a;
    pub const NEWS_CHANNEL_PAL: u64 = 0x0001000248414750;
    pub const EVERYBODY_VOTES_CHANNEL_NTSC_U: u64 = 0x0001000148414a45;
    pub const EVERYBODY_VOTES_CHANNEL_NTSC_J: u64 = 0x0001000148414a4a;
    pub const EVERYBODY_VOTES_CHANNEL_PAL: u64 = 0x0001000148414a50;
    pub const REGION_SELECT_CHANNEL_NTSC_U: u64 = 0x0001000848414c45;
    pub const REGION_SELECT_CHANNEL_NTSC_J: u64 = 0x0001000848414c4a;
    pub const REGION_SELECT_CHANNEL_PAL: u64 = 0x0001000848414c50;
    const fn ios(major_version: u32) -> u64 {
        0x0000000100000000 | (major_version as u64)
    }

    // IOS used by the latest System Menu (4.3). Corresponds to IOS80.
    pub const SYSTEM_MENU_IOS: u64 = ios(80);
    pub const BC: u64 = ios(0x100);
    pub const MIOS: u64 = ios(0x101);
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u32)]
pub enum TitleType {
    #[default]
    System = 0x00000001,
    Game = 0x00010000,
    Channel = 0x00010001,
    SystemChannel = 0x00010002,
    GameWithChannel = 0x00010004,
    DLC = 0x00010005,
    HiddenChannel = 0x00010008,
}

impl TitleType {
    pub fn is_title_type(title_id: u64, title_type: TitleType) -> bool {
        title_id.shr(32) as u32 == title_type as u32
    }

    pub fn is_disc_title(title_id: u64) -> bool {
        TitleType::is_title_type(title_id, TitleType::Game)
            || TitleType::is_title_type(title_id, TitleType::GameWithChannel)
    }

    pub fn is_channel(title_id: u64) -> bool {
        title_id == titles::SYSTEM_MENU
            || TitleType::is_title_type(title_id, TitleType::Channel)
            || TitleType::is_title_type(title_id, TitleType::SystemChannel)
            || TitleType::is_title_type(title_id, TitleType::GameWithChannel)
            || TitleType::is_title_type(title_id, TitleType::HiddenChannel)
    }
}

#[derive(Debug, Clone, Copy, Default)]
#[repr(u32)]
pub enum TitleFlags {
    // All official titles have this flag set.
    #[default]
    TitleTypeDefault = 0x1,
    // Unknown.
    TitleType0x4 = 0x4,
    // Used for DLC titles.
    TitleTypeData = 0x08,
    // Unknown.
    TitleType0x10 = 0x10,
    // Appears to be used for WFS titles.
    TitleTypeWfsMaybe = 0x20,
    // Unknown.
    TitleTypeCT = 0x40,
}

#[derive(Debug, Clone, Copy, Default)]
#[repr(C)]
#[repr(packed)]
pub struct TMDHeader {
    pub signature: SignatureRSA2048,
    pub tmd_version: u8,
    pub ca_crl_version: u8,
    pub signer_crl_version: u8,
    // This is usually an always 0 padding byte, which is set to 1 on vWii TMDs
    pub is_vwii: u8,
    pub ios_id: u64,
    pub title_id: u64,
    pub title_flags: u32,
    pub group_id: u16,
    pub zero: u16,
    pub region: u16,
    pub ratings: [u8; 16],
    pub reserved: [u8; 12],
    pub ipc_mask: [u8; 12],
    pub reserved2: [u8; 18],
    pub access_rights: u32,
    pub title_version: u16,
    pub num_contents: u16,
    pub boot_index: u16,
    pub fill2: u16,
}
pub const TMD_HEADER_SIZE: usize = 0x1e4;
crate::static_assert_eq_size!(TMDHeader, TMD_HEADER_SIZE);

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
#[repr(C)]
#[repr(packed)]
pub struct Content {
    pub id: u32,
    pub index: u16,
    pub content_type: u16,
    pub size: u64,
    pub sha1: [u8; 20],
}
pub const TMD_CONTENT_SIZE: usize = 36;
crate::static_assert_eq_size!(Content, TMD_CONTENT_SIZE);

impl Content {
    pub const fn is_shared(&self) -> bool {
        (self.content_type & 0x8000) != 0
    }

    pub const fn is_optional(&self) -> bool {
        (self.content_type & 0x4000) != 0
    }
}

#[derive(Debug, Clone, Copy, Default)]
#[repr(C)]
#[repr(packed)]
pub struct TimeLimit {
    pub enabled: u32,
    pub seconds: u32,
}
const _: () = assert!(::core::mem::size_of::<TimeLimit>() == 0x8);

#[derive(Debug, Clone, Copy)]
#[repr(C)]
#[repr(packed)]
pub struct TicketView {
    pub version: u8,
    _padding1: [u8; 3],
    pub ticket_id: u64,
    pub device_id: u32,
    pub title_id: u64,
    pub access_mask: u16,
    _padding2: u16,
    pub permitted_title_id: u32,
    pub permitted_title_mask: u32,
    pub title_export_allowed: u8,
    pub common_key_index: u8,
    pub unknown2: [u8; 0x30],
    pub content_access_permissions: [u8; 0x40],
    _padding3: u16,
    pub time_limits: [TimeLimit; 8],
}
const TICKET_VIEW_SIZE: usize = 0xd8;
crate::static_assert_eq_size!(TicketView, TICKET_VIEW_SIZE);
crate::static_assert_eq_offset!(TicketView, time_limits, 152);

impl Default for TicketView {
    fn default() -> Self {
        Self {
            version: Default::default(),
            ticket_id: Default::default(),
            device_id: Default::default(),
            title_id: Default::default(),
            access_mask: Default::default(),
            permitted_title_id: Default::default(),
            permitted_title_mask: Default::default(),
            title_export_allowed: Default::default(),
            common_key_index: Default::default(),
            unknown2: [0; _],
            content_access_permissions: [0; _],
            time_limits: Default::default(),
            _padding1: Default::default(),
            _padding2: Default::default(),
            _padding3: Default::default(),
        }
    }
}

#[derive(Debug, Clone, Copy)]
#[repr(C)]
#[repr(packed)]
pub struct Ticket {
    pub signature: SignatureRSA2048,
    pub server_public_key: iosc::Signature,
    pub version: u8,
    pub ca_crl_version: u8,
    pub signer_crl_version: u8,
    pub title_key: [u8; 0x10],
    _padding: u8,
    pub ticket_id: u64,
    pub device_id: u32,
    pub title_id: u64,
    pub access_mask: u16,
    pub ticket_version: u16,
    pub permitted_title_id: u32,
    pub permitted_title_mask: u32,
    pub title_export_allowed: u8,
    pub common_key_index: u8,
    pub unk: [u8; 0x30],
    pub content_access_permissions: [u8; 0x40],
    _padding2: u16,
    pub time_limits: [TimeLimit; 8],
}
crate::static_assert_eq_size!(Ticket, 0x2a4);
crate::static_assert_eq_offset!(Ticket, ticket_id, 0x1D0);
crate::static_assert_eq_offset!(Ticket, title_id, 0x1DC);
crate::static_assert_eq_offset!(Ticket, title_key, 0x1BF);
crate::static_assert_eq_offset!(Ticket, server_public_key, 0x180);
crate::static_assert_eq_offset!(Ticket, common_key_index, 0x1f1);

#[derive(Debug, Clone, Copy)]
#[repr(C)]
#[repr(packed)]
pub struct V1TicketHeader {
    pub version: u16,
    pub header_size: u16,
    pub v1_ticket_size: u32,
    pub section_header_table_offset: u32,
    pub number_of_section_headers: u16,
    pub section_header_size: u16,
    pub flags: u32,
}
crate::static_assert_eq_size!(V1TicketHeader, 0x14);
crate::static_assert_eq_offset!(V1TicketHeader, v1_ticket_size, 0x4);

pub const MAX_TMD_SIZE: u32 = 0x49e4;

pub trait SignedReader {
    fn get_bytes(&self) -> &[u8];
    fn set_bytes(&mut self, bytes: &[u8]);

    /// Get the SHA1 hash for this signed blob (starting at the issuer).
    fn get_sha1(&self) -> [u8; 20] {
        let mut hasher = sha1_smol::Sha1::new();
        let skip = get_issuer_offset(self.get_signature_type());
        hasher.update(&self.get_bytes()[skip..(self.get_bytes().len() - skip)]);
        hasher.digest().bytes()
    }

    /// Only checks whether the signature data could be parsed. The signature is not verified.
    fn is_signature_valid(&self) -> bool {
        // Too small for certificate type
        if self.get_bytes().len() < std::mem::size_of::<SignatureType>() {
            return false;
        }
        let signature_size = self.get_signature_size();
        if signature_size.is_none_or(|size| self.get_bytes().len() < size) {
            return false;
        }
        return true;
    }

    fn get_signature_type(&self) -> Option<SignatureType> {
        match byteorder::BE::read_u32(&self.get_bytes()[..4]) {
            0x00010000 => Some(SignatureType::RSA4096),
            0x00010001 => Some(SignatureType::RSA2048),
            0x00010002 => Some(SignatureType::ECC),
            _ => None,
        }
    }

    fn get_signature_data(&self) -> Option<Vec<u8>> {
        Some(detail_get_signature_data(
            self.get_signature_type()?,
            self.get_bytes(),
        ))
    }

    fn get_signature_size(&self) -> Option<usize> {
        match self.get_signature_type()? {
            SignatureType::RSA4096 => Some(0x280),
            SignatureType::RSA2048 => Some(0x180),
            SignatureType::ECC => Some(0xc0),
        }
    }

    /// Returns the whole uissuer chain.
    /// Example: Root-CA00000001 if the blob was signed by CA00000001, which is signed by the Root.
    fn get_issuer(&self) -> Option<CString> {
        Some(detail_get_issuer(
            self.get_signature_type(),
            self.get_bytes(),
        ))
    }
}

#[derive(Debug, Clone, Default)]
pub struct SignedBlobReader {
    bytes: Vec<u8>,
}

fn get_issuer_offset(signature_type: Option<SignatureType>) -> usize {
    match signature_type {
        Some(SignatureType::RSA4096) => 0x240,
        Some(SignatureType::RSA2048) => 0x140,
        Some(SignatureType::ECC) => 0x80,
        None => 0x0,
    }
}

fn detail_get_signature_size(signature_type: SignatureType) -> usize {
    match signature_type {
        SignatureType::RSA4096 => 0x200,
        SignatureType::RSA2048 => 0x100,
        SignatureType::ECC => 0x3c,
    }
}

fn detail_get_signature_data(signature_type: SignatureType, data: &[u8]) -> Vec<u8> {
    const SIGNATURE_OFFSET: usize = 0x4;
    let signature_size = detail_get_signature_size(signature_type);
    let mut buf = vec![0; signature_size];
    buf.copy_from_slice(&data[SIGNATURE_OFFSET..][..signature_size]);
    buf
}

fn detail_get_issuer(signature_type: Option<SignatureType>, data: &[u8]) -> CString {
    let issuer_offset = get_issuer_offset(signature_type);
    crate::utils::c_string_from_slice(&data[issuer_offset..]).unwrap_or_default()
}

impl SignedReader for SignedBlobReader {
    fn get_bytes(&self) -> &[u8] {
        &self.bytes
    }

    fn set_bytes(&mut self, bytes: &[u8]) {
        self.bytes.clear();
        self.bytes.extend_from_slice(bytes);
    }
}

pub fn is_valid_tmd_size(size: usize) -> bool {
    size <= MAX_TMD_SIZE as usize
}

#[derive(Debug, Clone, Default)]
pub struct TMDReader {
    bytes: Vec<u8>,
}

impl SignedReader for TMDReader {
    fn get_bytes(&self) -> &[u8] {
        &self.bytes
    }

    fn set_bytes(&mut self, bytes: &[u8]) {
        self.bytes.clear();
        self.bytes.extend_from_slice(bytes);
    }
}

pub const CONTENT_VIEW_SIZE: usize = 0x10;

pub const TITLE_ID_OFFSET: usize = 0x18c;
pub const TITLE_VERSION_OFFSET: usize = 0x1dc;
pub const GROUP_ID_OFFSET: usize = 0x198;

impl TMDReader {
    pub fn is_valid(&self) -> bool {
        if !self.is_signature_valid() {
            return false;
        }

        if self.bytes.len() < TMD_HEADER_SIZE {
            // TMD is too small to contain all its base fields.
            return false;
        }

        if self.bytes.len() < TMD_HEADER_SIZE + self.get_num_contents() as usize * TMD_CONTENT_SIZE
        {
            // TMD is too small to contain all its expected content entries.
            return false;
        }

        return true;
    }

    pub fn get_num_contents(&self) -> u16 {
        const TMD_NUM_CONTENT_OFFSET: usize = 0x1de;
        byteorder::BE::read_u16(&self.bytes[TMD_NUM_CONTENT_OFFSET..])
    }

    pub fn get_raw_view(&self) -> Vec<u8> {
        const START_OFFSET: usize = 0x180;
        const END_OFFSET: usize = 0x1d8;
        // Copy the base fields (from tmd_version [inclusive] to access_rights [exclusive]).
        let mut view = self.bytes[START_OFFSET..END_OFFSET].to_vec();
        const NUM_CONTENTS_OFFSET_END: usize = 0x1e0;
        // Copy both title_version and num_contents after the base fields.
        view.extend_from_slice(&self.bytes[TITLE_VERSION_OFFSET..NUM_CONTENTS_OFFSET_END]);
        // Content views (same as Content, but without the hash)
        for i in 0..self.get_num_contents() {
            let content_start = TMD_HEADER_SIZE + (i as usize) * TMD_CONTENT_SIZE;
            view.extend_from_slice(&self.bytes[content_start..][..CONTENT_VIEW_SIZE]);
        }
        view
    }

    pub fn get_boot_index(&self) -> u16 {
        const BOOT_INDEX_OFFSET: usize = 0x1e0;
        byteorder::BE::read_u16(&self.bytes[BOOT_INDEX_OFFSET..])
    }

    pub fn get_ios_id(&self) -> u64 {
        const IOS_ID_OFFSET: usize = 0x184;
        byteorder::BE::read_u64(&self.bytes[IOS_ID_OFFSET..])
    }

    pub fn get_title_id(&self) -> u64 {
        byteorder::BE::read_u64(&self.bytes[TITLE_ID_OFFSET..])
    }

    pub fn get_title_flags(&self) -> u32 {
        const TITLE_FLAGS_OFFSET: usize = 0x194;
        byteorder::BE::read_u32(&self.bytes[TITLE_FLAGS_OFFSET..])
    }

    pub fn get_title_version(&self) -> u16 {
        byteorder::BE::read_u16(&self.bytes[TITLE_VERSION_OFFSET..])
    }

    pub fn get_group_id(&self) -> u16 {
        byteorder::BE::read_u16(&self.bytes[GROUP_ID_OFFSET..])
    }

    pub fn get_region(&self) -> wii::disc::Region {
        if !TitleType::is_channel(self.get_title_id()) {
            return wii::disc::Region::Unknown;
        }

        if self.get_title_id() == titles::SYSTEM_MENU {
            return wii::disc::Region::from(self.get_title_version() & 0xf);
        }

        const REGION_OFFSET: usize = 0x1a4;
        wii::disc::Region::from(byteorder::BE::read_u16(&self.bytes[REGION_OFFSET..]))
    }

    pub fn is_vwii(&self) -> bool {
        const IS_VWII_OFFSET: usize = 0x183;
        self.bytes[IS_VWII_OFFSET] != 0
    }

    pub fn get_game_id(&self) -> CString {
        let mut game_id = [b'0'; 6];
        game_id[..4].copy_from_slice(&self.bytes[TITLE_ID_OFFSET + 4..][..4]);
        game_id[4..6].copy_from_slice(&self.bytes[GROUP_ID_OFFSET..][..2]);
        crate::utils::c_string_from_slice(&game_id).unwrap_or_else(|_| {
            CString::new(format!("{:016x}", self.get_title_id())).unwrap_or_default()
        })
    }

    pub fn get_game_tbdid(&self) -> CString {
        let mut buf: [u8; 4] = [b'0'; 4];
        buf[..4].copy_from_slice(&self.bytes[TITLE_ID_OFFSET..][..4]);
        crate::utils::c_string_from_slice(&buf).unwrap_or_else(|_| {
            CString::new(format!("{:016x}", self.get_title_id())).unwrap_or_default()
        })
    }

    pub fn get_content(&self, index: u16) -> Option<Content> {
        if index >= self.get_num_contents() {
            return None;
        }

        let mut content = Content::default();
        content.id = byteorder::BE::read_u32(&self.bytes[..]);
        const CONTENT_INDEX_OFFSET: usize = 4;
        content.index = byteorder::BE::read_u16(&self.bytes[CONTENT_INDEX_OFFSET..]);
        const CONTENT_TYPE_OFFSET: usize = 6;
        content.content_type = byteorder::BE::read_u16(&self.bytes[CONTENT_TYPE_OFFSET..]);
        const CONTENT_SIZE_OFFSET: usize = 8;
        content.size = byteorder::BE::read_u64(&self.bytes[CONTENT_SIZE_OFFSET..]);
        const CONTENT_SHA1_OFFSET: usize = 0x10;
        content
            .sha1
            .copy_from_slice(&self.bytes[CONTENT_SHA1_OFFSET..][..20]);

        Some(content)
    }

    pub fn get_contents(&self) -> Vec<Content> {
        let mut contents: Vec<Content> = Vec::with_capacity(self.get_num_contents() as usize);
        for i in 0..self.get_num_contents() {
            if let Some(content) = self.get_content(i) {
                contents.push(content);
            }
        }
        contents
    }

    pub fn find_content_by_id(&self, id: u32) -> Option<Content> {
        for index in 0..self.get_num_contents() {
            if let Some(content) = self.get_content(index) {
                if content.id == id {
                    return Some(content);
                }
            } else {
                return None;
            }
        }
        return None;
    }
}

#[derive(Debug, Clone, Default)]
pub struct TicketReader {
    bytes: Vec<u8>,
}

impl SignedReader for TicketReader {
    fn get_bytes(&self) -> &[u8] {
        &self.bytes
    }

    fn set_bytes(&mut self, bytes: &[u8]) {
        self.bytes.clear();
        self.bytes.extend_from_slice(bytes);
    }
}

impl TicketReader {
    pub fn is_valid(&self) -> bool {
        if !self.is_signature_valid() || self.bytes.is_empty() {
            return false;
        }

        if self.is_v1_ticket() {
            return self.bytes.len() == self.get_ticket_size() as usize;
        }

        self.bytes.len() % size_of::<Ticket>() == 0
    }

    pub fn is_v1_ticket(&self) -> bool {
        // Version can only be 0 or 1
        self.get_version() == 1
    }

    pub fn get_number_of_tickets(&self) -> usize {
        if self.is_v1_ticket() {
            return 1;
        }

        self.bytes.len() / size_of::<Ticket>()
    }

    pub fn get_ticket_size(&self) -> u32 {
        if self.is_v1_ticket() {
            const V1_TICKET_SIZE_OFFSET: usize = size_of::<Ticket>() + offset_of!(V1TicketHeader, v1_ticket_size);
            return byteorder::BE::read_u32(&self.bytes[V1_TICKET_SIZE_OFFSET..])
                + size_of::<Ticket>() as u32;
        }

        size_of::<Ticket>() as u32
    }

    pub fn get_raw_ticket(&self, ticket_id_to_find: u64) -> Vec<u8> {
        for i in 0..self.get_number_of_tickets() {
            let ticket_begin = self.get_ticket_size() as usize * i;
            let ticket_id = byteorder::BE::read_u64(&self.bytes[ticket_begin + offset_of!(Ticket, ticket_id)..]);
            if ticket_id == ticket_id_to_find {
                return self.bytes[ticket_begin..][..self.get_ticket_size() as usize].to_vec();
            }
        }
        return Vec::new();
    }

    pub fn get_raw_ticket_view(&self, ticket_num: u32) -> Vec<u8> {
        let ticket_start = size_of::<Ticket>() * ticket_num as usize;
        let view_start = ticket_start + offset_of!(Ticket, ticket_id);

        let mut view = vec![0u8; ::core::mem::size_of::<u32>()];
        view[0] = self.get_version();

        view.extend_from_slice(
            &self.bytes[view_start..][..TICKET_VIEW_SIZE - ::core::mem::size_of::<u32>()],
        );

        view
    }

    pub fn get_version(&self) -> u8 {
        const TICKET_VERSION_OFFSET: usize = 0x1bc;
        self.bytes[TICKET_VERSION_OFFSET]
    }

    pub fn get_device_id(&self) -> u32 {
        const TICKET_DEVICE_ID_OFFSET: usize = 0x1d8;
        byteorder::BE::read_u32(&self.bytes[TICKET_DEVICE_ID_OFFSET..])
    }

    pub fn get_title_id(&self) -> u64 {
        const TICKET_TITLE_ID_OFFSET: usize = 0x1dc;
        byteorder::BE::read_u64(&self.bytes[TICKET_TITLE_ID_OFFSET..])
    }

    pub fn get_common_key_index(&self) -> u8 {
        self.bytes[offset_of!(Ticket, common_key_index)]
    }

    pub fn get_title_key(&self, iosc: Option<&Iosc>) -> [u8; wii::consts::WII_KEY_SIZE] {
        let iosc = if let Some(iosc) = iosc {
            iosc
        } else {
            &Iosc::new(self.get_console_type())
        };
        let mut iv = [0u8; 0x10];
        iv[..8].copy_from_slice(&self.bytes[offset_of!(Ticket, title_id)..][..8]);
        let mut index = self.get_common_key_index() as usize;
        if index as usize >= iosc::hle::COMMON_KEY_HANDLES.len() {
            // TODO Log a warning "Bad common key index for title {:016x}: {} -- using common key 0" with get_title_id, index
            index = 0;
        }
        let common_key_handle = iosc::hle::COMMON_KEY_HANDLES[index];
        let mut output = [0u8; _];
        output.copy_from_slice(&self.bytes[offset_of!(Ticket, title_key)..][..wii::consts::WII_KEY_SIZE]);
        let decrypted = iosc.decrypt(
            common_key_handle.into(),
            &iv,
            &self.bytes[offset_of!(Ticket, title_key)..][..wii::consts::WII_KEY_SIZE],
            IOSUid::PidEs as u32,
        );
        if let Ok(data) = decrypted {
            output.copy_from_slice(&data[..]);
        }
        output
    }

    pub fn get_console_type(&self) -> ConsoleType {
        match self.get_issuer() {
            Some(issuer) if issuer == c"Root-CA00000002-XS00000006" => ConsoleType::RVT,
            _ => ConsoleType::Retail,
        }
    }

    pub fn delete_ticket(&mut self, ticket_id_to_delete: u64) {
        let mut new_ticket = Vec::new();
        let num_tickets = self.get_number_of_tickets();
        for i in 0..num_tickets {
            let ticket_start = self.get_ticket_size() as usize * i;
            let ticket_id = byteorder::BE::read_u64(&self.bytes[ticket_start + offset_of!(Ticket, ticket_id)..]);
            if ticket_id != ticket_id_to_delete {
                new_ticket.extend_from_slice(
                    &self.bytes[ticket_start..][..self.get_ticket_size() as usize],
                );
            }
        }
        self.set_bytes(&new_ticket);
    }

    #[doc = "IOS uses IOSC to compute an AES key from the peer public key and the device's private ECC key, which is used to the decrypt the title key. The IV is the ticket ID (8 bytes), zero extended."]
    pub fn unpersonalise(&mut self, iosc: &mut Iosc) -> Result<(), IoscError> {
        let public_handle = iosc.create_object(
            iosc::hle::ObjectType::TypePublicKey,
            iosc::hle::ObjectSubType::ECC233,
            IOSUid::PidEs as u32,
        )?;
        iosc.import_public_key(
            public_handle,
            &self.bytes[offset_of!(Ticket, server_public_key)..][..size_of::<Signature>()],
            None,
            IOSUid::PidFs as u32,
        )?;
        let key_handle = iosc.create_object(
            iosc::hle::ObjectType::TypeSecretKey,
            iosc::hle::ObjectSubType::AES128,
            IOSUid::PidFs as u32,
        )?;
        iosc.compute_shared_key(
            key_handle,
            DefaultHandle::HandleConsoleKey.into(),
            public_handle,
            IOSUid::PidFs as u32,
        )?;
        let mut iv = [0u8; 0x10];
        iv[..8].copy_from_slice(&self.bytes[offset_of!(Ticket, ticket_id)..][..8]);
        let key = iosc.decrypt(
            key_handle,
            &iv,
            &self.bytes[offset_of!(Ticket, title_key)..][..0x10],
            IOSUid::PidFs as u32,
        )?;
        self.bytes[offset_of!(Ticket, title_key)..][..0x10].copy_from_slice(&key);
        Ok(())
    }

    pub fn overwrite_common_key_index(&mut self, index: u8) {
        self.bytes[offset_of!(Ticket, common_key_index)] = index;
    }
}

#[derive(Debug, Clone, Default)]
pub struct CertReader {
    bytes: Vec<u8>,
}

impl SignedReader for CertReader {
    fn get_bytes(&self) -> &[u8] {
        &self.bytes
    }

    fn set_bytes(&mut self, bytes: &[u8]) {
        self.bytes.clear();
        self.bytes.extend_from_slice(bytes);
    }
}

impl CertReader {
    pub fn parse(bytes: Vec<u8>) -> Option<Self> {
        let mut this = Self { bytes };

        if !this.is_signature_valid() {
            return None;
        }

        const TYPES: [(SignatureType, PublicKeyType, usize); 4] = [
            (
                SignatureType::RSA4096,
                PublicKeyType::RSA2048,
                CERT_RSA4096_RSA2048_SIZE,
            ),
            (
                SignatureType::RSA2048,
                PublicKeyType::RSA2048,
                CERT_RSA2048_RSA2048_SIZE,
            ),
            (
                SignatureType::RSA2048,
                PublicKeyType::ECC,
                CERT_RSA2048_ECC_SIZE,
            ),
            (SignatureType::ECC, PublicKeyType::ECC, CERT_ECC_SIZE),
        ];

        let type_ = TYPES.iter().find(|(sig_type, key_type, size)| {
            this.bytes.len() >= *size
                && this.get_signature_type().is_some_and(|s| s == *sig_type)
                && this.get_public_key_type().is_ok_and(|k| k == *key_type)
        });
        if let Some((_, _, size)) = type_ {
            this.bytes.resize(*size, 0);
            Some(this)
        } else {
            None
        }
    }

    pub fn get_id(&self) -> u32 {
        const CERT_HEADER_ID_OFFSET: usize = 0x44;
        let offset = self.get_signature_size().unwrap_or_default() + CERT_HEADER_ID_OFFSET;
        byteorder::BE::read_u32(&self.bytes[offset..])
    }

    pub fn get_name(&self) -> CString {
        const CERT_HEADER_NAME_OFFSET: usize = 0x4;
        const CERT_HEADER_NAME_SIZE: usize = 0x40;
        let name_slice = &self.bytes
            [self.get_signature_size().unwrap_or_default() + CERT_HEADER_NAME_OFFSET..]
            [..CERT_HEADER_NAME_SIZE];
        crate::utils::c_string_from_slice(name_slice).unwrap_or_default()
    }

    pub fn get_public_key_type(&self) -> Result<PublicKeyType, u32> {
        byteorder::BE::read_u32(&self.bytes[self.get_signature_size().unwrap_or_default()..])
            .try_into()
    }

    pub fn get_public_key(&self) -> Option<Vec<u8>> {
        match self.get_signature_type() {
            Some(SignatureType::RSA4096) => Some(
                self.bytes[SIGNATURE_RSA4096_SIZE + CERT_HEADER_SIZE..][..RSA2048_PUBLIC_KEY_SIZE]
                    .to_vec(),
            ),
            Some(SignatureType::RSA2048) => {
                let public_key_size = if let Ok(PublicKeyType::RSA2048) = self.get_public_key_type()
                {
                    RSA2048_PUBLIC_KEY_SIZE
                } else {
                    ECC_PUBLIC_KEY_SIZE
                };
                Some(
                    self.bytes[SIGNATURE_RSA2048_SIZE + CERT_HEADER_SIZE..][..public_key_size]
                        .to_vec(),
                )
            }
            Some(SignatureType::ECC) => Some(
                self.bytes[SIGNATURE_ECC_SIZE + CERT_HEADER_SIZE..][..ECC_PUBLIC_KEY_SIZE].to_vec(),
            ),
            None => None,
        }
    }
}

pub fn parse_cert_chain(mut chain: &[u8]) -> HashMap<CString, CertReader> {
    let mut certs = HashMap::new();
    loop {
        let cert = CertReader::parse(chain.to_vec());
        chain = if let Some(cert) = cert {
            let (_, chain) = chain.split_at(cert.bytes.len());
            certs.insert(cert.get_name(), cert);
            chain
        } else {
            return certs;
        };
    }
}

// From Core/Common/Crypto/bn.cpp

#[allow(non_snake_case)]
pub mod bn {
    pub fn bn_sub_modulus(a: &mut [u8], b: &[u8]) {
        let n = std::cmp::min(a.len(), b.len());
        let mut c = 0u8;
        for i in (0..n).rev() {
            let dig: u32 = b[i] as u32 + c as u32;
            c = if (a[i] as u32) < dig { 1 } else { 0 };
            a[i] = a[i].wrapping_sub(dig as u8);
        }
    }

    pub fn bn_add(d: &mut [u8], a: &[u8], b: &[u8], N: &[u8]) {
        use std::cmp::min;
        let n = min(min(d.len(), a.len()), min(b.len(), N.len()));

        let mut c = 0u8;
        for i in (0..n).rev() {
            let dig: u32 = a[i] as u32 + b[i] as u32 + c as u32;
            c = if dig >= 0x100 { 1 } else { 0 };
            d[i] = dig as u8;
        }

        if c != 0 {
            bn_sub_modulus(&mut d[..n], &N[..n]);
        }
        if d.as_ref() >= N {
            bn_sub_modulus(&mut d[..n], &N[..n]);
        }
    }

    pub fn bn_mul(d: &mut [u8], a: &[u8], b: &[u8], N: &[u8]) {
        use std::cmp::min;
        let n = min(min(d.len(), a.len()), min(b.len(), N.len()));
        d.fill(0);

        for i in 0..n {
            for mask in (0..8).map(|m| 0x80u8 >> m) {
                let d2 = d.to_vec();
                bn_add(&mut d[..n], &d2[..n], &d2[..n], &N[..n]);
                if (a[i] & mask) != 0 {
                    let d2 = d.to_vec();
                    bn_add(&mut d[..n], &d2[..n], &b[..n], &N[..n]);
                }
            }
        }
    }

    pub fn bn_exp(d: &mut [u8], a: &[u8], N: &[u8], e: &[u8]) {
        use std::cmp::min;
        let n = min(min(d.len(), a.len()), N.len());
        let mut t = [0u8; 0x200];
        d[..n].fill(0);
        d[n - 1] = 1;
        for i in 0..e.len() {
            for mask in (0..8).map(|m| 0x80u8 >> m) {
                bn_mul(&mut t[..n], &d[..n], &d[..n], &N[..n]);
                if (e[i] & mask) != 0 {
                    bn_mul(&mut d[..n], &t[..n], &a[..n], &N[..n]);
                } else {
                    d[..n].copy_from_slice(&t[..n]);
                }
            }
        }
    }

    pub fn bn_inv(d: &mut [u8], a: &[u8], N: &[u8]) {
        use std::cmp::min;
        let mut t = [0u8; 512];
        let mut s = [0u8; 512];
        let n = min(min(d.len(), a.len()), N.len());

        t[..n].copy_from_slice(&N[..n]);
        s[n - 1] = 2;
        bn_sub_modulus(&mut t[..n], &s[..n]);
        bn_exp(&mut d[..n], &a[..n], &N[..n], &t[..n]);
    }
}

// From Core/Common/Crypto/ec.cpp

pub mod ec {
    use rand::{Rng, SeedableRng};
    use rand_chacha::ChaCha20Rng;

    use crate::{
        iosc::{PublicKey, Signature},
        wii::crypto::bn::{bn_add, bn_inv, bn_mul, bn_sub_modulus},
    };

    const SQUARE: [u8; 0x10] = [
        0x00, 0x01, 0x04, 0x05, 0x10, 0x11, 0x14, 0x15, 0x40, 0x41, 0x44, 0x45, 0x50, 0x51, 0x54,
        0x55,
    ];

    #[derive(Debug, Clone, Copy, Default)]
    pub struct Elt {
        data: [u8; 30],
    }

    impl Elt {
        pub fn is_zero(&self) -> bool {
            self.data.iter().all(|b| *b == 0)
        }

        pub fn mul_x(&mut self) {
            let carry = self.data[0] & 1;
            let mut x = 0u8;
            for i in 0..(self.data.len() - 1) {
                let y = self.data[i + 1];
                self.data[i] = x ^ (y.unbounded_shr(7));
                x = y.unbounded_shl(1);
            }
            self.data[29] = x ^ carry;
            self.data[20] ^= carry.unbounded_shl(2);
        }

        pub fn square(&self) -> Self {
            let mut wide = [0u8; 60];
            for i in 0..self.data.len() {
                wide[2 * i] = SQUARE[(self.data[i].unbounded_shr(4)) as usize];
                wide[2 * i + 1] = SQUARE[(self.data[i] & 0xf) as usize];
            }
            for i in 0..self.data.len() {
                let x = wide[i];
                wide[i + 19] ^= x.unbounded_shr(7);
                wide[i + 20] ^= x.unbounded_shl(1);
                wide[i + 29] ^= x.unbounded_shr(1);
                wide[i + 30] ^= x.unbounded_shl(7);
            }

            let x = wide[30] & !1u8;
            wide[49] ^= x.unbounded_shr(7);
            wide[50] ^= x.unbounded_shl(1);
            wide[59] ^= x.unbounded_shr(1);
            wide[30] &= 1;

            let mut result = Elt { data: [0; _] };
            result.data.copy_from_slice(&wide[30..]);
            result
        }

        pub fn itoh_tsujii(&self, b: &Elt, j: usize) -> Elt {
            let mut t = self.clone();
            for _ in 0..j {
                t = t.square();
            }
            t * (*b)
        }

        pub fn inv(&self) -> Elt {
            let mut t = self.itoh_tsujii(&self, 1);
            let mut s = t.itoh_tsujii(&self, 1);
            t = s.itoh_tsujii(&s, 3);
            s = t.itoh_tsujii(&self, 1);
            t = s.itoh_tsujii(&s, 7);
            s = t.itoh_tsujii(&t, 14);
            t = s.itoh_tsujii(&self, 1);
            s = t.itoh_tsujii(&t, 29);
            t = s.itoh_tsujii(&s, 58);
            s = t.itoh_tsujii(&t, 116);
            s.square()
        }
    }

    impl std::ops::Add for Elt {
        type Output = Elt;

        fn add(self, rhs: Self) -> Self::Output {
            let mut d = Elt { data: [0; _] };
            for i in 0..30 {
                d.data[i] = self.data[i] ^ rhs.data[i];
            }
            d
        }
    }

    impl std::ops::Mul for Elt {
        type Output = Self;

        fn mul(self, rhs: Self) -> Self::Output {
            let mut d = Elt { data: [0; _] };
            let mut i = 0usize;
            let mut mask = 1u8;
            for _ in 0..233 {
                d.mul_x();

                if (self.data[i] & mask) != 0 {
                    d = d + rhs;
                }

                mask >>= 1;
                if mask == 0 {
                    mask = 0x80;
                    i += 1;
                }
            }
            d
        }
    }

    impl std::ops::Div for Elt {
        type Output = Self;

        fn div(self, rhs: Self) -> Self::Output {
            self * rhs.inv()
        }
    }

    pub const EC_B: [u8; 30] = [
        0x00, 0x66, 0x64, 0x7e, 0xde, 0x6c, 0x33, 0x2c, 0x7f, 0x8c, 0x09, 0x23, 0xbb, 0x58, 0x21,
        0x3b, 0x33, 0x3b, 0x20, 0xe9, 0xce, 0x42, 0x81, 0xfe, 0x11, 0x5f, 0x7d, 0x8f, 0x90, 0xad,
    ];

    pub const EC_N: [u8; 30] = [
        0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x13, 0xe9, 0x74, 0xe7, 0x2f, 0x8a, 0x69, 0x22, 0x03, 0x1d, 0x26, 0x03, 0xcf, 0xe0, 0xd7,
    ];

    pub const EC_G: Point = Point {
        data: [
            Elt {
                data: [
                    0x00, 0xfa, 0xc9, 0xdf, 0xcb, 0xac, 0x83, 0x13, 0xbb, 0x21, 0x39, 0xf1, 0xbb,
                    0x75, 0x5f, 0xef, 0x65, 0xbc, 0x39, 0x1f, 0x8b, 0x36, 0xf8, 0xf8, 0xeb, 0x73,
                    0x71, 0xfd, 0x55, 0x8b,
                ],
            },
            Elt {
                data: [
                    0x01, 0x00, 0x6a, 0x08, 0xa4, 0x19, 0x03, 0x35, 0x06, 0x78, 0xe5, 0x85, 0x28,
                    0xbe, 0xbf, 0x8a, 0x0b, 0xef, 0xf8, 0x67, 0xa7, 0xca, 0x36, 0x71, 0x6f, 0x7e,
                    0x01, 0xf8, 0x10, 0x52,
                ],
            },
        ],
    };

    #[derive(Clone, Copy)]
    pub union Point {
        data: [Elt; 2],
        array: [u8; 60],
    }

    impl std::fmt::Debug for Point {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            unsafe { std::fmt::Debug::fmt(&self.data, f) }
        }
    }

    impl Default for Point {
        fn default() -> Self {
            Self {
                data: Default::default(),
            }
        }
    }

    impl Point {
        pub fn new(x: Elt, y: Elt) -> Self {
            Self { data: [x, y] }
        }

        pub fn from_raw(array: [u8; 60]) -> Self {
            Self { array }
        }

        pub fn x(&self) -> &Elt {
            unsafe { &self.data[0] }
        }

        pub fn x_mut(&mut self) -> &mut Elt {
            unsafe { &mut self.data[0] }
        }

        pub fn y(&self) -> &Elt {
            unsafe { &self.data[1] }
        }

        pub fn y_mut(&mut self) -> &mut Elt {
            unsafe { &mut self.data[1] }
        }

        pub fn is_zero(&self) -> bool {
            self.x().is_zero() && self.y().is_zero()
        }

        pub fn get_data(&self) -> &[u8] {
            unsafe { &self.array }
        }

        pub fn get_data_mut(&mut self) -> &mut [u8] {
            unsafe { &mut self.array }
        }

        pub fn double(&self) -> Self {
            let mut r = Point::default();
            if self.x().is_zero() {
                return r;
            }

            let s = (*self.y()) / (*self.x()) + (*self.x());
            *r.x_mut() = s.square() + s;
            r.x_mut().data[29] ^= 1;
            *r.y_mut() = (s * (*r.x())) + (*r.x()) + self.x().square();

            r
        }
    }

    impl std::ops::Add for Point {
        type Output = Self;

        fn add(self, rhs: Self) -> Self::Output {
            if self.is_zero() {
                return rhs;
            }
            if rhs.is_zero() {
                return self;
            }

            let mut u = (*self.x()) + (*rhs.x());
            if u.is_zero() {
                u = (*self.y()) + (*rhs.y());
                if u.is_zero() {
                    return self.double();
                }
                return Point::default();
            }

            let s = ((*self.y()) + (*rhs.y())) / u;
            let mut t = s.square() + s + (*rhs.x());
            t.data[29] ^= 1;

            let rx = t + (*self.x());
            let ry = s * t + (*self.y()) + rx;
            Point::new(rx, ry)
        }
    }

    impl std::ops::Mul<[u8; 30]> for Point {
        type Output = Point;

        fn mul(self, rhs: [u8; 30]) -> Self::Output {
            let mut d = Point::default();
            for i in 0..30 {
                for mask in (0..8).map(|m| 0x80u8 >> m) {
                    d = d.double();
                    if (rhs[i] & mask) != 0 {
                        d = d + self;
                    }
                }
            }
            d
        }
    }

    pub fn sign(key: [u8; 30], hash: [u8; 20]) -> Signature {
        let mut e = [0u8; 30];
        e[10..].copy_from_slice(&hash[..20]);

        let mut m = [0u8; 30];
        let mut rng = ChaCha20Rng::from_os_rng();
        loop {
            m.copy_from_slice(&(&mut rng).random_iter().take(30).collect::<Vec<_>>());
            m[0] &= 1;
            if m < EC_N {
                break;
            }
        }

        let mut r = *(EC_G * m).x();
        if r.data >= EC_N {
            super::bn::bn_sub_modulus(&mut r.data, &EC_N);
        }

        //  S = m**-1*(e + Rk) (mod N)

        let mut kk = [0u8; 30];
        kk.copy_from_slice(&key[..30]);
        if kk >= EC_N {
            bn_sub_modulus(&mut kk, &EC_N);
        }
        let mut s = Elt::default();
        bn_mul(&mut s.data, &r.data, &kk, &EC_N);
        bn_add(&mut kk, &s.data, &e, &EC_N);
        let mut minv = [0u8; 30];
        bn_inv(&mut minv, &m, &EC_N);
        bn_mul(&mut s.data, &minv, &kk, &EC_N);

        let mut signature: Signature = [0; _];
        signature[..30].copy_from_slice(&r.data);
        signature[30..].copy_from_slice(&s.data);
        signature
    }

    #[allow(non_snake_case)]
    pub fn verify_signature(public_key: [u8; 60], signature: [u8; 60], hash: [u8; 20]) -> bool {
        let R = &signature[..30];
        let S = &signature[30..];
        let mut Sinv = [0u8; 30];

        bn_inv(&mut Sinv, &S, &EC_N);
        let mut e = [0u8; 30];
        e[10..].copy_from_slice(&hash[..20]);

        let mut w1 = [0u8; 30];
        let mut w2 = [0u8; 30];
        bn_mul(&mut w1, &e, &Sinv, &EC_N);
        bn_mul(&mut w2, &R, &Sinv, &EC_N);

        let r1 = EC_G * w1 + Point::from_raw(public_key) * w2;
        let mut rx = r1.x().data;
        if rx >= EC_N {
            bn_sub_modulus(&mut rx, &EC_N);
        }
        rx == R
    }

    pub fn priv_to_pub(key: [u8; 30]) -> PublicKey {
        let data = EC_G * key;
        let mut result: PublicKey = [0; _];
        unsafe { result.copy_from_slice(&data.array) };
        result
    }

    pub fn compute_shared_secret(private_key: [u8; 30], public_key: [u8; 60]) -> [u8; 60] {
        let mut shared_secret = [0u8; 60];
        let data = Point::from_raw(public_key) * private_key;
        unsafe { shared_secret.copy_from_slice(&data.array) };
        shared_secret
    }
}

#[cfg(test)]
pub mod test {
    use rand::{Rng, SeedableRng};

    use crate::wii::crypto::{
        bn::bn_inv,
        ec::{EC_N, priv_to_pub, sign, verify_signature},
    };

    #[test]
    #[ignore = "This test might not be the right way to check if a signature is valid"]
    fn signature_is_valid() {
        let mut private_key = [0u8; _];
        let mut hash = [0x55; _];
        let mut rng = rand_chacha::ChaCha20Rng::from_os_rng();
        private_key.copy_from_slice(&(&mut rng).random_iter().take(30).collect::<Vec<_>>());
        hash.copy_from_slice(&(&mut rng).random_iter().take(20).collect::<Vec<_>>());
        let sign = sign(dbg!(private_key).clone(), dbg!(hash));
        let public_key = priv_to_pub(private_key.clone());
        assert!(verify_signature(dbg!(public_key), dbg!(sign), hash));
    }

    #[test]
    fn inverse_works() {
        let data: [u8; _] = [
            56, 158, 5, 55, 152, 22, 172, 27, 213, 215, 38, 222, 216, 46, 231, 10, 168, 193, 73,
            77, 251, 218, 104, 137, 190, 12, 245, 86, 96, 184,
        ];
        let expected: [u8; _] = [
            253, 117, 216, 243, 122, 128, 63, 124, 199, 79, 89, 48, 109, 93, 190, 58, 58, 142, 159,
            28, 184, 115, 155, 122, 139, 211, 184, 209, 203, 223,
        ];
        let mut out = [0u8; 30];
        bn_inv(&mut out, &data, &EC_N);
        assert_eq!(out, expected);
    }
}
