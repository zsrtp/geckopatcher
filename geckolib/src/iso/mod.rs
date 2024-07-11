use crate::crypto::Unpackable;

pub mod builder;
pub mod disc;
pub mod read;
pub mod write;

pub mod consts {
    // DOL_ALIGNMENT and FST_ALIGNMENT are set to 1024 and 256 to match the
    // original ISO. Due to poor documentation of how, and why, these values
    // should or shouldn't be changed we opted to preserve their values since
    // there was no observed benefit of setting them higher, however lower
    // values were not tested.

    pub const OFFSET_DOL_OFFSET: usize = 0x420;
    pub const OFFSET_FST_OFFSET: usize = 0x424;
    pub const OFFSET_FST_SIZE: usize = 0x428;
    pub const OFFSET_GC_MAGIC: usize = 0x01C;
    pub const OFFSET_WII_MAGIC: usize = 0x018;
    pub const GC_MAGIC: u32 = 0xC2339F3D;
    pub const WII_MAGIC: u32 = 0x5D1C9EA3;
    pub const HEADER_LENGTH: usize = 0x2440;
    pub const DOL_ALIGNMENT_BIT: usize = 10;
    pub const DOL_ALIGNMENT: usize = 1 << DOL_ALIGNMENT_BIT;
    pub const FST_ALIGNMENT_BIT: usize = 8;
    pub const FST_ALIGNMENT: usize = 1 << FST_ALIGNMENT_BIT;
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Default)]
#[repr(u8)]
pub(crate) enum FstNodeType {
    #[default]
    File = 0,
    Directory = 1,
}

impl TryFrom<u8> for FstNodeType {
    type Error = eyre::ErrReport;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(FstNodeType::File),
            1 => Ok(FstNodeType::Directory),
            n => Err(eyre::eyre!(
                "Out of range value for FstNodeType [value = {}]",
                n
            )),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct FstEntry {
    pub(crate) kind: FstNodeType,
    // Name of the file
    pub(crate) relative_file_name: String,
    // File offset for a File, Parent dir for a Directory
    pub(crate) file_offset_parent_dir: usize,
    // File size for a File, next directory for a Directory
    pub(crate) file_size_next_dir_index: usize,
    pub(crate) file_name_offset: usize,
}

impl Unpackable for FstEntry {
    const BLOCK_SIZE: usize = 0xC;
}
