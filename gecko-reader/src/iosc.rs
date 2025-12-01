#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[repr(u32)]
pub enum IOSUid {
    PidKernel = 0,
    PidEs = 1,
    PidFs = 2,
    PidDi = 3,
    PidOh0 = 4,
    PidOh1 = 5,
    PidEhci = 6,
    PidSdi = 7,
    PidUsbeth = 8,
    PidNet = 9,
    PidWd = 10,
    PidWl = 11,
    PidKd = 12,
    PidNcd = 13,
    PidStm = 14,
    PidPpcboot = 15,
    PidSsl = 16,
    PidUsb = 17,
    PidP2p = 18,
    #[default]
    PidUnknown = 19,
}

impl From<IOSUid> for u32 {
    fn from(value: IOSUid) -> Self {
        value as u32
    }
}

impl From<u32> for IOSUid {
    fn from(value: u32) -> Self {
        match value {
            0 => IOSUid::PidKernel,
            1 => IOSUid::PidEs,
            2 => IOSUid::PidFs,
            3 => IOSUid::PidDi,
            4 => IOSUid::PidOh0,
            5 => IOSUid::PidOh1,
            6 => IOSUid::PidEhci,
            7 => IOSUid::PidSdi,
            8 => IOSUid::PidUsbeth,
            9 => IOSUid::PidNet,
            10 => IOSUid::PidWd,
            11 => IOSUid::PidWl,
            12 => IOSUid::PidKd,
            13 => IOSUid::PidNcd,
            14 => IOSUid::PidStm,
            15 => IOSUid::PidPpcboot,
            16 => IOSUid::PidSsl,
            17 => IOSUid::PidUsb,
            18 => IOSUid::PidP2p,
            _ => IOSUid::PidUnknown,
        }
    }
}

pub type Signature = [u8; 60];
pub type PublicKey = [u8; 60];

pub const AES128_KEY_SIZE: usize = 0x10;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u32)]
pub enum SignatureType {
    #[default]
    RSA4096 = 0x00010000,
    RSA2048 = 0x00010001,
    ECC = 0x00010002,
}
crate::static_assert_eq_size!(SignatureType, 4);

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u32)]
pub enum PublicKeyType {
    #[default]
    RSA4096 = 0,
    RSA2048 = 1,
    ECC = 2,
}

impl TryFrom<u32> for PublicKeyType {
    type Error = u32;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        Self::try_from(&value).map_err(|_| value)
    }
}

impl<'a> TryFrom<&'a u32> for PublicKeyType {
    type Error = &'a u32;

    fn try_from(value: &'a u32) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(PublicKeyType::RSA4096),
            1 => Ok(PublicKeyType::RSA2048),
            2 => Ok(PublicKeyType::ECC),
            _ => Err(value),
        }
    }
}

#[derive(Debug, Clone, Copy)]
#[repr(C)]
#[repr(packed)]
pub struct SignatureRSA4096 {
    pub sig_type: SignatureType,
    pub sig: [u8; 0x200],
    pub fill: [u8; 0x3c],
    pub issuer: [u8; 0x40],
}
pub const SIGNATURE_RSA4096_SIZE: usize = 0x280;
crate::static_assert_eq_size!(SignatureRSA4096, 0x280);

impl Default for SignatureRSA4096 {
    fn default() -> Self {
        Self {
            sig_type: Default::default(),
            sig: [0u8; _],
            fill: [0; _],
            issuer: [0; _],
        }
    }
}

#[derive(Debug, Clone, Copy)]
#[repr(C)]
#[repr(packed)]
pub struct SignatureRSA2048 {
    pub sig_type: SignatureType,
    pub sig: [u8; 0x100],
    pub fill: [u8; 0x3c],
    pub issuer: [u8; 0x40],
}
pub const SIGNATURE_RSA2048_SIZE: usize = 0x180;
crate::static_assert_eq_size!(SignatureRSA2048, 0x180);

impl Default for SignatureRSA2048 {
    fn default() -> Self {
        Self {
            sig_type: Default::default(),
            sig: [0; _],
            fill: [0; _],
            issuer: [0; _],
        }
    }
}

#[derive(Debug, Clone, Copy)]
#[repr(C)]
#[repr(packed)]
pub struct SignatureECC {
    pub sig_type: SignatureType,
    pub sig: [u8; 0x3c],
    pub fill: [u8; 0x40],
    pub issuer: [u8; 0x40],
}
pub const SIGNATURE_ECC_SIZE: usize = 0xc0;
crate::static_assert_eq_size!(SignatureECC, 0xc0);

impl Default for SignatureECC {
    fn default() -> Self {
        Self {
            sig_type: Default::default(),
            sig: [0; _],
            fill: [0; _],
            issuer: [0; _],
        }
    }
}

impl serde_binary::Encode for SignatureECC {
    fn encode(&self, ser: &mut serde_binary::Serializer) -> serde_binary::Result<()> {
        ser.writer.write_u32(self.sig_type as u32)?;
        ser.writer.write_bytes(&self.sig)?;
        ser.writer.write_bytes(&self.fill)?;
        ser.writer.write_bytes(&self.issuer)?;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy)]
pub struct CertHeader {
    pub public_key_type: PublicKeyType,
    pub name: [u8; 0x40],
    pub id: u32,
}
pub const CERT_HEADER_SIZE: usize = 0x48;
crate::static_assert_eq_size!(CertHeader, 0x48);

impl Default for CertHeader {
    fn default() -> Self {
        Self {
            public_key_type: Default::default(),
            name: [0; _],
            id: Default::default(),
        }
    }
}

#[derive(Debug, Clone, Copy)]
#[repr(transparent)]
pub struct RSA2048PublicKey(pub [u8; 0x100]);
pub const RSA2048_PUBLIC_KEY_SIZE: usize = 0x100;
crate::static_assert_eq_size!(RSA2048PublicKey, 0x100);

impl Default for RSA2048PublicKey {
    fn default() -> Self {
        Self([0; _])
    }
}

#[derive(Debug, Clone, Copy)]
#[repr(transparent)]
pub struct ECCPublicKey(pub [u8; 0x3c]);
pub const ECC_PUBLIC_KEY_SIZE: usize = 0x3c;
crate::static_assert_eq_size!(ECCPublicKey, ECC_PUBLIC_KEY_SIZE);

impl Default for ECCPublicKey {
    fn default() -> Self {
        Self([0; _])
    }
}

#[derive(Debug, Clone, Copy)]
#[repr(C)]
#[repr(packed)]
pub struct CertRSA4096RSA2048 {
    pub signature: SignatureRSA4096,
    pub header: CertHeader,
    pub public_key: RSA2048PublicKey,
    pub exponent: [u8; 0x4],
    pub pad: [u8; 0x34],
}
pub const CERT_RSA4096_RSA2048_SIZE: usize = 0x400;
crate::static_assert_eq_size!(CertRSA4096RSA2048, 0x400);

impl Default for CertRSA4096RSA2048 {
    fn default() -> Self {
        Self {
            signature: Default::default(),
            header: Default::default(),
            public_key: Default::default(),
            exponent: [0; _],
            pad: [0; _],
        }
    }
}

#[derive(Debug, Clone, Copy)]
#[repr(C)]
#[repr(packed)]
pub struct CertRSA2048RSA2048 {
    pub signature: SignatureRSA2048,
    pub header: CertHeader,
    pub public_key: RSA2048PublicKey,
    pub exponent: [u8; 0x4],
    pub pad: [u8; 0x34],
}
pub const CERT_RSA2048_RSA2048_SIZE: usize = 0x300;
crate::static_assert_eq_size!(CertRSA2048RSA2048, 0x300);

impl Default for CertRSA2048RSA2048 {
    fn default() -> Self {
        Self {
            signature: Default::default(),
            header: Default::default(),
            public_key: Default::default(),
            exponent: [0; _],
            pad: [0; _],
        }
    }
}

#[derive(Debug, Clone, Copy)]
#[repr(C)]
#[repr(packed)]
pub struct CertRSA2048ECC {
    pub signature: SignatureRSA2048,
    pub header: CertHeader,
    pub public_key: ECCPublicKey,
    pub padding: [u8; 0x3c],
}
pub const CERT_RSA2048_ECC_SIZE: usize = 0x240;
crate::static_assert_eq_size!(CertRSA2048ECC, 0x240);

impl Default for CertRSA2048ECC {
    fn default() -> Self {
        Self {
            signature: Default::default(),
            header: Default::default(),
            public_key: Default::default(),
            padding: [0; _],
        }
    }
}

#[derive(Debug, Clone, Copy)]
#[repr(C)]
#[repr(packed)]
pub struct CertECC {
    pub signature: SignatureECC,
    pub header: CertHeader,
    pub public_key: ECCPublicKey,
    pub padding: [u8; 0x3c],
}
pub const CERT_ECC_SIZE: usize = 0x180;
crate::static_assert_eq_size!(CertECC, 0x180);

impl Default for CertECC {
    fn default() -> Self {
        Self {
            signature: Default::default(),
            header: Default::default(),
            public_key: Default::default(),
            padding: [0; _],
        }
    }
}

pub mod hle {
    use std::ffi::CString;

    use byteorder::ByteOrder;
    use sha1::Digest;

    #[cfg(test)]
    use crate::iosc::PublicKey;
    use crate::iosc::Signature;
    use crate::{
        iosc::{AES128_KEY_SIZE, CertECC, ECCPublicKey, SignatureType},
        wii::crypto::{self, CertReader, SignedReader, aes_decrypt_inplace, aes_encrypt_inplace},
    };

    const ROOT_PUBLIC_KEY: [u8; 512] = [
        0xF8, 0x24, 0x6C, 0x58, 0xBA, 0xE7, 0x50, 0x03, 0x01, 0xFB, 0xB7, 0xC2, 0xEB, 0xE0, 0x01,
        0x05, 0x71, 0xDA, 0x92, 0x23, 0x78, 0xF0, 0x51, 0x4E, 0xC0, 0x03, 0x1D, 0xD0, 0xD2, 0x1E,
        0xD3, 0xD0, 0x7E, 0xFC, 0x85, 0x20, 0x69, 0xB5, 0xDE, 0x9B, 0xB9, 0x51, 0xA8, 0xBC, 0x90,
        0xA2, 0x44, 0x92, 0x6D, 0x37, 0x92, 0x95, 0xAE, 0x94, 0x36, 0xAA, 0xA6, 0xA3, 0x02, 0x51,
        0x0C, 0x7B, 0x1D, 0xED, 0xD5, 0xFB, 0x20, 0x86, 0x9D, 0x7F, 0x30, 0x16, 0xF6, 0xBE, 0x65,
        0xD3, 0x83, 0xA1, 0x6D, 0xB3, 0x32, 0x1B, 0x95, 0x35, 0x18, 0x90, 0xB1, 0x70, 0x02, 0x93,
        0x7E, 0xE1, 0x93, 0xF5, 0x7E, 0x99, 0xA2, 0x47, 0x4E, 0x9D, 0x38, 0x24, 0xC7, 0xAE, 0xE3,
        0x85, 0x41, 0xF5, 0x67, 0xE7, 0x51, 0x8C, 0x7A, 0x0E, 0x38, 0xE7, 0xEB, 0xAF, 0x41, 0x19,
        0x1B, 0xCF, 0xF1, 0x7B, 0x42, 0xA6, 0xB4, 0xED, 0xE6, 0xCE, 0x8D, 0xE7, 0x31, 0x8F, 0x7F,
        0x52, 0x04, 0xB3, 0x99, 0x0E, 0x22, 0x67, 0x45, 0xAF, 0xD4, 0x85, 0xB2, 0x44, 0x93, 0x00,
        0x8B, 0x08, 0xC7, 0xF6, 0xB7, 0xE5, 0x6B, 0x02, 0xB3, 0xE8, 0xFE, 0x0C, 0x9D, 0x85, 0x9C,
        0xB8, 0xB6, 0x82, 0x23, 0xB8, 0xAB, 0x27, 0xEE, 0x5F, 0x65, 0x38, 0x07, 0x8B, 0x2D, 0xB9,
        0x1E, 0x2A, 0x15, 0x3E, 0x85, 0x81, 0x80, 0x72, 0xA2, 0x3B, 0x6D, 0xD9, 0x32, 0x81, 0x05,
        0x4F, 0x6F, 0xB0, 0xF6, 0xF5, 0xAD, 0x28, 0x3E, 0xCA, 0x0B, 0x7A, 0xF3, 0x54, 0x55, 0xE0,
        0x3D, 0xA7, 0xB6, 0x83, 0x26, 0xF3, 0xEC, 0x83, 0x4A, 0xF3, 0x14, 0x04, 0x8A, 0xC6, 0xDF,
        0x20, 0xD2, 0x85, 0x08, 0x67, 0x3C, 0xAB, 0x62, 0xA2, 0xC7, 0xBC, 0x13, 0x1A, 0x53, 0x3E,
        0x0B, 0x66, 0x80, 0x6B, 0x1C, 0x30, 0x66, 0x4B, 0x37, 0x23, 0x31, 0xBD, 0xC4, 0xB0, 0xCA,
        0xD8, 0xD1, 0x1E, 0xE7, 0xBB, 0xD9, 0x28, 0x55, 0x48, 0xAA, 0xEC, 0x1F, 0x66, 0xE8, 0x21,
        0xB3, 0xC8, 0xA0, 0x47, 0x69, 0x00, 0xC5, 0xE6, 0x88, 0xE8, 0x0C, 0xCE, 0x3C, 0x61, 0xD6,
        0x9C, 0xBB, 0xA1, 0x37, 0xC6, 0x60, 0x4F, 0x7A, 0x72, 0xDD, 0x8C, 0x7B, 0x3E, 0x3D, 0x51,
        0x29, 0x0D, 0xAA, 0x6A, 0x59, 0x7B, 0x08, 0x1F, 0x9D, 0x36, 0x33, 0xA3, 0x46, 0x7A, 0x35,
        0x61, 0x09, 0xAC, 0xA7, 0xDD, 0x7D, 0x2E, 0x2F, 0xB2, 0xC1, 0xAE, 0xB8, 0xE2, 0x0F, 0x48,
        0x92, 0xD8, 0xB9, 0xF8, 0xB4, 0x6F, 0x4E, 0x3C, 0x11, 0xF4, 0xF4, 0x7D, 0x8B, 0x75, 0x7D,
        0xFE, 0xFE, 0xA3, 0x89, 0x9C, 0x33, 0x59, 0x5C, 0x5E, 0xFD, 0xEB, 0xCB, 0xAB, 0xE8, 0x41,
        0x3E, 0x3A, 0x9A, 0x80, 0x3C, 0x69, 0x35, 0x6E, 0xB2, 0xB2, 0xAD, 0x5C, 0xC4, 0xC8, 0x58,
        0x45, 0x5E, 0xF5, 0xF7, 0xB3, 0x06, 0x44, 0xB4, 0x7C, 0x64, 0x06, 0x8C, 0xDF, 0x80, 0x9F,
        0x76, 0x02, 0x5A, 0x2D, 0xB4, 0x46, 0xE0, 0x3D, 0x7C, 0xF6, 0x2F, 0x34, 0xE7, 0x02, 0x45,
        0x7B, 0x02, 0xA4, 0xCF, 0x5D, 0x9D, 0xD5, 0x3C, 0xA5, 0x3A, 0x7C, 0xA6, 0x29, 0x78, 0x8C,
        0x67, 0xCA, 0x08, 0xBF, 0xEC, 0xCA, 0x43, 0xA9, 0x57, 0xAD, 0x16, 0xC9, 0x4E, 0x1C, 0xD8,
        0x75, 0xCA, 0x10, 0x7D, 0xCE, 0x7E, 0x01, 0x18, 0xF0, 0xDF, 0x6B, 0xFE, 0xE5, 0x1D, 0xDB,
        0xD9, 0x91, 0xC2, 0x6E, 0x60, 0xCD, 0x48, 0x58, 0xAA, 0x59, 0x2C, 0x82, 0x00, 0x75, 0xF2,
        0x9F, 0x52, 0x6C, 0x91, 0x7C, 0x6F, 0xE5, 0x40, 0x3E, 0xA7, 0xD4, 0xA5, 0x0C, 0xEC, 0x3B,
        0x73, 0x84, 0xDE, 0x88, 0x6E, 0x82, 0xD2, 0xEB, 0x4D, 0x4E, 0x42, 0xB5, 0xF2, 0xB1, 0x49,
        0xA8, 0x1E, 0xA7, 0xCE, 0x71, 0x44, 0xDC, 0x29, 0x94, 0xCF, 0xC4, 0x4E, 0x1F, 0x91, 0xCB,
        0xD4, 0x95,
    ];

    const ROOT_PUBLIC_KEY_DEV: [u8; 512] = [
        0xD0, 0x1F, 0xE1, 0x00, 0xD4, 0x35, 0x56, 0xB2, 0x4B, 0x56, 0xDA, 0xE9, 0x71, 0xB5, 0xA5,
        0xD3, 0x84, 0xB9, 0x30, 0x03, 0xBE, 0x1B, 0xBF, 0x28, 0xA2, 0x30, 0x5B, 0x06, 0x06, 0x45,
        0x46, 0x7D, 0x5B, 0x02, 0x51, 0xD2, 0x56, 0x1A, 0x27, 0x4F, 0x9E, 0x9F, 0x9C, 0xEC, 0x64,
        0x61, 0x50, 0xAB, 0x3D, 0x2A, 0xE3, 0x36, 0x68, 0x66, 0xAC, 0xA4, 0xBA, 0xE8, 0x1A, 0xE3,
        0xD7, 0x9A, 0xA6, 0xB0, 0x4A, 0x8B, 0xCB, 0xA7, 0xE6, 0xFB, 0x64, 0x89, 0x45, 0xEB, 0xDF,
        0xDB, 0x85, 0xBA, 0x09, 0x1F, 0xD7, 0xD1, 0x14, 0xB5, 0xA3, 0xA7, 0x80, 0xE3, 0xA2, 0x2E,
        0x6E, 0xCD, 0x87, 0xB5, 0xA4, 0xC6, 0xF9, 0x10, 0xE4, 0x03, 0x22, 0x08, 0x81, 0x4B, 0x0C,
        0xEE, 0xA1, 0xA1, 0x7D, 0xF7, 0x39, 0x69, 0x5F, 0x61, 0x7E, 0xF6, 0x35, 0x28, 0xDB, 0x94,
        0x96, 0x37, 0xA0, 0x56, 0x03, 0x7F, 0x7B, 0x32, 0x41, 0x38, 0x95, 0xC0, 0xA8, 0xF1, 0x98,
        0x2E, 0x15, 0x65, 0xE3, 0x8E, 0xED, 0xC2, 0x2E, 0x59, 0x0E, 0xE2, 0x67, 0x7B, 0x86, 0x09,
        0xF4, 0x8C, 0x2E, 0x30, 0x3F, 0xBC, 0x40, 0x5C, 0xAC, 0x18, 0x04, 0x2F, 0x82, 0x20, 0x84,
        0xE4, 0x93, 0x68, 0x03, 0xDA, 0x7F, 0x41, 0x34, 0x92, 0x48, 0x56, 0x2B, 0x8E, 0xE1, 0x2F,
        0x78, 0xF8, 0x03, 0x24, 0x63, 0x30, 0xBC, 0x7B, 0xE7, 0xEE, 0x72, 0x4A, 0xF4, 0x58, 0xA4,
        0x72, 0xE7, 0xAB, 0x46, 0xA1, 0xA7, 0xC1, 0x0C, 0x2F, 0x18, 0xFA, 0x07, 0xC3, 0xDD, 0xD8,
        0x98, 0x06, 0xA1, 0x1C, 0x9C, 0xC1, 0x30, 0xB2, 0x47, 0xA3, 0x3C, 0x8D, 0x47, 0xDE, 0x67,
        0xF2, 0x9E, 0x55, 0x77, 0xB1, 0x1C, 0x43, 0x49, 0x3D, 0x5B, 0xBA, 0x76, 0x34, 0xA7, 0xE4,
        0xE7, 0x15, 0x31, 0xB7, 0xDF, 0x59, 0x81, 0xFE, 0x24, 0xA1, 0x14, 0x55, 0x4C, 0xBD, 0x8F,
        0x00, 0x5C, 0xE1, 0xDB, 0x35, 0x08, 0x5C, 0xCF, 0xC7, 0x78, 0x06, 0xB6, 0xDE, 0x25, 0x40,
        0x68, 0xA2, 0x6C, 0xB5, 0x49, 0x2D, 0x45, 0x80, 0x43, 0x8F, 0xE1, 0xE5, 0xA9, 0xED, 0x75,
        0xC5, 0xED, 0x45, 0x1D, 0xCE, 0x78, 0x94, 0x39, 0xCC, 0xC3, 0xBA, 0x28, 0xA2, 0x31, 0x2A,
        0x1B, 0x87, 0x19, 0xEF, 0x0F, 0x73, 0xB7, 0x13, 0x95, 0x0C, 0x02, 0x59, 0x1A, 0x74, 0x62,
        0xA6, 0x07, 0xF3, 0x7C, 0x0A, 0xA7, 0xA1, 0x8F, 0xA9, 0x43, 0xA3, 0x6D, 0x75, 0x2A, 0x5F,
        0x41, 0x92, 0xF0, 0x13, 0x61, 0x00, 0xAA, 0x9C, 0xB4, 0x1B, 0xBE, 0x14, 0xBE, 0xB1, 0xF9,
        0xFC, 0x69, 0x2F, 0xDF, 0xA0, 0x94, 0x46, 0xDE, 0x5A, 0x9D, 0xDE, 0x2C, 0xA5, 0xF6, 0x8C,
        0x1C, 0x0C, 0x21, 0x42, 0x92, 0x87, 0xCB, 0x2D, 0xAA, 0xA3, 0xD2, 0x63, 0x75, 0x2F, 0x73,
        0xE0, 0x9F, 0xAF, 0x44, 0x79, 0xD2, 0x81, 0x74, 0x29, 0xF6, 0x98, 0x00, 0xAF, 0xDE, 0x6B,
        0x59, 0x2D, 0xC1, 0x98, 0x82, 0xBD, 0xF5, 0x81, 0xCC, 0xAB, 0xF2, 0xCB, 0x91, 0x02, 0x9E,
        0xF3, 0x5C, 0x4C, 0xFD, 0xBB, 0xFF, 0x49, 0xC1, 0xFA, 0x1B, 0x2F, 0xE3, 0x1D, 0xE7, 0xA5,
        0x60, 0xEC, 0xB4, 0x7E, 0xBC, 0xFE, 0x32, 0x42, 0x5B, 0x95, 0x6F, 0x81, 0xB6, 0x99, 0x17,
        0x48, 0x7E, 0x3B, 0x78, 0x91, 0x51, 0xDB, 0x2E, 0x78, 0xB1, 0xFD, 0x2E, 0xBE, 0x7E, 0x62,
        0x6B, 0x3E, 0xA1, 0x65, 0xB4, 0xFB, 0x00, 0xCC, 0xB7, 0x51, 0xAF, 0x50, 0x73, 0x29, 0xC4,
        0xA3, 0x93, 0x9E, 0xA6, 0xDD, 0x9C, 0x50, 0xA0, 0xE7, 0x38, 0x6B, 0x01, 0x45, 0x79, 0x6B,
        0x41, 0xAF, 0x61, 0xF7, 0x85, 0x55, 0x94, 0x4F, 0x3B, 0xC2, 0x2D, 0xC3, 0xBD, 0x0D, 0x00,
        0xF8, 0x79, 0x8A, 0x42, 0xB1, 0xAA, 0xA0, 0x83, 0x20, 0x65, 0x9A, 0xC7, 0x39, 0x5A, 0xB4,
        0xF3, 0x29,
    ];

    pub const DEFAULT_DEVICE_ID: u32 = 0x0403AC68;
    pub const DEFAULT_KEY_ID: u32 = 0x6AAB8C59;

    const DEFAULT_PRIVATE_KEY: [u8; 30] = [
        0x00, 0xAB, 0xEE, 0xC1, 0xDD, 0xB4, 0xA6, 0x16, 0x6B, 0x70, 0xFD, 0x7E, 0x56, 0x67, 0x70,
        0x57, 0x55, 0x27, 0x38, 0xA3, 0x26, 0xC5, 0x46, 0x16, 0xF7, 0x62, 0xC9, 0xED, 0x73, 0xF2,
    ];

    #[cfg(test)]
    const DEFAULT_PUBLIC_KEY: PublicKey = [
        0x01, 0x04, 0x0b, 0xe0, 0x46, 0xea, 0x95, 0x19, 0xf2, 0x85, 0x9b, 0x0d, 0x94, 0x29, 0xa2,
        0xc6, 0x91, 0x80, 0x15, 0x89, 0x8f, 0x2e, 0xba, 0x20, 0xcf, 0xfd, 0xb3, 0x16, 0x4f, 0x0c,
        0x01, 0x38, 0xc5, 0xd2, 0x2f, 0xc1, 0xe9, 0xee, 0x17, 0x6c, 0x2d, 0x8f, 0xa4, 0x74, 0xb0,
        0xe9, 0x38, 0x66, 0x6e, 0x60, 0xcf, 0x06, 0xd5, 0x08, 0x7a, 0xc2, 0x4f, 0x01, 0x39, 0x79,
    ];

    const DEFAULT_SIGNATURE: super::Signature = [
        // R
        0x00, 0xD8, 0x81, 0x63, 0xB2, 0x00, 0x6B, 0x0B, 0x54, 0x82, 0x88, 0x63, 0x81, 0x1C, 0x00,
        0x71, 0x12, 0xED, 0xB7, 0xFD, 0x21, 0xAB, 0x0E, 0x50, 0x0E, 0x1F, 0xBF, 0x78, 0xAD, 0x37,
        // S
        0x00, 0x71, 0x8D, 0x82, 0x41, 0xEE, 0x45, 0x11, 0xC7, 0x3B, 0xAC, 0x08, 0xB6, 0x83, 0xDC,
        0x05, 0xB8, 0xA8, 0x90, 0x1F, 0xA8, 0x2A, 0x0E, 0x4E, 0x76, 0xEF, 0x44, 0x72, 0x99, 0xF8,
    ];

    fn get_size_for_type(type_: ObjectType, sub_type: ObjectSubType) -> Option<usize> {
        match (type_, sub_type) {
            (ObjectType::TypeSecretKey, ObjectSubType::AES128) => Some(16),
            (ObjectType::TypeSecretKey, ObjectSubType::MAC) => Some(20),
            (ObjectType::TypeSecretKey, ObjectSubType::ECC233) => Some(30),
            (ObjectType::TypePublicKey, ObjectSubType::RSA2048) => Some(256),
            (ObjectType::TypePublicKey, ObjectSubType::RSA4096) => Some(512),
            (ObjectType::TypePublicKey, ObjectSubType::ECC233) => Some(60),
            _ => None,
        }
    }

    #[derive(Debug, Clone, Copy)]
    pub struct Handle(u32);

    impl From<&u32> for Handle {
        fn from(value: &u32) -> Self {
            Self(*value)
        }
    }

    impl From<&Handle> for u32 {
        fn from(value: &Handle) -> Self {
            value.0
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum ConsoleType {
        Retail,
        RVT,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
    #[repr(u32)]
    pub enum DefaultHandle {
        /// NG private key. ECC-233 private signing key (per-console)
        HandleConsoleKey = 0,
        /// Console ID
        HandleConsoleId = 1,
        /// NAND FS AES-128 key
        HandleFsKey = 2,
        /// NAND FS HMAC
        HandleFsMac = 3,
        /// Common key
        HandleCommonKey = 4,
        /// PRNG seed
        HandlePrngKey = 5,
        /// SD AES-128 key
        HandleSdKey = 6,
        /// boot2 version (writable)
        HandleBoot2Version = 7,
        /// Unknown
        HandleUnknown8 = 8,
        /// Unknown
        HandleUnknown9 = 9,
        /// Filesystem version (writable)
        HandleFsVersion = 10,
        /// New common key (aka Korean common key)
        HandleNewCommonKey = 11,
        HandleRootKey = 0xfffffff,
    }

    impl From<DefaultHandle> for Handle {
        fn from(value: DefaultHandle) -> Self {
            Self(value as u32)
        }
    }

    pub const COMMON_KEY_HANDLES: [DefaultHandle; 2] = [
        DefaultHandle::HandleCommonKey,
        DefaultHandle::HandleNewCommonKey,
    ];

    #[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
    #[repr(u8)]
    pub enum ObjectType {
        #[default]
        TypeSecretKey = 0,
        TypePublicKey = 1,
        TypeData = 3,
    }

    #[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
    #[repr(u8)]
    pub enum ObjectSubType {
        #[default]
        AES128 = 0,
        MAC = 1,
        RSA2048 = 2,
        RSA4096 = 3,
        ECC233 = 4,
        Data = 5,
        Version = 6,
    }

    #[derive(Debug, Clone)]
    struct KeyEntry {
        in_use: bool,
        key_type: ObjectType,
        sub_type: ObjectSubType,
        data: Vec<u8>,
        misc_data: u32,
        owner_mask: u32,
    }

    impl KeyEntry {
        fn new(
            type_: ObjectType,
            sub_type: ObjectSubType,
            data: Vec<u8>,
            owner_mask: u32,
            misc_data: Option<u32>,
        ) -> Self {
            Self {
                in_use: true,
                key_type: type_,
                sub_type,
                data,
                misc_data: misc_data.unwrap_or_default(),
                owner_mask,
            }
        }
    }

    impl Default for KeyEntry {
        fn default() -> Self {
            Self {
                in_use: false,
                key_type: Default::default(),
                sub_type: Default::default(),
                data: Default::default(),
                misc_data: Default::default(),
                owner_mask: Default::default(),
            }
        }
    }

    #[cfg(feature = "fs")]
    #[derive(Debug, Clone, Copy, Default)]
    #[repr(C)]
    #[repr(packed)]
    struct BootMiiKeyDumpUnionNgPriv {
        pub data: [u8; 0x1e],
        pub pad1: [u8; 0x12],
    }
    #[cfg(feature = "fs")]
    crate::static_assert_eq_size!(BootMiiKeyDumpUnionNgPriv, 0x30);

    #[cfg(feature = "fs")]
    #[derive(Debug, Clone, Copy, Default)]
    #[repr(C)]
    #[repr(packed)]
    struct BootMiiKeyDumpUnionNandHmac {
        pub pad2: [u8; 0x1c],
        pub data: [u8; 0x14],
    }
    #[cfg(feature = "fs")]
    crate::static_assert_eq_size!(BootMiiKeyDumpUnionNandHmac, 0x30);

    #[cfg(feature = "fs")]
    #[derive(Clone, Copy)]
    #[repr(C)]
    #[repr(packed)]
    union BootMiiKeyDumpUnion {
        ng_priv: BootMiiKeyDumpUnionNgPriv,
        nand_hmac: BootMiiKeyDumpUnionNandHmac,
    }
    #[cfg(feature = "fs")]
    crate::static_assert_eq_size!(BootMiiKeyDumpUnion, 0x30);

    #[cfg(feature = "fs")]
    impl core::fmt::Debug for BootMiiKeyDumpUnion {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            unsafe {
                f.debug_tuple("BootMiiKeyDumpUnion")
                    .field(&self.ng_priv)
                    .field(&self.nand_hmac)
                    .finish()
            }
        }
    }

    #[cfg(feature = "fs")]
    impl Default for BootMiiKeyDumpUnion {
        fn default() -> Self {
            BootMiiKeyDumpUnion {
                ng_priv: Default::default(),
            }
        }
    }

    #[cfg(feature = "fs")]
    #[derive(Debug, Clone, Copy, Default)]
    #[repr(C)]
    #[repr(packed)]
    struct BootMiiKeyDumpCounter {
        pub boot2version: u8,
        pub unknown1: u8,
        pub unknown2: u8,
        pub pad: u8,
        pub update_tag: u32,
        pub checksum: u16,
    }
    #[cfg(feature = "fs")]
    crate::static_assert_eq_size!(BootMiiKeyDumpCounter, 0xA);

    /*
     * Structs for keys.bin taken from:
     *
     * mini - a Free Software replacement for the Nintendo/BroadOn IOS.
     * crypto hardware support
     *
     * Copyright (C) 2008, 2009 Haxx Enterprises <bushing@gmail.com>
     * Copyright (C) 2008, 2009 Sven Peter <svenpeter@gmail.com>
     * Copyright (C) 2008, 2009 Hector Martin "marcan" <marcan@marcansoft.com>
     *
     * # This code is licensed to you under the terms of the GNU GPL, version 2;
     * # see file COPYING or http://www.gnu.org/licenses/old-licenses/gpl-2.0.txt
     */
    #[cfg(feature = "fs")]
    #[derive(Debug, Clone, Copy)]
    #[repr(C)]
    #[repr(packed)]
    struct BootMiiKeyDump {
        pub creator: [u8; 0x100],
        pub boot1_hash: [u8; 20],
        pub common_key: [u8; 0x10],
        pub ng_id: u32,
        pub union: BootMiiKeyDumpUnion,
        pub nand_key: [u8; 0x10],
        pub backup_key: [u8; 0x10],
        pub unk1: u32,
        pub unk2: u32,
        pub eeprom_pad: [u8; 0x80],
        pub ms_id: u32,
        pub ca_id: u32,
        pub ng_key_id: u32,
        pub ng_sig: Signature,
        pub counters: [BootMiiKeyDumpCounter; 2],
        pub fill: [u8; 0x18],
        pub korean_key: [u8; 0x10],
        pub pad3: [u8; 0x74],
        pub prng_seed: [u16; 2],
        pub pad4: [u8; 4],
        pub crack_pad: [u8; 0x100],
    }
    #[cfg(feature = "fs")]
    crate::static_assert_eq_size!(BootMiiKeyDump, 0x400);
    #[cfg(feature = "fs")]
    crate::static_assert_eq_offset!(BootMiiKeyDump, common_key, 0x114);
    #[cfg(feature = "fs")]
    crate::static_assert_eq_offset!(BootMiiKeyDump, union, 0x128);
    #[cfg(feature = "fs")]
    crate::static_assert_eq_offset!(BootMiiKeyDump, eeprom_pad, 0x180);
    #[cfg(feature = "fs")]
    crate::static_assert_eq_offset!(BootMiiKeyDump, crack_pad, 0x300);

    #[cfg(feature = "fs")]
    impl Default for BootMiiKeyDump {
        fn default() -> Self {
            Self {
                creator: [0; _],
                boot1_hash: Default::default(),
                common_key: Default::default(),
                ng_id: Default::default(),
                union: Default::default(),
                nand_key: Default::default(),
                backup_key: Default::default(),
                unk1: Default::default(),
                unk2: Default::default(),
                eeprom_pad: [0; _],
                ms_id: Default::default(),
                ca_id: Default::default(),
                ng_key_id: Default::default(),
                ng_sig: [0; _],
                counters: Default::default(),
                fill: Default::default(),
                korean_key: Default::default(),
                pad3: [0; _],
                prng_seed: Default::default(),
                pad4: Default::default(),
                crack_pad: [0; _],
            }
        }
    }

    #[cfg(feature = "fs")]
    impl serde_binary::Decode for BootMiiKeyDump {
        fn decode(&mut self, de: &mut serde_binary::Deserializer) -> serde_binary::Result<()> {
            self.creator.copy_from_slice(&de.reader.read_bytes(0x100)?);
            self.boot1_hash.copy_from_slice(&de.reader.read_bytes(20)?);
            self.common_key
                .copy_from_slice(&de.reader.read_bytes(0x10)?);
            self.ng_id = de.reader.read_u32()?;
            unsafe {
                self.union
                    .ng_priv
                    .data
                    .copy_from_slice(&de.reader.read_bytes(0x1e)?);
                self.union
                    .ng_priv
                    .pad1
                    .copy_from_slice(&de.reader.read_bytes(0x12)?);
            }
            self.nand_key.copy_from_slice(&de.reader.read_bytes(0x10)?);
            self.backup_key
                .copy_from_slice(&de.reader.read_bytes(0x10)?);
            self.unk1 = de.reader.read_u32()?;
            self.unk2 = de.reader.read_u32()?;
            self.eeprom_pad
                .copy_from_slice(&de.reader.read_bytes(0x80)?);
            self.ms_id = de.reader.read_u32()?;
            self.ca_id = de.reader.read_u32()?;
            self.ng_key_id = de.reader.read_u32()?;
            self.ng_sig.copy_from_slice(&de.reader.read_bytes(60)?);
            self.counters[0].boot2version = de.reader.read_u8()?;
            self.counters[0].unknown1 = de.reader.read_u8()?;
            self.counters[0].unknown2 = de.reader.read_u8()?;
            self.counters[0].pad = de.reader.read_u8()?;
            self.counters[0].update_tag = de.reader.read_u32()?;
            self.counters[0].checksum = de.reader.read_u16()?;
            self.counters[1].boot2version = de.reader.read_u8()?;
            self.counters[1].unknown1 = de.reader.read_u8()?;
            self.counters[1].unknown2 = de.reader.read_u8()?;
            self.counters[1].pad = de.reader.read_u8()?;
            self.counters[1].update_tag = de.reader.read_u32()?;
            self.counters[1].checksum = de.reader.read_u16()?;
            self.fill.copy_from_slice(&de.reader.read_bytes(24)?);
            self.korean_key
                .copy_from_slice(&de.reader.read_bytes(0x10)?);
            self.pad3.copy_from_slice(&de.reader.read_bytes(0x74)?);
            self.prng_seed[0] = de.reader.read_u16()?;
            self.prng_seed[1] = de.reader.read_u16()?;
            self.pad4.copy_from_slice(&de.reader.read_bytes(4)?);
            self.crack_pad
                .copy_from_slice(&de.reader.read_bytes(0x100)?);
            Ok(())
        }
    }

    type KeyEntries = [KeyEntry; 32];

    #[derive(Debug, Clone)]
    pub struct Iosc {
        console_type: ConsoleType,
        key_entries: KeyEntries,
        root_key_entry: KeyEntry,
        console_signature: super::Signature,
        ms_id: u32,
        ca_id: u32,
        console_key_id: u32,
    }

    impl Iosc {
        pub fn new(console_type: ConsoleType) -> Self {
            let mut this = Self {
                console_type,
                key_entries: std::array::from_fn(|_| KeyEntry::default()),
                root_key_entry: KeyEntry::default(),
                console_signature: std::array::repeat(0),
                ms_id: Default::default(),
                ca_id: Default::default(),
                console_key_id: Default::default(),
            };
            this.load_default_entries();
            #[cfg(feature = "fs")]
            this.load_entries();
            this
        }
    }

    impl Iosc {
        fn load_default_entries(&mut self) {
            // [From Dolphin-emu]
            // NOTE: IOS on real hardware supplies defaulted values if common key is not blown in OTP.
            // In some cases, the default values used differ between retail and dev builds of IOS.
            // Dolphin does not use the same "default" values as IOS does, as we do not emulate unblown
            // scenario.

            self.key_entries[DefaultHandle::HandleConsoleKey as usize] = KeyEntry::new(
                ObjectType::TypeSecretKey,
                ObjectSubType::ECC233,
                DEFAULT_PRIVATE_KEY.to_vec(),
                3,
                None,
            );
            self.console_signature = DEFAULT_SIGNATURE;
            self.console_key_id = DEFAULT_KEY_ID;
            self.key_entries[DefaultHandle::HandleConsoleId as usize] = KeyEntry::new(
                ObjectType::TypeData,
                ObjectSubType::Data,
                Vec::new(),
                0xFFFFFFF,
                Some(DEFAULT_DEVICE_ID),
            );
            self.key_entries[DefaultHandle::HandleFsKey as usize] = KeyEntry::new(
                ObjectType::TypeSecretKey,
                ObjectSubType::AES128,
                vec![0; AES128_KEY_SIZE],
                5,
                None,
            );
            self.key_entries[DefaultHandle::HandleFsMac as usize] = KeyEntry::new(
                ObjectType::TypeSecretKey,
                ObjectSubType::MAC,
                vec![0; 20],
                5,
                None,
            );
            match self.console_type {
                ConsoleType::Retail => {
                    self.key_entries[DefaultHandle::HandleCommonKey as usize] = KeyEntry::new(
                        ObjectType::TypeSecretKey,
                        ObjectSubType::AES128,
                        vec![
                            0xeb, 0xe4, 0x2a, 0x22, 0x5e, 0x85, 0x93, 0xe4, 0x48, 0xd9, 0xc5, 0x45,
                            0x73, 0x81, 0xaa, 0xf7,
                        ],
                        3,
                        None,
                    );
                    self.root_key_entry = KeyEntry::new(
                        ObjectType::TypePublicKey,
                        ObjectSubType::RSA4096,
                        ROOT_PUBLIC_KEY.to_vec(),
                        0,
                        Some(u32::swap_bytes(0x00010001)),
                    );
                    self.ms_id = 2;
                    self.ca_id = 1;
                }
                ConsoleType::RVT => {
                    self.key_entries[DefaultHandle::HandleCommonKey as usize] = KeyEntry::new(
                        ObjectType::TypeSecretKey,
                        ObjectSubType::AES128,
                        vec![
                            0xa1, 0x60, 0x4a, 0x6a, 0x71, 0x23, 0xb5, 0x29, 0xae, 0x8b, 0xec, 0x32,
                            0xc8, 0x16, 0xfc, 0xaa,
                        ],
                        3,
                        None,
                    );
                    self.root_key_entry = KeyEntry::new(
                        ObjectType::TypePublicKey,
                        ObjectSubType::RSA4096,
                        ROOT_PUBLIC_KEY_DEV.to_vec(),
                        0,
                        Some(u32::swap_bytes(0x00010001)),
                    );
                    self.ms_id = 3;
                    self.ca_id = 2;
                }
            }
            self.key_entries[DefaultHandle::HandlePrngKey as usize] = KeyEntry::new(
                ObjectType::TypeSecretKey,
                ObjectSubType::AES128,
                vec![0; 0x10],
                3,
                None,
            );
            self.key_entries[DefaultHandle::HandleSdKey as usize] = KeyEntry::new(
                ObjectType::TypeSecretKey,
                ObjectSubType::AES128,
                vec![
                    0xab, 0x01, 0xb9, 0xd8, 0xe1, 0x62, 0x2b, 0x08, 0xaf, 0xba, 0xd8, 0x4d, 0xbf,
                    0xc2, 0xa5, 0x5d,
                ],
                3,
                None,
            );
            self.key_entries[DefaultHandle::HandleBoot2Version as usize] = KeyEntry::new(
                ObjectType::TypeData,
                ObjectSubType::Version,
                Vec::new(),
                3,
                None,
            );
            self.key_entries[DefaultHandle::HandleUnknown8 as usize] = KeyEntry::new(
                ObjectType::TypeData,
                ObjectSubType::Version,
                Vec::new(),
                3,
                None,
            );
            self.key_entries[DefaultHandle::HandleUnknown9 as usize] = KeyEntry::new(
                ObjectType::TypeData,
                ObjectSubType::Version,
                Vec::new(),
                3,
                None,
            );
            self.key_entries[DefaultHandle::HandleFsVersion as usize] = KeyEntry::new(
                ObjectType::TypeData,
                ObjectSubType::Version,
                Vec::new(),
                3,
                None,
            );
            self.key_entries[DefaultHandle::HandleNewCommonKey as usize] = KeyEntry::new(
                ObjectType::TypeSecretKey,
                ObjectSubType::AES128,
                vec![
                    0x63, 0xb8, 0x2b, 0xb4, 0xf4, 0x61, 0x4e, 0x2e, 0x13, 0xf2, 0xfe, 0xfb, 0xba,
                    0x4c, 0x9b, 0x7e,
                ],
                3,
                None,
            );
        }

        #[cfg(feature = "fs")]
        fn load_entries(&mut self) {
            let mut path = std::env::var("GECKO_KEYS_PATH")
                .map(|s| ::std::path::PathBuf::from(s))
                .unwrap_or_default();
            path.push("keys.bin");
            let data = if let Ok(data) = std::fs::read(path) {
                data
            } else {
                #[cfg(feature = "log")]
                log::warn!(
                    "[Gecko Reader] keys.bin could not be found. Default values will be used."
                );
                return;
            };

            let dump: BootMiiKeyDump = if let Ok(dump) =
                serde_binary::decode(&data, serde_binary::binary_stream::Endian::Big)
            {
                dump
            } else {
                #[cfg(feature = "log")]
                log::warn!("[Gecko Reader] Failed to read from keys.bin.");
                return;
            };

            self.key_entries[DefaultHandle::HandleConsoleKey as usize].data =
                unsafe { dump.union.ng_priv }.data.to_vec();
            self.console_signature.copy_from_slice(&dump.ng_sig);
            self.ms_id = dump.ms_id;
            self.ca_id = dump.ca_id;
            self.console_key_id = dump.ng_key_id;
            self.key_entries[DefaultHandle::HandleConsoleId as usize].misc_data = dump.ng_id;
            self.key_entries[DefaultHandle::HandleFsKey as usize].data = dump.nand_key.to_vec();
            self.key_entries[DefaultHandle::HandleFsMac as usize].data =
                unsafe { dump.union.nand_hmac }.data.to_vec();
            self.key_entries[DefaultHandle::HandlePrngKey as usize].data = dump.backup_key.to_vec();
            self.key_entries[DefaultHandle::HandleBoot2Version as usize].misc_data =
                dump.counters[0].boot2version as u32;
        }

        #[allow(unused)]
        fn find_free_entry(&self) -> Option<(Handle, &KeyEntry)> {
            self.key_entries
                .iter()
                .enumerate()
                .find_map(|(handle, entry)| {
                    if !entry.in_use {
                        Some((Handle(handle as u32), entry))
                    } else {
                        None
                    }
                })
        }

        fn find_free_entry_mut(&mut self) -> Option<(Handle, &mut KeyEntry)> {
            self.key_entries
                .iter_mut()
                .enumerate()
                .find_map(|(handle, entry)| {
                    if !entry.in_use {
                        Some((Handle(handle as u32), entry))
                    } else {
                        None
                    }
                })
        }

        fn find_entry(&self, handle: Handle, search_include_root: bool) -> Option<&KeyEntry> {
            if search_include_root && handle.0 == DefaultHandle::HandleRootKey as u32 {
                return Some(&self.root_key_entry);
            }
            if (handle.0 as usize) < self.key_entries.len() {
                Some(&self.key_entries[handle.0 as usize])
            } else {
                None
            }
        }

        // Root is not mutable, so excluded by default
        fn find_entry_mut(&mut self, handle: Handle) -> Option<&mut KeyEntry> {
            if (handle.0 as usize) < self.key_entries.len() {
                Some(&mut self.key_entries[handle.0 as usize])
            } else {
                None
            }
        }

        fn has_ownership(&self, handle: Handle, pid: u32) -> bool {
            handle.0 == DefaultHandle::HandleRootKey as u32
                || (self
                    .get_ownership(handle)
                    .is_ok_and(|owner_mask| (1u32.unbounded_shl(pid) & owner_mask) != 0))
        }

        fn is_default_handle(&self, handle: Handle) -> bool {
            const LAST_DEFAULT_HANDLE: DefaultHandle = DefaultHandle::HandleNewCommonKey;
            handle.0 <= LAST_DEFAULT_HANDLE as u32
                || handle.0 == DefaultHandle::HandleRootKey as u32
        }

        fn decrypt_encrypt(
            &self,
            is_encrypt: bool,
            key_handle: Handle,
            iv: &[u8],
            input: &[u8],
            pid: u32,
        ) -> Result<Vec<u8>, IoscError> {
            if !self.has_ownership(key_handle, pid) {
                return Err(IoscError::IoscEAccess);
            }

            let entry = self
                .find_entry(key_handle, false)
                .ok_or(IoscError::IoscEInval)?;
            if entry.key_type != ObjectType::TypeSecretKey
                || entry.sub_type != ObjectSubType::AES128
            {
                return Err(IoscError::IoscInvalidObjtype);
            }
            if entry.data.len() != AES128_KEY_SIZE {
                return Err(IoscError::IoscFailInteral);
            }

            let mut out = input.to_vec();
            let mut key = [0u8; AES128_KEY_SIZE];
            let mut iv_ = [0u8; AES128_KEY_SIZE];
            key.copy_from_slice(&entry.data);
            iv_.copy_from_slice(iv);
            if is_encrypt {
                aes_encrypt_inplace(&mut out, &iv_, &key);
            } else {
                aes_decrypt_inplace(&mut out, &iv_, &key);
            }
            Ok(out)
        }
    }

    #[derive(Debug, Clone, Copy)]
    #[repr(i32)]
    pub enum IoscError {
        IoscEAccess = -2000,
        IoscEInval = -2002,
        IoscInvalidObjtype = -2005,
        IoscFailCheckvalue = -2011,
        IoscFailInteral = -2012,
        IoscFailAlloc = -2013,
    }

    impl std::error::Error for IoscError {}

    impl std::fmt::Display for IoscError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            match self {
                IoscError::IoscEAccess => {
                    write!(f, "IOSC_EACCESS ; Access to a resource is denied")
                }
                IoscError::IoscEInval => write!(f, "IOSC_EINVAL ; Invalid value"),
                IoscError::IoscFailAlloc => write!(f, "IOSC_FAIL_ALLOC ; Couldn't allocate object"),
                IoscError::IoscFailCheckvalue => {
                    write!(f, "IOSC_FAIL_CHECKVALUE ; Couldn't validate signature key")
                }
                IoscError::IoscFailInteral => {
                    write!(f, "IOSC_FAIL_INTERNAL ; IV has the wrong size")
                }
                IoscError::IoscInvalidObjtype => {
                    write!(f, "IOSC_INVALID_OBJTYPE ; Object type is not valid")
                }
            }
        }
    }

    fn make_blank_ecc_cert<S: AsRef<::core::ffi::CStr>>(
        issuer: &S,
        name: &S,
        private_key: &[u8],
        key_id: u32,
    ) -> CertECC {
        let mut cert = CertECC::default();
        cert.signature.sig_type = SignatureType::ECC;
        let issuer_len =
            ::core::cmp::min(cert.signature.issuer.len(), issuer.as_ref().count_bytes());
        cert.signature.issuer[..issuer_len]
            .copy_from_slice(&issuer.as_ref().to_bytes()[..issuer_len]);
        cert.header.public_key_type = super::PublicKeyType::ECC;
        let name_len = ::core::cmp::min(cert.header.name.len(), name.as_ref().count_bytes());
        cert.header.name[..name_len].copy_from_slice(&name.as_ref().to_bytes()[..name_len]);
        cert.header.id = u32::swap_bytes(key_id);
        let mut private_key_buf = [0; _];
        private_key_buf.copy_from_slice(&private_key);
        cert.public_key = ECCPublicKey(crypto::ec::priv_to_pub(private_key_buf));
        cert
    }

    impl Iosc {
        #[doc = "Create an object for use with the other functions that operate on objects."]
        pub fn create_object(
            &mut self,
            type_: ObjectType,
            sub_type: ObjectSubType,
            pid: u32,
        ) -> Result<Handle, IoscError> {
            let (handle, key_entry) = self.find_free_entry_mut().ok_or(IoscError::IoscFailAlloc)?;
            key_entry.in_use = true;
            key_entry.key_type = type_;
            key_entry.sub_type = sub_type;
            key_entry.owner_mask = 1 << pid;

            Ok(handle)
        }

        #[doc = "Delete an object. Built-in objects cannot be deleted."]
        pub fn delete_object(&mut self, handle: Handle, pid: u32) -> Result<(), IoscError> {
            if self.is_default_handle(handle) || self.has_ownership(handle, pid) {
                return Err(IoscError::IoscEInval);
            }

            let key_entry = self.find_entry_mut(handle).ok_or(IoscError::IoscEInval)?;
            key_entry.in_use = false;
            key_entry.data.clear();
            Ok(())
        }

        #[doc = "Import a secret, encrypted key in to dest_handle. If decrypt_handle_iv is provided, decrypt the key using decrypt_handle and the iv."]
        pub fn import_secret_key(
            &mut self,
            dest_handle: Handle,
            decrypt_handle_iv: Option<(Handle, &[u8])>,
            key: &[u8],
            pid: u32,
        ) -> Result<(), IoscError> {
            let key = match decrypt_handle_iv {
                Some((decrypt_handle, iv)) => self.decrypt(decrypt_handle, iv, key, pid)?,
                None => key.to_vec(),
            };

            if !self.has_ownership(dest_handle, pid) || self.is_default_handle(dest_handle) {
                return Err(IoscError::IoscEAccess);
            }

            let dest_entry = self
                .find_entry_mut(dest_handle)
                .ok_or(IoscError::IoscEInval)?;
            if dest_entry.key_type != ObjectType::TypeSecretKey
                || dest_entry.sub_type != ObjectSubType::AES128
            {
                return Err(IoscError::IoscInvalidObjtype);
            }

            dest_entry.data.clear();
            dest_entry.data.extend_from_slice(&key);
            Ok(())
        }

        #[doc = "Import a public key. public_key_exponent must be passed for RSA keys."]
        pub fn import_public_key(
            &mut self,
            dest_handle: Handle,
            public_key: &[u8],
            public_key_exponent: Option<&[u8]>,
            pid: u32,
        ) -> Result<(), IoscError> {
            if !self.has_ownership(dest_handle, pid) || self.is_default_handle(dest_handle) {
                return Err(IoscError::IoscEAccess);
            }

            let dest_entry = self
                .find_entry_mut(dest_handle)
                .ok_or(IoscError::IoscEInval)?;
            if dest_entry.key_type != ObjectType::TypePublicKey {
                return Err(IoscError::IoscInvalidObjtype);
            }

            let size =
                if let Some(size) = get_size_for_type(dest_entry.key_type, dest_entry.sub_type) {
                    size
                } else {
                    return Err(IoscError::IoscInvalidObjtype);
                };

            dest_entry.data.clear();
            dest_entry.data.extend_from_slice(&public_key[..size]);

            if dest_entry.sub_type == ObjectSubType::RSA2048
                || dest_entry.sub_type == ObjectSubType::RSA4096
            {
                dest_entry.misc_data = byteorder::BE::read_u32(&public_key_exponent.expect("Public key exponent should be provided for Object sub types RSA2048 and RSA4096"));
            }

            Ok(())
        }

        #[doc = "Compute an AES key from an ECDH shared secret."]
        pub fn compute_shared_key(
            &mut self,
            dest_handle: Handle,
            private_handle: Handle,
            public_handle: Handle,
            pid: u32,
        ) -> Result<(), IoscError> {
            use sha1::digest::Digest;
            if !self.has_ownership(dest_handle, pid)
                || !self.has_ownership(private_handle, pid)
                || !self.has_ownership(public_handle, pid)
                || !self.is_default_handle(dest_handle)
            {
                return Err(IoscError::IoscEAccess);
            }

            let private_entry = self
                .find_entry(private_handle, false)
                .ok_or(IoscError::IoscEInval)?;
            let public_entry = self
                .find_entry(public_handle, false)
                .ok_or(IoscError::IoscEInval)?;
            if private_entry.key_type != ObjectType::TypeSecretKey
                || private_entry.sub_type != ObjectSubType::ECC233
                || public_entry.key_type != ObjectType::TypePublicKey
                || public_entry.sub_type != ObjectSubType::ECC233
            {
                return Err(IoscError::IoscInvalidObjtype);
            }

            let mut private_key = [0u8; 30];
            let mut public_key = [0u8; 60];
            private_key.copy_from_slice(&private_entry.data);
            public_key.copy_from_slice(&public_entry.data);
            let shared_secret = crypto::ec::compute_shared_secret(private_key, public_key);

            let sha1: [u8; _] = sha1::Sha1::digest(&shared_secret[..30]).into();

            let dest_entry = self
                .find_entry_mut(dest_handle)
                .ok_or(IoscError::IoscEInval)?;
            if dest_entry.key_type != ObjectType::TypeSecretKey
                || dest_entry.sub_type != ObjectSubType::AES128
            {
                return Err(IoscError::IoscInvalidObjtype);
            }
            dest_entry.data.resize(size_of_val(&sha1), 0);
            dest_entry.data.copy_from_slice(&sha1);
            Ok(())
        }

        #[doc = "AES encrypt."]
        pub fn encrypt(
            &self,
            key_handle: Handle,
            iv: &[u8],
            input: &[u8],
            pid: u32,
        ) -> Result<Vec<u8>, IoscError> {
            self.decrypt_encrypt(false, key_handle, iv, input, pid)
        }

        #[doc = "AES decrypt."]
        pub fn decrypt(
            &self,
            key_handle: Handle,
            iv: &[u8],
            input: &[u8],
            pid: u32,
        ) -> Result<Vec<u8>, IoscError> {
            self.decrypt_encrypt(true, key_handle, iv, input, pid)
        }

        pub fn verify_public_key_sign(
            &self,
            sha1: [u8; 20],
            signer_handle: Handle,
            signature: &[u8],
            pid: u32,
        ) -> Result<(), IoscError> {
            if !self.has_ownership(signer_handle, pid) {
                return Err(IoscError::IoscEAccess);
            }

            let entry = self
                .find_entry(signer_handle, true)
                .ok_or(IoscError::IoscEInval)?;
            if entry.key_type != ObjectType::TypePublicKey {
                return Err(IoscError::IoscInvalidObjtype);
            }

            match entry.sub_type {
                ObjectSubType::RSA4096 | ObjectSubType::RSA2048 => {
                    use rsa::signature::Verifier;
                    let expected_key_size: usize = if entry.sub_type == ObjectSubType::RSA2048 {
                        0x100
                    } else {
                        0x200
                    };
                    assert_eq!(entry.data.len(), expected_key_size);
                    assert_eq!(signature.len(), expected_key_size);

                    let rsa_public_key = rsa::RsaPublicKey::new(
                        rsa::BigUint::from_bytes_le(&entry.data),
                        rsa::BigUint::from_slice(&[entry.misc_data]),
                    )
                    .or(Err(IoscError::IoscFailCheckvalue))?;
                    let verifying_key =
                        rsa::pkcs1v15::VerifyingKey::<sha1::Sha1>::new(rsa_public_key);

                    // TODO Instead of directly returning an IOSC_FAIL_CHECKVALUE, apply a powmod and check that it ends with digest.
                    rsa::pkcs1v15::Signature::try_from(signature)
                        .and_then(|signature| verifying_key.verify(&sha1, &signature))
                        .or(Err(IoscError::IoscFailCheckvalue))?;

                    Ok(())
                }
                ObjectSubType::ECC233 => {
                    assert_eq!(entry.data.len(), ::core::mem::size_of::<ECCPublicKey>());
                    assert_eq!(signature.len(), ::core::mem::size_of::<Signature>());
                    let mut key = [0; _];
                    let mut sign = [0; _];
                    let mut hash = [0; _];
                    key.copy_from_slice(&entry.data);
                    sign.copy_from_slice(&signature);
                    hash.copy_from_slice(&sha1);
                    match crypto::ec::verify_signature(key, sign, hash) {
                        true => Ok(()),
                        false => Err(IoscError::IoscFailCheckvalue),
                    }
                }
                _ => Err(IoscError::IoscInvalidObjtype),
            }
        }

        pub fn import_certificate(
            &mut self,
            cert: &CertReader,
            signer_handle: Handle,
            dest_handle: Handle,
            pid: u32,
        ) -> Result<(), IoscError> {
            if !self.has_ownership(signer_handle, pid) || !self.has_ownership(dest_handle, pid) {
                return Err(IoscError::IoscEAccess);
            }

            let signer_entry = self
                .find_entry(signer_handle, true)
                .ok_or(IoscError::IoscEInval)?;
            let dest_entry = self
                .find_entry(dest_handle, true)
                .ok_or(IoscError::IoscEInval)?;
            if signer_entry.key_type != ObjectType::TypePublicKey
                || dest_entry.key_type != ObjectType::TypePublicKey
            {
                return Err(IoscError::IoscInvalidObjtype);
            }

            if let Some(public_key) = cert.get_public_key() {
                let exponent = if let Some(SignatureType::ECC) = cert.get_signature_type() {
                    Some(&public_key[public_key.len() - 4..])
                } else {
                    None
                };
                self.import_public_key(dest_handle, &public_key, exponent, pid)
            } else {
                return Err(IoscError::IoscEInval);
            }
        }

        pub fn get_ownership(&self, handle: Handle) -> Result<u32, IoscError> {
            let entry = self
                .find_entry(handle, false)
                .and_then(|entry| if entry.in_use { Some(entry) } else { None })
                .ok_or(IoscError::IoscEInval)?;
            Ok(entry.owner_mask)
        }

        pub fn set_ownership(
            &mut self,
            handle: Handle,
            owner: u32,
            pid: u32,
        ) -> Result<(), IoscError> {
            if !self.has_ownership(handle, pid) {
                return Err(IoscError::IoscEAccess);
            }

            let entry = self.find_entry_mut(handle).ok_or(IoscError::IoscEInval)?;
            let mask_with_current_pid = 1u32.unbounded_shl(pid);
            let mask = entry.owner_mask | mask_with_current_pid;
            if mask != mask_with_current_pid {
                return Err(IoscError::IoscEAccess);
            }
            entry.owner_mask = (owner & !7) | mask;
            Ok(())
        }

        pub fn is_using_default_id(&self) -> bool {
            self.get_device_id() == DEFAULT_DEVICE_ID
        }

        pub fn get_device_id(&self) -> u32 {
            self.key_entries[DefaultHandle::HandleConsoleId as usize].misc_data
        }

        pub fn get_device_certificate(&self) -> CertECC {
            let name: CString = CString::new(format!("NG{:08x}", self.get_device_id()))
                .unwrap_or(c"NG00000000".into());
            let (ca_id, ms_id) = (self.ca_id, self.ms_id);
            let mut cert = make_blank_ecc_cert(
                &CString::new(format!("Root-CA{ca_id:08x}-MS{ms_id:08x}"))
                    .unwrap_or(c"Root-CA00000000-MS00000000".into()),
                &name,
                &self.key_entries[DefaultHandle::HandleConsoleKey as usize].data,
                self.console_key_id,
            );
            cert.signature.sig = self.console_signature;
            cert
        }

        pub fn sign(&self, title_id: u64, data: &[u8]) -> ([u8; ::core::mem::size_of::<CertECC>()], Signature) {
            let mut ap_priv = [0u8; 30];
            ap_priv[0x1d] = 1;
            // setup random ap_priv here if desired
            // rand::fill(&mut ap_priv);
            // ap_priv[0x1d] = 1;

            let signer = CString::new(format!(
                "Root-CA{:08x}-MS{:08x}-NG{:08x}",
                self.ca_id,
                self.ms_id,
                self.get_device_id()
            ))
            .expect(&format!("Signer should be formattable in all cases. [CA = {:?}; MS = {:?}; NG = {:?}]", self.ca_id, self.ms_id, self.get_device_id()));
            let name = CString::new(format!("AP{:016x}", title_id))
                .expect(&format!("Certification header's name should be formattable in all cases. [title_id = {:?}]", title_id));
            let mut cert = make_blank_ecc_cert(&signer, &name, &ap_priv, 0);
            // Sign AP cert.
            const SKIP: usize = ::core::mem::offset_of!(CertECC, signature.issuer);
            const CERT_LEN: usize = ::core::mem::size_of::<CertECC>() - SKIP;
            let ap_cert_digest: [u8; _] = unsafe {
                sha1::Sha1::digest(&::core::mem::transmute_copy::<CertECC, 
                    [u8; ::core::mem::size_of::<CertECC>()]>(&cert)[SKIP..][..CERT_LEN])
                .into()
            };
            let mut console_key = [0; 30];
            console_key.copy_from_slice(
                &self.key_entries[DefaultHandle::HandleConsoleKey as usize].data[..30],
            );
            cert.signature.sig = crypto::ec::sign(console_key, ap_cert_digest);
            let ap_cert_out = unsafe {
                ::core::mem::transmute_copy::<CertECC, [u8; ::core::mem::size_of::<CertECC>()]>(&cert)
            };
            // Sign the data.
            let data_digest: [u8; _] = sha1::Sha1::digest(data).into();
            let signature = crypto::ec::sign(ap_priv, data_digest);
            (ap_cert_out, signature)
        }
    }

    #[cfg(test)]
    mod test {
        use crate::wii::crypto::titles;

        use super::*;

        #[test]
        fn default_private_public_key_are_coherent() {
            let mut private_key = [0u8; 30];
            private_key.copy_from_slice(&DEFAULT_PRIVATE_KEY);
            let public_key = crypto::ec::priv_to_pub(private_key);
            assert_eq!(public_key, DEFAULT_PUBLIC_KEY);
        }

        #[test]
        fn check_iosc_sign() {
            const DATA: [u8; 20] = [89, 88, 252, 243, 48, 242, 210, 85, 215, 137, 104, 205, 76, 41, 234, 35, 95, 121, 87, 77];
            let iosc = Iosc::new(ConsoleType::Retail);
            let (cert, signature) = iosc.sign(titles::SYSTEM_MENU, &DATA);
            println!("Cert: {:?}; Signature: {:?}", unsafe {::core::mem::transmute::<[u8; _], CertECC>(cert)}, signature);
        }
    }
}
