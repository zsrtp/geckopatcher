pub mod read;
pub mod write;
pub mod crypto;
pub mod disc;
pub(crate) mod keys;

pub mod consts {
    // DOL_ALIGNMENT and FST_ALIGNMENT are set to 1024 and 256 to match the
    // original ISO. Due to poor documentation of how, and why, these values
    // should or shouldn't be changed we opted to preserve their values since
    // there was no observed benefit of setting them higher, however lower
    // values were not tested.

    pub const WII_HASH_SIZE: usize = 160 / 8; // 160 bits with 8 bits per byte
    pub const WII_KEY_SIZE: usize = 16;
    pub const WII_CKEY_AMNT: usize = 3;
    pub const WII_H3_OFFSET: u64 = 0x8000;
    pub const WII_H3_SIZE: u64 = 0x18000;
    pub const WII_HX_OFFSETS: [usize; 3] = [0, 0x280, 0x340];
    pub const WII_SECTOR_HASH_SIZE: usize = 0x400;
    pub const WII_SECTOR_SIZE: usize = 0x8000;
    pub const WII_SECTOR_DATA_SIZE: u64 = (WII_SECTOR_SIZE - WII_SECTOR_HASH_SIZE) as u64;
    pub const WII_SECTOR_IV_OFF: u64 = 0x3D0;
    pub const WII_PARTITION_INFO_OFFSET: u64 = 0x40000;
    pub const WII_SECTOR_DATA_HASH_SIZE: u64 = 0x400; // Size of chunks of data hashed together for h0
    /// Number of chunks of data which are hashed for h0.
    pub const WII_SECTOR_DATA_HASH_COUNT: u64 = WII_SECTOR_DATA_SIZE / WII_SECTOR_DATA_HASH_SIZE;
}
