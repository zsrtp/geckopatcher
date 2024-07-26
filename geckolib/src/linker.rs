use crate::dol::{DolFile, Section};
use crate::{info, UPDATER};
use byteorder::{ByteOrder, BE};
use goblin::archive::{Archive, Member};
use goblin::elf::{section_header, sym, Elf, Reloc};
use std::collections::{BTreeMap, HashMap, HashSet};

pub static BASIC_LIB: &[u8] = include_bytes!("../../resources/libbasic.a");

fn symbols_referenced_in_section<F>(section_index: usize, elf: &Elf, mut f: F)
where
    F: FnMut(usize),
{
    let reloc_table = reloc_table_for_section(section_index, elf);
    for relocation in reloc_table {
        let symbol_index = relocation.r_sym;
        f(symbol_index);
    }
}

fn reloc_table_for_section(section_index: usize, elf: &Elf) -> Vec<Reloc> {
    elf.shdr_relocs
        .iter()
        .find(|&&(reloc_index, _)| {
            section_index == elf.section_headers[reloc_index].sh_info as usize
        })
        .map_or(Vec::new(), |(_, r)| r.to_vec())
}

fn function_symbols_for_section<'a>(
    section_index: usize,
    elf: &'a Elf,
) -> Box<dyn Iterator<Item = sym::Sym> + 'a> {
    Box::new(
        elf.syms
            .iter()
            .filter(move |sym| section_index == sym.st_shndx && sym.is_function()),
    )
}

#[derive(Copy, Clone, PartialOrd, Ord, Hash, PartialEq, Eq, Debug)]
pub enum SectionKind {
    TextSection,
    DataSection,
    BlockStartedBySymbol,
}

#[derive(PartialOrd, Ord, Hash, PartialEq, Eq, Debug)]
struct SectionInfo<'a> {
    kind: SectionKind,
    archive_index: usize,
    member_name: &'a str,
    section_index: usize,
}

#[derive(PartialOrd, Ord, Hash, PartialEq, Eq)]
struct LookupKey<'a> {
    archive_index: usize,
    member_name: &'a str,
    section_index: usize,
}

struct LocatedSection<'a> {
    address: u32,
    padding: u32,
    len: u32,
    section_info: SectionInfo<'a>,
}

pub struct Linked<'a> {
    pub dol: DolFile,
    pub symbol_table: BTreeMap<&'a str, u32>,
    pub sections: Vec<LinkedSection<'a>>,
}

pub struct LinkedSection<'a> {
    pub address: u32,
    pub len: u32,
    pub member_name: &'a str,
    pub section_name: &'a str,
    pub sym_offset: u32,
    pub kind: SectionKind,
}

fn resolve_archive_symbol_to_member<'a, 'b: 'a>(
    symbol: &str,
    archive: &'b Archive<'a>,
) -> Option<(&'a str, &'b Member<'a>)> {
    let member_name = archive.member_of_symbol(symbol)?;
    Some((member_name, archive.get(member_name).unwrap()))
}

fn resolve_symbol_to_archive_mut<'a: 'b, 'b>(
    symbol: &str,
    archive_bufs: &'a [Vec<u8>],
    archives: &'b mut [Option<Archive<'a>>],
) -> Option<(usize, &'b Archive<'a>)> {
    for (index, (archive, archive_buf)) in archives.iter_mut().zip(archive_bufs).enumerate() {
        let archive = archive.get_or_insert_with(|| Archive::parse(archive_buf).unwrap());
        if archive.member_of_symbol(symbol).is_some() {
            return Some((index, archive));
        }
    }
    None
}

fn resolve_symbol_to_archive<'a: 'b, 'b>(
    symbol: &str,
    archives: &'b [Option<Archive<'a>>],
) -> Option<(usize, &'b Archive<'a>)> {
    for (index, archive) in archives.iter().enumerate() {
        if let Some(archive) = archive {
            if archive.member_of_symbol(symbol).is_some() {
                return Some((index, archive));
            }
        }
    }
    None
}

fn traverse_global<'a>(
    global_symbols_to_visit: &mut Vec<String>,
    archive_bufs: &'a [Vec<u8>],
    archives: &mut [Option<Archive<'a>>],
    parsed_elfs: &mut BTreeMap<(usize, &'a str), Elf<'a>>,
    visited_sections: &mut HashSet<SectionInfo<'a>>,
    prelinked_symbols: &HashMap<String, u32>,
) -> eyre::Result<()> {
    let mut archive_symbols_to_visit = Vec::new();

    while let Some(symbol) = global_symbols_to_visit.pop() {
        if let Some((archive_index, archive)) =
            resolve_symbol_to_archive_mut(&symbol, archive_bufs, archives)
        {
            archive_symbols_to_visit.push(symbol);

            traverse_archive(
                &archive_bufs[archive_index],
                archive,
                archive_index,
                global_symbols_to_visit,
                &mut archive_symbols_to_visit,
                parsed_elfs,
                visited_sections,
            );
        } else if !prelinked_symbols.contains_key(&symbol) {
            return Err(eyre::eyre!("Unresolved symbol `{}`", symbol));
        }
    }

    Ok(())
}

fn traverse_archive<'a>(
    archive_buf: &'a [u8],
    archive: &Archive<'a>,
    archive_index: usize,
    global_symbols_to_visit: &mut Vec<String>,
    archive_symbols_to_visit: &mut Vec<String>,
    parsed_elfs: &mut BTreeMap<(usize, &'a str), Elf<'a>>,
    visited_sections: &mut HashSet<SectionInfo<'a>>,
) {
    let mut symbols_to_visit = Vec::new();

    while let Some(archive_symbol_name) = archive_symbols_to_visit.pop() {
        let member_name = match archive.member_of_symbol(&archive_symbol_name) {
            Some(member_name) => member_name,
            None => {
                global_symbols_to_visit.push(archive_symbol_name.to_owned());
                continue;
            }
        };

        let member = archive.get(member_name).unwrap();
        let elf_buf = &archive_buf[member.offset as usize..][..member.header.size];
        let elf = parsed_elfs
            .entry((archive_index, member_name))
            .or_insert_with(|| Elf::parse(elf_buf).unwrap());

        for (symbol_index, symbol) in elf.syms.iter().enumerate() {
            let name_index = symbol.st_name;
            let name = elf.strtab.get_at(name_index).unwrap();
            if name == archive_symbol_name {
                symbols_to_visit.push(symbol_index);
            }
        }

        while let Some(symbol_index) = symbols_to_visit.pop() {
            let symbol = elf.syms.get(symbol_index).unwrap();
            let section_index = symbol.st_shndx;
            let name_index = symbol.st_name;
            let name = elf.strtab.get_at(name_index).unwrap();

            let section = &elf.section_headers[section_index];

            if symbol.is_import() && section.sh_type == section_header::SHT_NULL {
                archive_symbols_to_visit.push(name.to_string());
            } else if visited_sections.insert(SectionInfo {
                archive_index,
                member_name,
                section_index,
                kind: if section.is_executable() {
                    SectionKind::TextSection
                } else if section.sh_type == section_header::SHT_NOBITS {
                    SectionKind::BlockStartedBySymbol
                } else {
                    SectionKind::DataSection
                },
            }) {
                symbols_referenced_in_section(section_index, elf, |symbol_index| {
                    symbols_to_visit.push(symbol_index);
                });
            }
        }
    }
}

struct Layout<'a> {
    sections: Vec<LocatedSection<'a>>,
    lookup: HashMap<LookupKey<'a>, usize>,
    data_section_address: Option<u32>,
    symbol_table: BTreeMap<&'a str, u32>,
}

fn create_layout<'a>(
    base_address: u32,
    visited_sections: HashSet<SectionInfo<'a>>,
    parsed_elfs: &BTreeMap<(usize, &'a str), Elf<'a>>,
) -> Layout<'a> {
    let mut data_section_address = None;
    let mut address = base_address;
    let mut symbol_table = BTreeMap::new();

    let mut visited_sections = visited_sections.into_iter().collect::<Vec<_>>();
    visited_sections.sort_unstable();

    let mut lookup = HashMap::with_capacity(visited_sections.len());

    let sections = visited_sections
        .into_iter()
        .enumerate()
        .map(|(index, section_info)| {
            let elf = &parsed_elfs[&(section_info.archive_index, section_info.member_name)];
            let section = &elf.section_headers[section_info.section_index];
            let align = section.sh_addralign as u32;
            let rem = address % align;
            let padding = if rem != 0 {
                let padding = align - rem;
                address += padding;
                padding
            } else {
                0
            };

            if data_section_address.is_none() && section_info.kind != SectionKind::TextSection {
                data_section_address = Some(address);
            }

            for symbol in function_symbols_for_section(section_info.section_index, elf) {
                let bind = symbol.st_bind();
                let is_global = bind == sym::STB_GLOBAL || bind == sym::STB_WEAK;
                if is_global {
                    let name_index = symbol.st_name;
                    let name = elf.strtab.get_at(name_index).unwrap();
                    symbol_table.insert(name, address + symbol.st_value as u32);
                }
            }

            lookup.insert(
                LookupKey {
                    archive_index: section_info.archive_index,
                    member_name: section_info.member_name,
                    section_index: section_info.section_index,
                },
                index,
            );
            let val = LocatedSection {
                padding,
                address,
                len: section.sh_size as u32,
                section_info,
            };

            address += section.sh_size as u32;

            val
        })
        .collect();

    Layout {
        sections,
        lookup,
        data_section_address,
        symbol_table,
    }
}

fn relocate_and_collect<'a>(
    layout: &Layout<'a>,
    archives: &'a [Option<Archive<'a>>],
    archive_bufs: &'a [Vec<u8>],
    parsed_elfs: &BTreeMap<(usize, &'a str), Elf<'a>>,
    prelinked_symbols: &HashMap<String, u32>,
) -> (Vec<u8>, Vec<u8>) {
    let (mut text_section, mut data_section) = (Vec::new(), Vec::new());

    for &LocatedSection {
        section_info:
            SectionInfo {
                archive_index,
                member_name,
                section_index,
                kind: section_kind,
                ..
            },
        address: located_section_address,
        padding: located_section_padding,
        ..
    } in &layout.sections
    {
        let archive = archives[archive_index].as_ref().unwrap();
        let archive_buf = &archive_bufs[archive_index];
        let member = archive.get(member_name).unwrap();
        let elf_buf = &archive_buf[member.offset as usize..][..member.header.size];

        let elf = &parsed_elfs[&(archive_index, member_name)];
        let section = &elf.section_headers[section_index];
        let mut section_buf;
        let mut section_slice = if section_kind != SectionKind::BlockStartedBySymbol {
            &elf_buf[section.sh_offset as usize..][..section.sh_size as usize]
        } else {
            &[]
        };

        let reloc_table = reloc_table_for_section(section_index, elf);
        if !reloc_table.is_empty() {
            section_buf = section_slice.to_owned();

            for reloc in reloc_table {
                let instruction = &mut section_buf[reloc.r_offset as usize..][..4];
                let symbol_index = reloc.r_sym;
                let symbol = elf.syms.get(symbol_index).unwrap();
                let symbol_section_index = symbol.st_shndx;
                let (section_address, symbol_offset) = layout
                    .lookup
                    .get(&LookupKey {
                        archive_index,
                        member_name,
                        section_index: symbol_section_index,
                    })
                    .map(|&index| (layout.sections[index].address, symbol.st_value as u32))
                    .unwrap_or_else(|| {
                        let name_index = symbol.st_name;
                        let archive_symbol_name = elf.strtab.get_at(name_index).unwrap();
                        if let Some((archive_index, member_name)) = archive
                            .member_of_symbol(archive_symbol_name)
                            .map(|n| (archive_index, n))
                            .or_else(|| {
                                let (archive_index, archive) =
                                    resolve_symbol_to_archive(archive_symbol_name, archives)?;
                                let (member_name, _) =
                                    resolve_archive_symbol_to_member(archive_symbol_name, archive)?;
                                Some((archive_index, member_name))
                            })
                        {
                            let elf = &parsed_elfs[&(archive_index, member_name)];

                            for (symbol_index, symbol) in elf.syms.iter().enumerate() {
                                let name_index = symbol.st_name;
                                let name = elf.strtab.get_at(name_index).unwrap();
                                if name == archive_symbol_name {
                                    let symbol = elf.syms.get(symbol_index).unwrap();
                                    let section_index = symbol.st_shndx;
                                    let located_section_index = layout.lookup[&LookupKey {
                                        archive_index,
                                        member_name,
                                        section_index,
                                    }];
                                    return (
                                        layout.sections[located_section_index].address,
                                        symbol.st_value as u32,
                                    );
                                }
                            }
                            unreachable!()
                        } else {
                            if let Ok(mut updater) = UPDATER.lock() {
                                let _ = updater.set_message(format!(
                                    "Game Symbol {} at addr: {:08x}",
                                    archive_symbol_name,
                                    located_section_address
                                        .wrapping_add(reloc.r_offset as u32)
                                ));
                            }
                            info!(
                                "Game Symbol {} at addr: {:08x}",
                                archive_symbol_name,
                                located_section_address
                                    .wrapping_add(reloc.r_offset as u32)
                            );
                            (prelinked_symbols[archive_symbol_name], 0)
                        }
                    });

                // Based on:
                // https://github.com/llvm-mirror/lld/blob/0e7ca58c010ce93e66ce716923b0570c91248b7e/ELF/InputSection.cpp#L641

                // S -> Sym.getVA(0)
                let symbol_address = section_address.wrapping_add(symbol_offset);
                // A -> Addend
                let a = reloc.r_addend.unwrap_or(0) as u32;
                // P -> getVA(Rel.Offset)
                // getVa(Offset) => (Out ? Out->Addr : 0) + getOffset(Offset)
                let p = located_section_address.wrapping_add(reloc.r_offset as u32);

                // The enum can be found here:
                // https://github.com/vocho/openqnx/blob/master/trunk/lib/elf/public/sys/elf_ppc.h#L50
                const R_PPC_ADDR32: u32 = 1;
                const R_PPC_ADDR16_LO: u32 = 4;
                const R_PPC_ADDR16_HI: u32 = 5;
                const R_PPC_ADDR16_HA: u32 = 6;
                const R_PPC_REL24: u32 = 10;
                const R_PPC_PLTREL24: u32 = 18;
                const R_PPC_REL32: u32 = 26;

                let value = match reloc.r_type {
                    R_PPC_ADDR32 | R_PPC_ADDR16_HA | R_PPC_ADDR16_HI | R_PPC_ADDR16_LO => {
                        // R_ABS -> S + A -> Sym.getVA(A)
                        symbol_address.wrapping_add(a)
                    }
                    R_PPC_REL24 | R_PPC_REL32 => {
                        // R_PC -> S + A - P -> Sym.getVA(A) - P
                        symbol_address.wrapping_add(a).wrapping_sub(p)
                    }
                    R_PPC_PLTREL24 => {
                        // R_PLT_PC -> L + A - P -> Sym.getPltVA() + A - P
                        // There is not dynamic linking, lower this as S + A - P
                        symbol_address.wrapping_add(a).wrapping_sub(p)
                    }
                    t => panic!("Unknown reloc type {}", t),
                };

                assert_ne!(
                    reloc.r_offset as i32, -1,
                    "Should be end of section. Can't handle this yet"
                );

                // Based on LLD:
                // https://github.com/llvm-mirror/lld/blob/6d2b0b2fa1005a104120a93bad32f487377e989b/ELF/Arch/PPC.cpp#L49
                match reloc.r_type {
                    R_PPC_ADDR16_HA => {
                        BE::write_u16(instruction, (value.wrapping_add(0x8000) >> 16) as u16)
                    }
                    R_PPC_ADDR16_HI => BE::write_u16(instruction, (value >> 16) as u16),
                    R_PPC_ADDR16_LO => BE::write_u16(instruction, value as u16),
                    R_PPC_ADDR32 | R_PPC_REL32 => BE::write_u32(instruction, value),
                    R_PPC_PLTREL24 | R_PPC_REL24 => {
                        let val = BE::read_u32(instruction) | (value & 0x3FFFFFC);
                        BE::write_u32(instruction, val);
                    }
                    t => panic!("Unknown reloc type {}", t),
                }
            }

            section_slice = &section_buf;
        }

        if section_kind == SectionKind::TextSection {
            text_section.extend(vec![0; located_section_padding as usize]);
            text_section.extend(section_slice);
        } else {
            data_section.extend(vec![0; located_section_padding as usize]);
            if section_kind == SectionKind::DataSection {
                data_section.extend(section_slice);
            } else {
                data_section.extend(vec![0; section.sh_size as usize]);
            }
        }
    }

    (text_section, data_section)
}

pub fn link<'a>(
    archive_bufs: &'a [Vec<u8>],
    base_address: u32,
    mut global_symbols_to_visit: Vec<String>,
    prelinked_symbols: &HashMap<String, u32>,
) -> eyre::Result<Linked<'a>> {
    // TODO Handle "weak" and "merge" symbols

    let mut visited_sections = HashSet::new();
    let mut parsed_elfs = BTreeMap::new();

    let mut archives = Vec::with_capacity(archive_bufs.len());
    for _ in 0..archive_bufs.len() {
        archives.push(None);
    }

    traverse_global(
        &mut global_symbols_to_visit,
        archive_bufs,
        &mut archives,
        &mut parsed_elfs,
        &mut visited_sections,
        prelinked_symbols,
    )?;

    let layout = create_layout(base_address, visited_sections, &parsed_elfs);

    let (text_section, data_section) = relocate_and_collect(
        &layout,
        &archives,
        archive_bufs,
        &parsed_elfs,
        prelinked_symbols,
    );

    let dol = DolFile {
        text_sections: vec![Section {
            address: base_address,
            data: text_section.into_boxed_slice(),
        }],
        data_sections: vec![Section {
            address: layout.data_section_address.unwrap_or(base_address),
            data: data_section.into_boxed_slice(),
        }],
        bss_address: 0,
        bss_size: 0,
        entry_point: 0,
    };

    Ok(Linked {
        dol,
        symbol_table: layout.symbol_table,
        sections: layout
            .sections
            .into_iter()
            .map(|s| {
                let section_index = s.section_info.section_index;
                let elf = &parsed_elfs[&(s.section_info.archive_index, s.section_info.member_name)];
                let section = &elf.section_headers[section_index];
                let section_name = elf.shdr_strtab.get_at(section.sh_name).unwrap();

                let sym_offset =
                    if let Some(sym) = function_symbols_for_section(section_index, elf).next() {
                        sym.st_value as u32
                    } else {
                        0
                    };

                LinkedSection {
                    address: s.address,
                    len: s.len,
                    member_name: s.section_info.member_name,
                    section_name,
                    kind: s.section_info.kind,
                    sym_offset,
                }
            })
            .collect(),
    })
}
