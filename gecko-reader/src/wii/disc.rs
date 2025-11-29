#[derive(Debug, Clone, Copy, Default)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
pub struct WiiDiscRegionsAgeRating {
    pub jp: u8,
    pub us: u8,
    pub unknown1: u8,
    pub de: u8,
    pub pegi: u8,
    pub fi: u8,
    pub pt: u8,
    pub gb: u8,
    pub au: u8,
    pub kr: u8,
}

#[derive(Debug, Clone, Copy, Default)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
pub enum Region {
    NTSCJ = 0,   // Japan and Taiwan (and South Korea for Gamecube only)
    NTSCU,   // Mainly North America
    PAL,     // Mainly Europe and Oceania
    #[default]
    Unknown, // Nintendo uses this to mean region free, but we also use it for unknown regions
    KOR,     // South Korea (Wii only)
}

#[derive(Debug, Clone, Copy, Default)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
pub enum Language {
    #[default]
    Japanese = 0,
    English = 1,
    German = 2,
    Frenc = 3,
    Spanish = 4,
    Italian = 5,
    Dutch = 6,
    SimplifiedChinese = 7,   // Not Selectable on any unmodded retail Wii
    TraditionalChinese = 8,  // Not Selectable on any unmodded retail Wii
    Korean = 9,
    Unknown,
}

impl From<u32> for Region {
    fn from(value: u32) -> Self {
        Self::from(&value)
    }
}

impl From<&u32> for Region {
    fn from(value: &u32) -> Self {
        match value {
            0 => Region::NTSCJ,
            1 => Region::NTSCU,
            2 => Region::PAL,
            4 => Region::KOR,
            _ => Region::Unknown,
        }
    }
}

impl From<Region> for u32 {
    fn from(value: Region) -> Self {
        Self::from(&value)
    }
}

impl From<&Region> for u32 {
    fn from(value: &Region) -> Self {
        match value {
            Region::NTSCJ => 0,
            Region::NTSCU => 1,
            Region::PAL => 2,
            Region::Unknown => 3,
            Region::KOR => 4,
        }
    }
}

impl From<u16> for Region {
    fn from(value: u16) -> Self {
        Self::from(&value)
    }
}

impl From<&u16> for Region {
    fn from(value: &u16) -> Self {
        Self::from(*value as u32)
    }
}

impl From<Region> for u16 {
    fn from(value: Region) -> Self {
        Self::from(&value)
    }
}

impl From<&Region> for u16 {
    fn from(value: &Region) -> Self {
        u32::from(value) as u16
    }
}

#[derive(Debug, Clone, Copy, Default)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
pub struct WiiDiscRegion {
    pub region: Region,
    pub age_rating: WiiDiscRegionsAgeRating,
}

// Add implementation for serde_binary::Encode for WiiDiscRegion
impl serde_binary::Encode for WiiDiscRegion {
    fn encode(&self, ser: &mut serde_binary::Serializer) -> serde_binary::Result<()> {
        ser.writer.write_u32(&self.region.into())?;
        // Write the padding of 12 bytes
        ser.writer.write_bytes([0u8; 12])?;
        ser.writer.write_u8(self.age_rating.jp)?;
        ser.writer.write_u8(self.age_rating.us)?;
        ser.writer.write_u8(self.age_rating.unknown1)?;
        ser.writer.write_u8(self.age_rating.de)?;
        ser.writer.write_u8(self.age_rating.pegi)?;
        ser.writer.write_u8(self.age_rating.fi)?;
        ser.writer.write_u8(self.age_rating.pt)?;
        ser.writer.write_u8(self.age_rating.gb)?;
        ser.writer.write_u8(self.age_rating.au)?;
        ser.writer.write_u8(self.age_rating.kr)?;
        // Write the remaining 6 bytes
        ser.writer.write_bytes([0u8; 6])?;
        Ok(())
    }
}

impl serde_binary::Decode for WiiDiscRegion {
    fn decode(&mut self, de: &mut serde_binary::Deserializer) -> serde_binary::Result<()> {
        self.region.clone_from(&de.reader.read_u32()?.into());
        // Skip padding of 12 bytes
        let _ = de.reader.read_bytes(12)?;
        self.age_rating.jp = de.reader.read_u8()?;
        self.age_rating.us = de.reader.read_u8()?;
        self.age_rating.unknown1 = de.reader.read_u8()?;
        self.age_rating.de = de.reader.read_u8()?;
        self.age_rating.pegi = de.reader.read_u8()?;
        self.age_rating.fi = de.reader.read_u8()?;
        self.age_rating.pt = de.reader.read_u8()?;
        self.age_rating.gb = de.reader.read_u8()?;
        self.age_rating.au = de.reader.read_u8()?;
        self.age_rating.kr = de.reader.read_u8()?;
        // Skip remaining 6 bytes
        let _ = de.reader.read_bytes(6)?;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, Default)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
pub struct PartitionInfoEntry {
    pub partition_type: u32,
    pub offset: u64,
}

#[derive(Debug, Clone, Default)]
pub struct PartitionInfo {
    pub offset: u64,
    pub entries: Vec<PartitionInfoEntry>,
}