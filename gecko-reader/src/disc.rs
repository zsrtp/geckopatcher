use crate::endian;

#[derive(Debug, Clone)]
pub struct DiscHeader {
    pub disc_id: u8,
    pub game_code: u16,
    pub region_code: u8,
    pub maker_code: u16,
    pub disc_number: u8,
    pub disc_version: u8,
    pub audio_streaming: bool,
    pub unk: [u8; 14],
    pub streaming_buffer_size: u8,
    pub is_wii: bool,
    pub game_title: [u8; 0x40],
    pub disable_hash_verification: bool,
    pub disable_disc_encryption: bool,
}

impl Default for DiscHeader {
    fn default() -> Self {
        Self {
            // Defaults to 'G' for a gamecube game
            disc_id: b'G',
            game_code: Default::default(),
            // Defaults to 'J' for Japan
            region_code: b'J',
            maker_code: Default::default(),
            disc_number: Default::default(),
            disc_version: Default::default(),
            audio_streaming: Default::default(),
            streaming_buffer_size: Default::default(),
            unk: Default::default(),
            is_wii: Default::default(),
            game_title: [0; _],
            disable_hash_verification: Default::default(),
            disable_disc_encryption: Default::default(),
        }
    }
}

impl serde_binary::Encode for DiscHeader {
    fn encode(&self, ser: &mut serde_binary::Serializer) -> serde_binary::Result<()> {
        ser.writer.write_u8(self.disc_id)?;
        ser.writer.write_u16(self.game_code)?;
        ser.writer.write_u8(self.region_code)?;
        ser.writer.write_u16(self.maker_code)?;
        ser.writer.write_u8(self.disc_number)?;
        ser.writer.write_u8(self.disc_version)?;
        ser.writer.write_bool(self.audio_streaming)?;
        ser.writer.write_u8(self.streaming_buffer_size)?;
        ser.writer.write_bytes(self.unk)?;
        ser.writer.write_u32(match self.is_wii {
            true => crate::consts::WII_MAGIC,
            false => 0,
        })?;
        ser.writer.write_u32(match self.is_wii {
            true => 0,
            false => crate::consts::GC_MAGIC,
        })?;
        ser.writer.write_bytes(self.game_title)?;
        ser.writer.write_bool(self.disable_hash_verification)?;
        ser.writer.write_bool(self.disable_disc_encryption)?;
        ser.writer.write_bytes(vec![0u8; 0x39e])?;
        Ok(())
    }
}

impl serde_binary::Decode for DiscHeader {
    fn decode(&mut self, de: &mut serde_binary::Deserializer) -> serde_binary::Result<()> {
        self.disc_id = de.reader.read_u8()?;
        self.game_code = de.reader.read_u16()?;
        self.region_code = de.reader.read_u8()?;
        self.maker_code = de.reader.read_u16()?;
        self.disc_number = de.reader.read_u8()?;
        self.disc_version = de.reader.read_u8()?;
        self.audio_streaming = de.reader.read_bool()?;
        self.streaming_buffer_size = de.reader.read_u8()?;
        self.unk.copy_from_slice(&de.reader.read_bytes(14)?[..]);
        let wii_magic = de.reader.read_u32()?;
        self.is_wii = wii_magic == crate::consts::WII_MAGIC;
        self.game_title.copy_from_slice(&de.reader.read_bytes(64)?);
        self.disable_hash_verification = de.reader.read_bool()?;
        self.disable_disc_encryption = de.reader.read_bool()?;
        // skip the remaining 0x39e padding bytes
        de.reader.read_bytes(0x39e)?;
        Ok(())
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

pub type Partition = Option<u64>;

pub trait Volume: std::io::Read + std::io::Seek {
    fn read_swapped<T: endian::FromBytesBE>(&mut self) -> Result<T, std::io::Error> {
        let mut buf = vec![0u8; T::N];
        self.read_exact(&mut buf)?;
        Ok(endian::FromBytesBE::from_bytes_be(&buf))
    }

    fn read_swapped_and_shifted<T: endian::FromBytesBE + std::ops::Shl<Output = T> + From<u8>>(
        &mut self,
    ) -> Result<T, std::io::Error> {
        self.read_swapped::<T>()
            .map(|n| n.shl(T::from(self.get_offset_shift())))
    }

    fn get_offset_shift(&self) -> u8 {
        0
    }

    fn has_wii_hashes(&self) -> bool;
    fn has_wii_encryption(&self) -> bool;
    fn get_partitions(&self) -> Vec<Partition> {
        Vec::new()
    }
    fn get_game_partition(&self) -> Partition {
        Partition::None
    }
    fn get_partition_type(&self, partition: &Partition) -> Option<u32> {
        let _ = partition;
        None
    }
    fn get_title_id(&self, partition: Option<&Partition>) -> Option<u64>;
}

#[cfg(test)]
mod tests {
    use super::*;

    struct VolumeTest;

    impl std::io::Seek for VolumeTest {
        fn seek(&mut self, _pos: std::io::SeekFrom) -> std::io::Result<u64> {
            Ok(0)
        }
    }

    impl std::io::Read for VolumeTest {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            buf.copy_from_slice(&(0..).take(buf.len()).collect::<Vec<_>>()[..]);
            Ok(buf.len())
        }
    }

    impl Volume for VolumeTest {
        fn get_offset_shift(&self) -> u8 {
            2
        }
        
        fn has_wii_hashes(&self) -> bool {
            todo!()
        }
        
        fn has_wii_encryption(&self) -> bool {
            todo!()
        }
        
        fn get_title_id(&self, partition: Option<&Partition>) -> Option<u64> {
            todo!()
        }
    }

    #[test]
    fn read_swapped_accept_numbers() {
        let mut volume = VolumeTest;
        let value: u32 = volume.read_swapped().unwrap();
        assert_eq!(value, 0x00010203);
    }

    #[test]
    fn read_swapped_and_shifted_accept_number() {
        let mut volume = VolumeTest;
        let value: u32 = volume.read_swapped_and_shifted().unwrap();
        assert_eq!(value, 0x0004080C);
    }
}
