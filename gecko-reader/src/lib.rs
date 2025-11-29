pub mod wii;
pub mod disc;
pub mod read;
pub mod write;
pub mod endian;
pub mod vfs;
pub mod iosc;

pub(crate) mod utils;

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

#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {
    }
}
