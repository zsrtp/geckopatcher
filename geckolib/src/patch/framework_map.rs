use crate::config::Config;
use crate::patch::demangle::demangle as demangle_tww;
use crate::patch::linker::{LinkedSection, SectionKind};
use async_std::io::{Read as AsyncRead, ReadExt, Seek as AsyncSeek};
use eyre::Context;
use regex::{Captures, Regex};
use rustc_demangle::demangle as demangle_rust;
use std::collections::HashMap;
use std::fs::File;
use std::io::{prelude::*, BufWriter};
use std::str;

pub fn create(
    config: &Config,
    original: Option<&[u8]>,
    sections: &[LinkedSection],
) -> eyre::Result<()> {
    let path = match &config.build.map {
        Some(path) => path,
        None => return Ok(()),
    };

    let mut file = BufWriter::new(File::create(path).context("Couldn't create the symbol map")?);

    writeln!(file, ".text section layout")?;

    for section in sections {
        let section_name_buf;
        let section_name = section.section_name;
        let section_name = if section_name.starts_with(".text.")
            && section.kind == SectionKind::TextSection
        {
            section_name_buf = demangle_rust(&section_name[".text.".len()..]).to_string();
            let mut section_name: &str = &section_name_buf;
            if section_name.len() >= 19 && &section_name[section_name.len() - 19..][..3] == "::h" {
                section_name = &section_name[..section_name.len() - 19];
            }
            section_name
        } else {
            section_name
        };

        writeln!(
            file,
            "  00000000 {:06x} {:08x}  4 {} \t{}",
            section.len - section.sym_offset,
            section.address + section.sym_offset,
            section_name,
            section.member_name
        )?;
    }

    if let Some(original) = original {
        let regex = Regex::new(r"(\s{2}\d\s)(.*)(\s{2}.*)").unwrap();

        writeln!(file)?;
        writeln!(file)?;

        for line in str::from_utf8(original)?.lines() {
            let line = regex.replace(line, |c: &Captures| {
                let demangled = demangle_tww(&c[2]);
                format!("{}{}{}", &c[1], demangled.unwrap_or(c[2].into()), &c[3])
            });

            writeln!(file, "{}", line)?;
        }
    }

    Ok(())
}

pub async fn parse<R: AsyncRead + AsyncSeek + Unpin>(
    buf: &mut R,
) -> eyre::Result<HashMap<String, u32>> {
    let mut symbols = HashMap::new();
    let regex = Regex::new(r"\s{2}\w{8}\s\w{6}\s(\w{8}).{4}(.*)\s{2}").unwrap();
    let text = {
        let mut text = String::new();
        std::pin::pin!(buf).read_to_string(&mut text).await?;
        text
    };
    for line in text.lines() {
        if let Some(captures) = regex.captures(line) {
            let name = captures.get(2).unwrap().as_str();
            if !name.starts_with('.') {
                let address = u32::from_str_radix(captures.get(1).unwrap().as_str(), 16)?;

                symbols.insert(
                    demangle_tww(name)
                        .map(|n| n.into_owned())
                        .unwrap_or_else(|_| name.to_owned()),
                    address,
                );
            }
        }
    }
    Ok(symbols)
}
