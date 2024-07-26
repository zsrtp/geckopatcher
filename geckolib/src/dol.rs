use crate::assembler::Instruction;
use async_std::io::{prelude::*, Read as AsyncRead, ReadExt, Seek as AsyncSeek};
use byteorder::{ByteOrder, BE};
use std::fmt::{self, Debug};

pub struct Section {
    pub address: u32,
    pub data: Box<[u8]>,
}

#[derive(Default)]
pub struct DolFile {
    pub text_sections: Vec<Section>,
    pub data_sections: Vec<Section>,
    pub bss_address: u32,
    pub bss_size: u32,
    pub entry_point: u32,
}

pub struct DolHeader {
    pub text_section_offsets: [u32; 7],
    pub data_section_offsets: [u32; 11],
    pub text_section_addresses: [u32; 7],
    pub data_section_addresses: [u32; 11],
    pub text_section_sizes: [u32; 7],
    pub data_section_sizes: [u32; 11],
    pub bss_address: u32,
    pub bss_size: u32,
    pub entry_point: u32,
}

impl Debug for Section {
    fn fmt(&self, formatter: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        write!(formatter, "{:x}", self.address)
    }
}

impl Debug for DolFile {
    fn fmt(&self, formatter: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        write!(
            formatter,
            r"text_sections: {:#?},
data_sections: {:#?},
bss_address: {:x},
bss_size: {},
entry_point: {:x}",
            self.text_sections,
            self.data_sections,
            self.bss_address,
            self.bss_size,
            self.entry_point
        )
    }
}

async fn read<R: AsyncRead + AsyncSeek + Unpin>(
    data: &mut R,
    offset: usize,
    length: usize,
) -> eyre::Result<Vec<u8>> {
    let mut buf = vec![0u8; length];
    data.seek(std::io::SeekFrom::Start(offset as u64)).await?;
    data.read_exact(&mut buf).await?;
    Ok(buf)
}

async fn read_u32<R: AsyncRead + AsyncSeek + Unpin>(
    data: &mut R,
    offset: usize,
) -> eyre::Result<u32> {
    let mut buf = vec![0u8; 4];
    data.seek(std::io::SeekFrom::Start(offset as u64)).await?;
    data.read_exact(&mut buf).await?;
    Ok(BE::read_u32(&buf))
}

fn write_u32(data: &mut [u8], value: u32) {
    BE::write_u32(data, value)
}

async fn read_sections<R: AsyncRead + AsyncSeek + Unpin>(
    data: &mut R,
    offsets_offset: usize,
    addresses_offset: usize,
    lengths_offset: usize,
    max: usize,
) -> eyre::Result<Vec<Section>> {
    let mut sections = Vec::new();
    for i in 0..max {
        let offset = read_u32(data, 4 * i + offsets_offset).await?;
        let address = read_u32(data, 4 * i + addresses_offset).await?;
        let length = read_u32(data, 4 * i + lengths_offset).await?;
        if length == 0 {
            break;
        }
        let section_data = read(data, offset as usize, length as usize)
            .await?
            .into_boxed_slice();
        let section = Section {
            address,
            data: section_data,
        };
        sections.push(section);
    }
    Ok(sections)
}

impl DolFile {
    pub async fn parse<R: AsyncRead + AsyncSeek + Unpin>(data: &mut R) -> eyre::Result<Self> {
        let text_sections = read_sections(data, 0x0, 0x48, 0x90, 7).await?;
        let data_sections = read_sections(data, 0x1c, 0x64, 0xac, 11).await?;
        let bss_address = read_u32(data, 0xd8).await?;
        let bss_size = read_u32(data, 0xdc).await?;
        let entry_point = read_u32(data, 0xe0).await?;

        Ok(DolFile {
            text_sections,
            data_sections,
            bss_address,
            bss_size,
            entry_point,
        })
    }

    pub fn append(&mut self, other: DolFile) {
        self.text_sections.extend(other.text_sections);
        self.data_sections.extend(other.data_sections);
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut header = DolHeader::new();
        header.bss_address = self.bss_address;
        header.bss_size = self.bss_size;
        header.entry_point = self.entry_point;

        let mut data = Vec::<u8>::new();
        let mut i = 0;
        let mut offset = 256;

        for section in &self.text_sections {
            header.text_section_offsets[i] = offset as u32;
            header.text_section_addresses[i] = section.address;
            header.text_section_sizes[i] = section.data.len() as u32;

            i += 1;
            offset += section.data.len();
            data.extend(section.data.as_ref());
        }

        i = 0;

        for section in &self.data_sections {
            header.data_section_offsets[i] = offset as u32;
            header.data_section_addresses[i] = section.address;
            header.data_section_sizes[i] = section.data.len() as u32;

            i += 1;
            offset += section.data.len();
            data.extend(section.data.as_ref());
        }

        let mut bytes = header.to_bytes();
        bytes.extend(data);

        bytes
    }

    pub fn patch(&mut self, instructions: &[Instruction]) -> eyre::Result<()> {
        for instruction in instructions {
            let section = self
                .text_sections
                .iter_mut()
                .chain(self.data_sections.iter_mut())
                .find(|d| {
                    d.address <= instruction.address
                        && d.address + d.data.len() as u32 > instruction.address
                });

            if let Some(section) = section {
                let index = (instruction.address - section.address) as usize;
                write_u32(&mut section.data[index..], instruction.data);
            } else {
                return Err(eyre::eyre!("Patch couldn't be applied."));
            }
        }

        Ok(())
    }
}

impl Default for DolHeader {
    fn default() -> Self {
        Self::new()
    }
}

impl DolHeader {
    pub fn new() -> Self {
        DolHeader {
            text_section_offsets: [0; 7],
            data_section_offsets: [0; 11],
            text_section_addresses: [0; 7],
            data_section_addresses: [0; 11],
            text_section_sizes: [0; 7],
            data_section_sizes: [0; 11],
            bss_address: 0,
            bss_size: 0,
            entry_point: 0,
        }
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut data = vec![0; 256];
        let mut offset = 0;

        for &value in &self.text_section_offsets {
            write_u32(&mut data[offset..], value);
            offset += 4;
        }

        for &value in &self.data_section_offsets {
            write_u32(&mut data[offset..], value);
            offset += 4;
        }

        for &value in &self.text_section_addresses {
            write_u32(&mut data[offset..], value);
            offset += 4;
        }

        for &value in &self.data_section_addresses {
            write_u32(&mut data[offset..], value);
            offset += 4;
        }

        for &value in &self.text_section_sizes {
            write_u32(&mut data[offset..], value);
            offset += 4;
        }

        for &value in &self.data_section_sizes {
            write_u32(&mut data[offset..], value);
            offset += 4;
        }

        write_u32(&mut data[offset..], self.bss_address);
        offset += 4;
        write_u32(&mut data[offset..], self.bss_size);
        offset += 4;
        write_u32(&mut data[offset..], self.entry_point);

        data
    }
}
