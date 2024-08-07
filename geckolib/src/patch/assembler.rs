use eyre::Context;
use std::collections::{BTreeMap, HashMap};
use syn::Error as ParseError;

pub struct Assembler<'a> {
    symbol_table: Option<BTreeMap<&'a str, u32>>,
    prelinked_symbols: &'a HashMap<String, u32>,
    program_counter: u32,
}

pub struct Instruction {
    pub address: u32,
    pub data: u32,
}

impl<'a> Assembler<'a> {
    pub fn new(
        symbol_table: Option<BTreeMap<&'a str, u32>>,
        prelinked_symbols: &'a HashMap<String, u32>,
    ) -> Assembler<'a> {
        Assembler {
            symbol_table,
            prelinked_symbols,
            program_counter: 0,
        }
    }

    pub fn assemble_all_lines(&mut self, lines: &[&str]) -> eyre::Result<Vec<Instruction>> {
        let mut instructions = Vec::new();

        let filtered_lines = lines
            .iter()
            .map(|l| reduce_line_to_code(l))
            .filter(|l| !l.is_empty());

        for line in filtered_lines {
            if line.ends_with(':') {
                self.program_counter = self
                    .parse_program_counter_label(line)
                    .context("Couldn't parse address label")?;
            } else {
                let instruction = self.parse_instruction(line)?;
                instructions.push(instruction);
                self.program_counter += 4;
            }
        }

        Ok(instructions)
    }

    fn parse_instruction(&self, line: &str) -> eyre::Result<Instruction> {
        let data;

        if let Some(operand) = line.strip_prefix("bl ") {
            let destination = self.resolve_symbol(operand)?;
            data = build_branch_instruction(self.program_counter, destination, false, true);
        } else if let Some(operand) = line.strip_prefix("b ") {
            let destination = self.resolve_symbol(operand)?;
            data = build_branch_instruction(self.program_counter, destination, false, false);
        } else if let Some(operand) = line.strip_prefix("u32 ") {
            data = parse_u32_literal(operand).context("Couldn't parse the u32 literal")?;
        } else if let Some(operand) = line.strip_prefix("lis ") {
            let mut splits = operand.split(',').map(|s| s.trim());
            let register = splits
                .next()
                .ok_or_else(|| eyre::eyre!("Expected register"))?;
            if !register.starts_with('r') {
                eyre::bail!("Unexpected register: \"{}\"", register);
            }
            let register =
                parse_i64_literal(&register[1..]).context("Couldn't parse the register index")?;
            let imm = splits
                .next()
                .ok_or_else(|| eyre::eyre!("Expected immediate for lis instruction"))?;
            let imm =
                parse_i64_literal(imm).context("Couldn't parse immediate for lis instruction")?;
            data = build_lis_instruction(register as u8, imm as i16);
        } else if line == "nop" {
            data = 0x60000000;
        } else {
            eyre::bail!("Unknown instruction: \"{}\"", line);
        }

        Ok(Instruction {
            address: self.program_counter,
            data,
        })
    }

    fn resolve_symbol(&self, symbol: &str) -> eyre::Result<u32> {
        if let Ok(address) = parse_u32_literal(symbol) {
            return Ok(address);
        }

        if let Some(&symbol) = self.symbol_table.as_ref().and_then(|s| s.get(symbol)) {
            return Ok(symbol);
        }

        if let Some(&symbol) = self.prelinked_symbols.get(symbol) {
            return Ok(symbol);
        }

        eyre::bail!(format!("The symbol \"{}\" wasn't found", symbol))
    }

    fn parse_program_counter_label(&self, line: &str) -> eyre::Result<u32> {
        let mut line = line[..line.len() - 1].trim_start();
        let mut address = 0u32;
        let mut is_add = true;
        loop {
            let val = match line
                .chars()
                .next()
                .ok_or_else(|| eyre::eyre!("Expected integer literal or symbol"))?
            {
                '0'..='9' | '-' => {
                    let len = line
                        .char_indices()
                        .take_while(|&(i, c)| match c {
                            '0'..='9' | 'A'..='F' | 'a'..='f' | '_' => true,
                            '-' if i == 0 => true,
                            'x' if i == 1 => true,
                            _ => false,
                        })
                        .map(|(_, c)| c.len_utf8())
                        .sum();
                    let symbol = &line[..len];
                    line = &line[len..];
                    parse_u32_literal(symbol)?
                }
                '[' => {
                    let mut open_count = 1;
                    line = &line[1..];
                    let len = line
                        .chars()
                        .take_while(|c| match c {
                            '[' => {
                                open_count += 1;
                                true
                            }
                            ']' => {
                                open_count -= 1;
                                open_count != 0
                            }
                            _ => true,
                        })
                        .map(|c| c.len_utf8())
                        .sum();
                    let symbol = &line[..len];
                    line = &line[len + 1..];
                    self.resolve_symbol(symbol)?
                }
                _ => eyre::bail!("Expected integer literal or symbol"),
            };
            address = if is_add {
                address.wrapping_add(val)
            } else {
                address.wrapping_sub(val)
            };
            line = line.trim_start();
            is_add = match line.chars().next() {
                Some('+') => true,
                Some('-') => false,
                None => return Ok(address),
                _ => eyre::bail!("Expected + or - operator but found \"{}\"", line),
            };
            line = line[1..].trim_start();
        }
    }
}

fn reduce_line_to_code(line: &str) -> &str {
    let mut line = line;
    if let Some(index) = line.find(';') {
        line = &line[..index];
    }
    line.trim()
}

fn parse_i64_literal(literal: &str) -> Result<i64, ParseError> {
    let val: syn::LitInt = syn::parse_str(literal)?;
    val.base10_parse::<i64>()
}

fn parse_u32_literal(literal: &str) -> Result<u32, ParseError> {
    parse_i64_literal(literal).map(|i| i as u32)
}

fn build_branch_instruction(address: u32, destination: u32, aa: bool, lk: bool) -> u32 {
    let bits_dest = if aa {
        destination
    } else {
        destination - address
    };
    let bits_aa = if aa { 1 } else { 0 };
    let bits_lk = if lk { 1 } else { 0 };

    (18 << 26) | (0x3FFFFFC & bits_dest) | (bits_aa << 1) | bits_lk
}

fn build_lis_instruction(register: u8, imm: i16) -> u32 {
    build_addis_instruction(register, 0, imm)
}

fn build_addis_instruction(reg_d: u8, reg_a: u8, imm: i16) -> u32 {
    0x3C00_0000
        | ((reg_d as u32 & 0b11111) << 21)
        | ((reg_a as u32 & 0b11111) << 16)
        | (imm as u32 & 0xFFFF)
}
