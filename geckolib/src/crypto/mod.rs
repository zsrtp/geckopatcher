use aes::{
    cipher::{BlockDecryptMut, BlockEncryptMut, KeyIvInit},
    Block,
};
use std::ops::Deref;
use thiserror::Error;

pub mod consts {
    // DOL_ALIGNMENT and FST_ALIGNMENT are set to 1024 and 256 to match the
    // original ISO. Due to poor documentation of how, and why, these values
    // should or shouldn't be changed we opted to preserve their values since
    // there was no observed benefit of setting them higher, however lower
    // values were not tested.

    pub const WII_HASH_SIZE: usize = 160 / 8; // 160 bits with 8 bits per byte
    pub const WII_KEY_SIZE: usize = 16;
    pub const WII_CKEY_AMNT: usize = 3;
    pub const WII_H3_OFFSET: usize = 0x8000;
    pub const WII_H3_SIZE: usize = 0x18000;
    pub const WII_HX_OFFSETS: [usize; 3] = [0, 0x280, 0x340];
    pub const WII_SECTOR_HASH_SIZE: usize = 0x400;
    pub const WII_SECTOR_SIZE: usize = 0x8000;
    pub const WII_SECTOR_DATA_SIZE: usize = WII_SECTOR_SIZE - WII_SECTOR_HASH_SIZE;
    pub const WII_SECTOR_IV_OFF: usize = 0x3D0;
    pub const WII_PARTITION_INFO_OFF: usize = 0x40000;
    pub const WII_SECTOR_DATA_HASH_SIZE: usize = 0x400; // Size of chunks of data hashed together for h0
    /// Number of chunks of data which are hashed for h0.
    pub const WII_SECTOR_DATA_HASH_COUNT: usize = WII_SECTOR_DATA_SIZE / WII_SECTOR_DATA_HASH_SIZE;
}

const COMMON_KEY_: [[u8; consts::WII_KEY_SIZE]; consts::WII_CKEY_AMNT] = [
    [
        2, 26, 224, 229, 43, 205, 59, 3, 6, 0, 157, 118, 65, 31, 22, 93,
    ],
    [0x0; consts::WII_KEY_SIZE],
    [0x0; consts::WII_KEY_SIZE],
];
const COMMON_KEY_MASK: [[u8; consts::WII_KEY_SIZE]; consts::WII_CKEY_AMNT] = [
    [
        233, 254, 202, 199, 117, 72, 168, 231, 78, 217, 88, 51, 50, 158, 188, 170,
    ],
    [0x0; consts::WII_KEY_SIZE],
    [0x0; consts::WII_KEY_SIZE],
];

lazy_static! {
    pub static ref COMMON_KEY: [[u8; consts::WII_KEY_SIZE]; consts::WII_CKEY_AMNT] = {
        let mut ck = [[0u8; consts::WII_KEY_SIZE]; consts::WII_CKEY_AMNT];
        for (v, (k, m)) in ck.iter_mut().flatten().zip(
            COMMON_KEY_
                .iter()
                .flatten()
                .zip(COMMON_KEY_MASK.iter().flatten()),
        ) {
            *v = *k ^ *m;
        }
        ck
    };
}

// create an alias for convenience
pub type Aes128CbcEnc = cbc::Encryptor<aes::Aes128>;
pub type Aes128CbcDec = cbc::Decryptor<aes::Aes128>;

#[derive(Copy, Clone)]
pub struct AesKey {
    array: [u8; 0x10],
}

impl From<[u8; 0x10]> for AesKey {
    fn from(array: [u8; 0x10]) -> AesKey {
        AesKey { array }
    }
}

impl From<&[u8]> for AesKey {
    fn from(buf: &[u8]) -> AesKey {
        let mut key = AesKey { array: [0u8; 0x10] };
        let copy_len = std::cmp::min(buf.len(), 0x10);
        key.array[..copy_len].copy_from_slice(&buf[..copy_len]);
        key
    }
}

impl std::ops::Deref for AesKey {
    type Target = [u8; 0x10];

    fn deref(&self) -> &<Self as std::ops::Deref>::Target {
        &self.array
    }
}

impl std::ops::DerefMut for AesKey {
    fn deref_mut(&mut self) -> &mut <Self as std::ops::Deref>::Target {
        &mut self.array
    }
}

impl Default for AesKey {
    fn default() -> AesKey {
        AesKey::from([0u8; 0x10])
    }
}

impl std::fmt::Debug for AesKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::result::Result<(), std::fmt::Error> {
        let mut arr = String::new();
        for i in 0..0x10 {
            arr += format!("{:#02}", self.array[i]).as_str();
        }
        f.debug_struct("Keys").field("array", &arr).finish()
    }
}

pub struct DecryptedBlock {
    array: [u8; 0x7C00],
}

impl From<[u8; 0x7C00]> for DecryptedBlock {
    fn from(array: [u8; 0x7C00]) -> DecryptedBlock {
        DecryptedBlock { array }
    }
}

impl From<&[u8]> for DecryptedBlock {
    fn from(buf: &[u8]) -> DecryptedBlock {
        let mut key = DecryptedBlock {
            array: [0u8; 0x7C00],
        };
        let copy_len = std::cmp::min(buf.len(), 0x7C00);
        key.array[..copy_len].copy_from_slice(&buf[..copy_len]);
        key
    }
}

impl std::ops::Deref for DecryptedBlock {
    type Target = [u8; 0x7C00];

    fn deref(&self) -> &<Self as std::ops::Deref>::Target {
        &self.array
    }
}

impl std::ops::DerefMut for DecryptedBlock {
    fn deref_mut(&mut self) -> &mut <Self as std::ops::Deref>::Target {
        &mut self.array
    }
}

impl Default for DecryptedBlock {
    fn default() -> DecryptedBlock {
        DecryptedBlock::from([0u8; 0x7C00])
    }
}
pub type Sha1Digest = [u8; consts::WII_HASH_SIZE];

#[derive(Debug, Clone, Copy)]
#[repr(C, packed(1))]
pub struct HashBlockStruct {
    pub h0: [Sha1Digest; 31],
    pub padding_0: [u8; 20],
    pub h1: [Sha1Digest; 8],
    pub padding_1: [u8; 32],
    pub h2: [Sha1Digest; 8],
    pub padding_2: [u8; 32],
}
assert_eq_size!(HashBlockStruct, [u8; consts::WII_SECTOR_HASH_SIZE]);

#[repr(C, packed(1))]
pub union HashBlock {
    pub s: HashBlockStruct,
    pub array: [u8; consts::WII_SECTOR_HASH_SIZE],
}
assert_eq_size!(HashBlock, [u8; consts::WII_SECTOR_HASH_SIZE]);

#[derive(Error, Debug)]
pub enum WiiCryptoError {
    #[error("Invalid Wii disc: magic is {magic:#010X}")]
    NotWiiDisc { magic: u32 },
    #[error("There is no game parition in this disc")]
    NoGamePartition,
    #[error("Could not decrypt block")]
    AesDecryptError,
    #[error("Could not encrypt block")]
    AesEncryptError,
    #[error("The provided slice is too small to be converted into a {name}.")]
    ConvertError { name: String },
}

impl From<WiiCryptoError> for std::io::Error {
    fn from(value: WiiCryptoError) -> Self {
        std::io::Error::new(std::io::ErrorKind::Other, value)
    }
}

pub trait Unpackable {
    const BLOCK_SIZE: usize;
}

#[macro_export]
macro_rules! declare_tryfrom {
    ( $T:ty ) => {
        impl TryFrom<&[u8]> for $T {
            type Error = WiiCryptoError;

            fn try_from(slice: &[u8]) -> Result<$T, Self::Error> {
                if slice.len() < <$T as Unpackable>::BLOCK_SIZE {
                    Err(WiiCryptoError::ConvertError {
                        name: stringify![$T].to_string(),
                    })
                } else {
                    let mut buf = [0_u8; <$T as Unpackable>::BLOCK_SIZE];
                    buf.clone_from_slice(&slice[..<$T as Unpackable>::BLOCK_SIZE]);
                    Ok(<$T>::from(&buf))
                }
            }
        }
    };
}

pub fn aes_decrypt_inplace<K: Deref<Target = [u8; consts::WII_KEY_SIZE]>>(
    data: &mut [u8],
    iv: K,
    key: K,
) {
    let mut cipher = Aes128CbcDec::new_from_slices(&*key, &*iv).unwrap();
    data.chunks_exact_mut(consts::WII_KEY_SIZE)
        .for_each(|chunk| cipher.decrypt_block_mut(Block::from_mut_slice(chunk)));
}

pub fn aes_encrypt_inplace<K: Deref<Target = [u8; consts::WII_KEY_SIZE]>>(
    data: &mut [u8],
    iv: K,
    key: K,
) {
    let mut cipher = Aes128CbcEnc::new_from_slices(&*key, &*iv).unwrap();
    data.chunks_exact_mut(consts::WII_KEY_SIZE)
        .for_each(|chunk| cipher.encrypt_block_mut(Block::from_mut_slice(chunk)));
}
