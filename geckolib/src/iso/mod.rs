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
    pub const DOL_ALIGNMENT_BIT: usize = 8;
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
#[repr(C)]
pub(crate) struct FstEntry {
    node_type_file_name_offset: u32,
    file_offset_parent_dir: u32,
    file_size_next_dir_index: u32,
}

impl Unpackable for FstEntry {
    const BLOCK_SIZE: usize = 0xC;
}

impl TryFrom<&[u8]> for FstEntry {
    type Error = eyre::ErrReport;

    fn try_from(value: &[u8]) -> Result<Self, Self::Error> {
        if value.len() < Self::BLOCK_SIZE {
            return Err(eyre::eyre!(
                "Invalid length for FstNode [length = 0x{:X}; expected = 0x{:X}]",
                value.len(),
                Self::BLOCK_SIZE
            ));
        }
        let node_type_file_name_offset = u32::from_be_bytes(value[0..4].try_into()?);
        // Check if the node type is valid
        FstNodeType::try_from((node_type_file_name_offset >> 24) as u8)?;
        let file_offset_parent_dir = u32::from_be_bytes(value[4..8].try_into()?);
        let file_size_next_dir_index = u32::from_be_bytes(value[8..12].try_into()?);

        Ok(Self {
            node_type_file_name_offset,
            file_offset_parent_dir,
            file_size_next_dir_index,
        })
    }
}

impl FstEntry {
    pub(crate) fn pack(&self) -> [u8; Self::BLOCK_SIZE] {
        let mut buf = [0u8; Self::BLOCK_SIZE];
        buf[0..4].copy_from_slice(&self.node_type_file_name_offset.to_be_bytes());
        buf[4..8].copy_from_slice(&self.file_offset_parent_dir.to_be_bytes());
        buf[8..12].copy_from_slice(&self.file_size_next_dir_index.to_be_bytes());
        buf
    }

    pub(crate) fn get_node_type(&self) -> FstNodeType {
        FstNodeType::try_from((self.node_type_file_name_offset >> 24) as u8).unwrap()
    }

    pub(crate) fn set_node_type(&mut self, node_type: FstNodeType) {
        self.node_type_file_name_offset = (self.node_type_file_name_offset & 0x00FFFFFF)
            | ((node_type as u32) << 24);
    }

    pub(crate) fn get_file_name_offset(&self) -> u32 {
        self.node_type_file_name_offset & 0x00FFFFFF
    }

    pub(crate) fn set_file_name_offset(&mut self, file_name_offset: u32) {
        self.node_type_file_name_offset = (self.node_type_file_name_offset & 0xFF000000)
            | (file_name_offset & 0x00FFFFFF);
    }

    pub(crate) fn get_file_offset(&self, is_wii: bool) -> u64 {
        (self.file_offset_parent_dir as u64) << if is_wii { 2 } else { 0 }
    }

    pub(crate) fn set_file_offset_parent_dir(&mut self, file_offset_parent_dir: u64, is_wii: bool) -> eyre::Result<()> {
        if file_offset_parent_dir > ((u32::MAX as u64) << 2) as u64 {
            return Err(eyre::eyre!(
                "File offset is too large [offset = 0x{:X}; max = 0x{:X}]",
                file_offset_parent_dir,
                u32::MAX
            ));
        }
        self.file_offset_parent_dir = (file_offset_parent_dir >> if is_wii { 2 } else { 0 }) as u32;
        Ok(())
    }

    pub(crate) fn get_file_size_next_dir_index(&self) -> u32 {
        self.file_size_next_dir_index
    }

    pub(crate) fn set_file_size_next_dir_index(&mut self, file_size_next_dir_index: u32) {
        self.file_size_next_dir_index = file_size_next_dir_index;
    }

    pub(crate) fn new_file(file_name_offset: u32, file_offset: u64, file_size: u32, is_wii: bool) -> eyre::Result<Self> {
        let mut node = Self::default();
        node.set_node_type(FstNodeType::File);
        node.set_file_name_offset(file_name_offset);
        node.set_file_offset_parent_dir(file_offset, is_wii)?;
        node.set_file_size_next_dir_index(file_size);
        Ok(node)
    }

    pub(crate) fn new_directory(file_name_offset: u32, parent_dir: u64, next_dir_index: u32, is_wii: bool) -> eyre::Result<Self> {
        let mut node = Self::default();
        node.set_node_type(FstNodeType::Directory);
        node.set_file_name_offset(file_name_offset);
        node.set_file_offset_parent_dir(parent_dir, is_wii)?;
        node.set_file_size_next_dir_index(next_dir_index);
        Ok(node)
    }
}

#[derive(Debug, Clone)]
pub(crate) enum FstNode {
    File {
        // Name of the file
        relative_file_name: String,
        // File offset
        file_offset: u64,
        // File size
        file_size: usize,
    },
    Directory {
        // Name of the file
        relative_file_name: String,
        // Parent directory
        parent_dir: u64,
        // Next directory index
        next_dir_index: usize,
    },
}

impl Default for FstNode {
    fn default() -> Self {
        Self::File {
            relative_file_name: String::new(),
            file_offset: 0,
            file_size: 0,
        }
    }
}

impl FstNode {
    pub(crate) fn from_fstnode(node: &FstEntry, file_name_table: &[u8]) -> eyre::Result<Self> {
        // Get the file name
        let pos = (node.node_type_file_name_offset & 0x00FFFFFF) as usize;
        let mut end = pos;
        while file_name_table[end] != 0 {
            end += 1;
        }
        let mut str_buf = Vec::new();
        str_buf.extend_from_slice(&file_name_table[pos..end]);
        let relative_file_name = String::from_utf8(str_buf)?;
        match FstNodeType::try_from((node.node_type_file_name_offset >> 24) as u8)? {
            FstNodeType::File => {
                let file_offset = node.file_offset_parent_dir as u64;
                let file_size = node.file_size_next_dir_index as usize;
                Ok(Self::File {
                    relative_file_name,
                    file_offset,
                    file_size,
                })
            }
            FstNodeType::Directory => {
                let parent_dir = node.file_offset_parent_dir as u64;
                let next_dir_index = node.file_size_next_dir_index as usize;
                Ok(Self::Directory {
                    relative_file_name,
                    parent_dir,
                    next_dir_index,
                })
            }
        }
    }

    pub(crate) fn get_relative_file_name(&self) -> &str {
        match self {
            Self::File { relative_file_name, .. } => relative_file_name,
            Self::Directory { relative_file_name, .. } => relative_file_name,
        }
    }

    pub(crate) fn get_file_offset(&self) -> Option<u64> {
        match self {
            Self::File { file_offset, .. } => Some(*file_offset),
            _ => None,
        }
    }

    pub(crate) fn get_file_size(&self) -> Option<usize> {
        match self {
            Self::File { file_size, .. } => Some(*file_size),
            _ => None,
        }
    }

    pub(crate) fn get_parent_dir(&self) -> Option<u64> {
        match self {
            Self::Directory { parent_dir, .. } => Some(*parent_dir),
            _ => None,
        }
    }

    pub(crate) fn get_next_dir_index(&self) -> Option<usize> {
        match self {
            Self::Directory { next_dir_index, .. } => Some(*next_dir_index),
            _ => None,
        }
    }

    pub(crate) fn is_file(&self) -> bool {
        matches!(self, Self::File { .. })
    }

    pub(crate) fn is_directory(&self) -> bool {
        matches!(self, Self::Directory { .. })
    }

    pub(crate) fn get_relative_file_name_mut(&mut self) -> &mut String {
        match self {
            Self::File {
                relative_file_name, ..
            } => relative_file_name,
            Self::Directory {
                relative_file_name, ..
            } => relative_file_name,
        }
    }

    pub(crate) fn get_file_offset_mut(&mut self) -> Option<&mut u64> {
        match self {
            Self::File { file_offset, .. } => Some(file_offset),
            _ => None,
        }
    }

    pub(crate) fn get_file_size_mut(&mut self) -> Option<&mut usize> {
        match self {
            Self::File { file_size, .. } => Some(file_size),
            _ => None,
        }
    }

    pub(crate) fn get_parent_dir_mut(&mut self) -> Option<&mut u64> {
        match self {
            Self::Directory { parent_dir, .. } => Some(parent_dir),
            _ => None,
        }
    }

    pub(crate) fn get_next_dir_index_mut(&mut self) -> Option<&mut usize> {
        match self {
            Self::Directory { next_dir_index, .. } => Some(next_dir_index),
            _ => None,
        }
    }
}
