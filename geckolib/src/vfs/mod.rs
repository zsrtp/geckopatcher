use crate::crypto::Unpackable;
use crate::iso::consts::OFFSET_DOL_OFFSET;
use crate::iso::disc::{align_addr, DiscType};
use crate::iso::read::DiscReader;
use crate::iso::{consts, FstEntry, FstNode, FstNodeType};
#[cfg(feature = "progress")]
use crate::UPDATER;
use async_std::io::prelude::{ReadExt, SeekExt, WriteExt};
use async_std::io::{self, Read as AsyncRead, Seek as AsyncSeek, Write as AsyncWrite};
use async_std::path::PathBuf;
use async_std::sync::Arc;
use byteorder::{ByteOrder, BE};
use eyre::Result;
#[cfg(feature = "progress")]
use human_bytes::human_bytes;
use num::ToPrimitive;
#[cfg(feature = "parallel")]
use rayon::prelude::*;
use std::io::{Error, SeekFrom};
use std::path::Path;
#[cfg(feature = "progress")]
use std::sync::TryLockError;
use std::task::{Context, Poll};

pub trait Node<R> {
    fn name(&self) -> String;
    fn get_type(&self) -> NodeType;
    fn into_directory(self) -> Option<Directory<R>>;
    fn as_directory_ref(&self) -> Option<&Directory<R>>;
    fn as_directory_mut(&mut self) -> Option<&mut Directory<R>>;
    fn into_file(self) -> Option<File<R>>;
    fn as_file_ref(&self) -> Option<&File<R>>;
    fn as_file_mut(&mut self) -> Option<&mut File<R>>;

    fn as_enum_ref(&self) -> NodeEnumRef<'_, R> {
        match self.get_type() {
            NodeType::File => NodeEnumRef::File(self.as_file_ref().unwrap()),
            NodeType::Directory => NodeEnumRef::Directory(self.as_directory_ref().unwrap()),
        }
    }

    fn as_enum_mut(&mut self) -> NodeEnumMut<'_, R> {
        match self.get_type() {
            NodeType::File => NodeEnumMut::File(self.as_file_mut().unwrap()),
            NodeType::Directory => NodeEnumMut::Directory(self.as_directory_mut().unwrap()),
        }
    }

    fn into_enum(self) -> NodeEnum<R>
    where
        Self: Sized,
    {
        match self.get_type() {
            NodeType::File => NodeEnum::File(self.into_file().unwrap()),
            NodeType::Directory => NodeEnum::Directory(self.into_directory().unwrap()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub enum NodeType {
    File = 0,
    Directory,
}

pub enum NodeEnumRef<'a, R> {
    File(&'a File<R>),
    Directory(&'a Directory<R>),
}

pub enum NodeEnumMut<'a, R> {
    File(&'a mut File<R>),
    Directory(&'a mut Directory<R>),
}

pub enum NodeEnum<R> {
    File(File<R>),
    Directory(Directory<R>),
}

pub struct GeckoFS<R> {
    pub(super) root: Directory<R>,
    pub(super) system: Directory<R>,
}

impl<R> GeckoFS<R>
where
    R: 'static,
{
    pub fn new() -> Self {
        Self {
            root: Directory::new(""),
            system: Directory::new("&&systemdata"),
        }
    }
}

impl<R> GeckoFS<R>
where
    R: AsyncRead + AsyncSeek + Unpin + Clone + 'static,
{
    #[doc = r"Utility function to read the disc."]
    async fn read_exact(reader: &mut DiscReader<R>, pos: SeekFrom, buf: &mut [u8]) -> Result<()> {
        reader.seek(pos).await?;
        Ok(reader.read_exact(buf).await?)
    }

    fn get_dir_structure_recursive(
        cur_index: &mut usize,
        fst: &Vec<FstNode>,
        parent_dir: &mut Directory<R>,
        reader: &DiscReader<R>,
    ) {
        let entry = &fst[*cur_index];

        match entry.clone() {
            FstNode::Directory {
                relative_file_name,
                parent_dir: _,
                next_dir_index,
            } => {
                let dir = parent_dir.mkdir(relative_file_name.clone());

                while *cur_index < next_dir_index - 1 {
                    *cur_index += 1;
                    GeckoFS::get_dir_structure_recursive(cur_index, fst, dir, reader);
                }
            }
            FstNode::File {
                relative_file_name,
                file_offset,
                file_size,
            } => {
                parent_dir.add_file(File::new(FileDataSource::Reader {
                    reader: reader.clone(),
                    fst: FstNode::File {
                        relative_file_name,
                        file_offset,
                        file_size,
                    },
                }));
            }
        }
    }

    pub async fn parse(mut reader: DiscReader<R>) -> Result<Self> {
        let mut root = Directory::new("");
        let mut system = Directory::new("&&systemdata");
        {
            let is_wii = reader.get_type() == DiscType::Wii;
            crate::debug!(
                "{}",
                if is_wii {
                    "The disc is a Wii game"
                } else {
                    "The disc is NOT a Wii game"
                }
            );
            let mut buf = [0u8; 4];
            GeckoFS::read_exact(
                &mut reader,
                SeekFrom::Start(consts::OFFSET_FST_OFFSET as u64),
                &mut buf,
            )
            .await?;
            let fst_offset = (BE::read_u32(&buf[..]) << (if is_wii { 2 } else { 0 })) as u64;
            GeckoFS::read_exact(&mut reader, SeekFrom::Start(fst_offset + 8), &mut buf).await?;
            let num_entries = BE::read_u32(&buf[..]) as usize;
            let mut fst_list_buf = vec![0u8; num_entries * FstEntry::BLOCK_SIZE];
            GeckoFS::read_exact(&mut reader, SeekFrom::Start(fst_offset), &mut fst_list_buf)
                .await?;
            let string_table_offset = num_entries as u64 * FstEntry::BLOCK_SIZE as u64;

            GeckoFS::read_exact(
                &mut reader,
                SeekFrom::Start(consts::OFFSET_FST_SIZE as u64),
                &mut buf,
            )
            .await?;
            let fst_size = (BE::read_u32(&buf) as usize) << (if is_wii { 2 } else { 0 });
            let mut str_tbl_buf = vec![0u8; fst_size - string_table_offset as usize];
            GeckoFS::read_exact(
                &mut reader,
                SeekFrom::Start(string_table_offset + fst_offset),
                &mut str_tbl_buf,
            )
            .await?;

            crate::debug!(
                "#fst enties: {}; #names: {}",
                num_entries,
                str_tbl_buf.split(|b| *b == 0).count()
            );

            let fst_entries: Vec<FstNode> = {
                #[cfg(feature = "parallel")]
                let chunks = fst_list_buf.par_chunks_exact(FstEntry::BLOCK_SIZE);
                #[cfg(not(feature = "parallel"))]
                let chunks = fst_list_buf.chunks_exact(FstEntry::BLOCK_SIZE);
                chunks
            }
            .map(|entry_buf| {
                let entry = FstEntry::try_from(entry_buf).unwrap();
                let mut node = FstNode::from_fstnode(&entry, &str_tbl_buf).unwrap();

                match &mut node {
                    FstNode::File { file_offset, .. } => {
                        *file_offset <<= if is_wii { 2 } else { 0 };
                    }
                    FstNode::Directory { parent_dir, .. } => {
                        *parent_dir <<= if is_wii { 2 } else { 0 };
                    }
                }

                node
            })
            .collect();

            GeckoFS::read_exact(
                &mut reader,
                SeekFrom::Start(consts::OFFSET_DOL_OFFSET as u64),
                &mut buf,
            )
            .await?;
            let dol_offset = (BE::read_u32(&buf) as usize) << (if is_wii { 2 } else { 0 });
            crate::debug!(
                "fst_size: 0x{:08X}; fst entries list size: 0x{:08X}",
                fst_size,
                num_entries * FstEntry::BLOCK_SIZE
            );

            system.add_file(File::new(FileDataSource::Reader {
                reader: reader.clone(),
                fst: FstNode::File {
                    relative_file_name: "iso.hdr".to_owned(),
                    file_offset: 0,
                    file_size: consts::HEADER_LENGTH,
                },
            }));
            system.add_file(File::new(FileDataSource::Reader {
                reader: reader.clone(),
                fst: FstNode::File {
                    relative_file_name: "AppLoader.ldr".to_owned(),
                    file_offset: consts::HEADER_LENGTH as u64,
                    file_size: dol_offset - consts::HEADER_LENGTH,
                },
            }));
            system.add_file(File::new(FileDataSource::Reader {
                reader: reader.clone(),
                fst: FstNode::File {
                    relative_file_name: "Start.dol".to_owned(),
                    file_offset: dol_offset as u64,
                    file_size: fst_offset as usize - dol_offset,
                },
            }));
            system.add_file(File::new(FileDataSource::Reader {
                reader: reader.clone(),
                fst: FstNode::File {
                    relative_file_name: "Game.toc".to_owned(),
                    file_offset: fst_offset,
                    file_size: fst_size,
                },
            }));

            let mut count = 1;
            while count < num_entries {
                GeckoFS::get_dir_structure_recursive(&mut count, &fst_entries, &mut root, &reader);
                count += 1;
            }
        }
        crate::debug!("{} children", root.children.len());
        Ok(Self { root, system })
    }

    /// Visits the directory tree to calculate the length of the FST table
    fn visitor_fst_len(mut acc: usize, node: &dyn Node<R>) -> usize {
        match node.as_enum_ref() {
            NodeEnumRef::Directory(dir) => {
                acc += 12 + dir.name().len() + 1;

                for child in &dir.children {
                    acc = GeckoFS::visitor_fst_len(acc, child.as_ref());
                }
            }
            NodeEnumRef::File(file) => {
                acc += 12 + file.name().len() + 1;
            }
        };
        acc
    }

    fn visitor_fst_entries(
        node: &mut dyn Node<R>,
        output_fst: &mut Vec<FstEntry>,
        files: &mut Vec<(File<R>, u64)>,
        fst_name_bank: &mut Vec<u8>,
        cur_parent_dir_index: usize,
        offset: &mut u64,
        is_wii: bool,
    ) -> Result<()> {
        match node.as_enum_mut() {
            NodeEnumMut::Directory(dir) => {
                let fst_entry = FstEntry::new_directory(
                    fst_name_bank.len() as u32,
                    cur_parent_dir_index as u64,
                    0,
                    is_wii,
                )?;

                fst_name_bank.extend_from_slice(dir.name().as_bytes());
                fst_name_bank.push(0);

                let this_dir_index = output_fst.len();

                output_fst.push(fst_entry);

                for child in &mut dir.children {
                    GeckoFS::visitor_fst_entries(
                        child.as_mut(),
                        output_fst,
                        files,
                        fst_name_bank,
                        this_dir_index,
                        offset,
                        is_wii,
                    )?;
                }

                let next_dir_index = output_fst.len() as u32;
                output_fst[this_dir_index].set_file_size_next_dir_index(next_dir_index);
            }
            NodeEnumMut::File(file) => {
                let pos = align_addr(*offset, 5);
                *offset = pos;

                let fst_entry = FstEntry::new_file(
                    fst_name_bank.len() as u32,
                    pos as u64,
                    file.len()? as u32,
                    is_wii,
                )?;

                fst_name_bank.extend_from_slice(file.name().as_bytes());
                fst_name_bank.push(0);

                *offset += file.len()? as u64;
                *offset = align_addr(*offset, 2);

                output_fst.push(fst_entry);
                files.push((file.clone(), pos));
            }
        };
        Ok(())
    }

    pub async fn serialize<W>(&mut self, writer: &mut W, is_wii: bool) -> Result<()>
    where
        W: AsyncWrite + Unpin,
    {
        crate::debug!("Serializing the FileSystem");
        let mut pos: u64 = 0;
        let header_size = self.sys().get_file("iso.hdr")?.len()?;
        let apploader_size = self.sys().get_file("AppLoader.ldr")?.len()?;

        // Calculate dynamic offsets
        let dol_offset_raw = header_size + apploader_size;
        let dol_offset = align_addr(dol_offset_raw, consts::DOL_ALIGNMENT_BIT);
        let dol_padding_size = dol_offset - dol_offset_raw;
        let dol_size = self.sys().get_file("Start.dol")?.len()?;

        let fst_list_offset_raw = dol_offset + dol_size;
        let fst_list_offset = align_addr(fst_list_offset_raw, consts::FST_ALIGNMENT_BIT);
        let fst_list_padding_size = fst_list_offset - fst_list_offset_raw;

        let fst_len = GeckoFS::visitor_fst_len(0, &self.root) - 1;

        let d = [
            (dol_offset >> if is_wii { 2u8 } else { 0u8 }) as u32,
            (fst_list_offset >> if is_wii { 2u8 } else { 0u8 }) as u32,
            fst_len as u32,
            fst_len as u32,
        ];
        let mut b = vec![0u8; 0x10];
        BE::write_u32_into(&d, &mut b);

        // Write header and app loader
        let mut buf = Vec::new();
        self.sys_mut()
            .get_file_mut("iso.hdr")?
            .read_to_end(&mut buf)
            .await?;
        writer.write_all(&buf[..OFFSET_DOL_OFFSET]).await?;
        writer.write_all(&b).await?;
        writer.write_all(&buf[OFFSET_DOL_OFFSET + 0x10..]).await?;
        pos += buf.len().to_u64().ok_or(eyre::eyre!("Buffer too large"))?;
        buf.clear();
        self.sys_mut()
            .get_file_mut("AppLoader.ldr")?
            .read_to_end(&mut buf)
            .await?;
        writer.write_all(&buf).await?;
        pos += buf.len().to_u64().ok_or(eyre::eyre!("Buffer too large"))?;
        writer.write_all(&vec![0u8; dol_padding_size]).await?;
        pos += dol_padding_size
            .to_u64()
            .ok_or(eyre::eyre!("DOL padding too large"))?;

        buf.clear();
        self.sys_mut()
            .get_file_mut("Start.dol")?
            .read_to_end(&mut buf)
            .await?;
        writer.write_all(&buf).await?;
        pos += buf.len().to_u64().ok_or(eyre::eyre!("Buffer too large"))?;
        writer.write_all(&vec![0u8; fst_list_padding_size]).await?;
        pos += fst_list_padding_size
            .to_u64()
            .ok_or(eyre::eyre!("FST list padding too large"))?;

        let mut output_fst = vec![FstEntry::new_directory(0, 0, 0, is_wii)?];
        let mut fst_name_bank = Vec::new();
        let mut files = Vec::new();

        let mut offset = (fst_list_offset + fst_len) as u64;
        for node in self.root_mut().iter_mut() {
            let l = 0;
            GeckoFS::visitor_fst_entries(
                node.as_mut(),
                &mut output_fst,
                &mut files,
                &mut fst_name_bank,
                l,
                &mut offset,
                is_wii,
            )?;
        }
        {
            let next_dir_index = output_fst.len() as u32;
            output_fst[0].set_file_size_next_dir_index(next_dir_index);
        }
        crate::debug!("output_fst size = {}", output_fst.len());
        crate::debug!("first fst_name entry = {}", fst_name_bank[0]);
        #[cfg(feature = "progress")]
        let write_total_size: u64 = output_fst
            .iter()
            .filter_map(|f| {
                if let FstNodeType::File = f.get_node_type() {
                    Some(f.get_file_size_next_dir_index() as u64)
                } else {
                    None
                }
            })
            .sum();

        for entry in output_fst {
            writer.write_all(&entry.pack()).await?;
            pos += FstEntry::BLOCK_SIZE as u64;
        }

        writer.write_all(&fst_name_bank).await?;
        pos += fst_name_bank
            .len()
            .to_u64()
            .ok_or(eyre::eyre!("Buffer too large"))?;

        // Traverse the root directory tree to write all the files in order
        #[cfg(feature = "progress")]
        if let Ok(mut updater) = UPDATER.lock() {
            updater.set_type(crate::update::UpdaterType::Progress)?;
            updater.init(Some(write_total_size as usize))?;
            updater.set_title("Writing virtual FileSystem".to_string())?;
        }
        let mut offset = pos.to_usize().ok_or(eyre::eyre!("Offset too large"))?;
        #[cfg(feature = "progress")]
        let mut inc_buffer = 0usize;
        for (mut file, file_offset) in files {
            #[cfg(feature = "progress")]
            if let Ok(mut updater) = UPDATER.try_lock() {
                updater.set_message(format!(
                    "{:<32.32} ({:>8})",
                    file.name(),
                    human_bytes(file.len()? as f64)
                ))?;
            }
            let padding_size = file_offset as usize - offset;
            writer.write_all(&vec![0u8; padding_size]).await?;
            // Copy the file from the FileSystem to the Writer.
            // async_std::io::copy(file, writer).await?; // way too slow
            let mut rem = file.len()?;
            loop {
                if rem == 0 {
                    break;
                }
                let transfer_size = std::cmp::min(rem, 1024 * 1024);
                let mut buf = vec![0u8; transfer_size];
                file.read_exact(&mut buf).await?;
                writer.write_all(&buf).await?;
                rem -= transfer_size;
                #[cfg(feature = "progress")]
                match UPDATER.try_lock() {
                    Ok(mut updater) => {
                        updater.increment(transfer_size + inc_buffer)?;
                        inc_buffer = 0;
                    }
                    Err(TryLockError::WouldBlock) => {
                        inc_buffer += transfer_size;
                    }
                    _ => (),
                }
            }
            offset = (file_offset + file.len()? as u64) as usize;
        }

        // The disc apparently needs to be aligned to 8 bits
        let padding_size = align_addr(offset as u64, 8) as usize - offset;
        writer.write_all(&vec![0u8; padding_size]).await?;
        //offset += padding_size; // Unececssary, but kept for clarity

        #[cfg(feature = "progress")]
        if let Ok(mut updater) = UPDATER.lock() {
            updater.finish()?;
        }

        Ok(())
    }

    pub fn sys(&self) -> &Directory<R> {
        &self.system
    }

    pub fn sys_mut(&mut self) -> &mut Directory<R> {
        &mut self.system
    }

    pub fn root(&self) -> &Directory<R> {
        &self.root
    }

    pub fn root_mut(&mut self) -> &mut Directory<R> {
        &mut self.root
    }
}

impl<R> Default for GeckoFS<R>
where
    R: AsyncRead + AsyncSeek + Unpin + 'static,
{
    fn default() -> Self {
        Self::new()
    }
}

impl<R> Node<R> for GeckoFS<R>
where
    R: AsyncRead + AsyncSeek,
{
    fn name(&self) -> String {
        self.root.name()
    }

    fn get_type(&self) -> NodeType {
        NodeType::Directory
    }

    fn into_directory(self) -> Option<Directory<R>> {
        Some(self.root)
    }

    fn as_directory_ref(&self) -> Option<&Directory<R>> {
        Some(&self.root)
    }

    fn as_directory_mut(&mut self) -> Option<&mut Directory<R>> {
        Some(&mut self.root)
    }

    fn into_file(self) -> Option<File<R>> {
        None
    }

    fn as_file_ref(&self) -> Option<&File<R>> {
        None
    }

    fn as_file_mut(&mut self) -> Option<&mut File<R>> {
        None
    }
}

pub struct Directory<R> {
    name: String,
    children: Vec<Box<dyn Node<R>>>,
}

impl<R> Directory<R>
where
    R: 'static,
{
    pub fn new<S: Into<String>>(name: S) -> Directory<R> {
        Self {
            name: name.into(),
            children: Vec::new(),
        }
    }

    pub fn resolve_node<P: AsRef<Path>>(&self, path: P) -> Option<&dyn Node<R>> {
        let mut dir = self;
        let mut segments = path.as_ref().components().peekable();

        while let Some(segment) = segments.next() {
            if segments.peek().is_some() {
                // Must be a folder
                dir = dir
                    .children
                    .iter()
                    .filter_map(|c| c.as_directory_ref())
                    .find(|d| d.name == segment.as_os_str().to_string_lossy())?;
            } else {
                return dir
                    .children
                    .iter()
                    .map(|c| c.as_ref())
                    .find(|f| f.name() == segment.as_os_str().to_string_lossy())
                    .map(|x| x as &dyn Node<R>);
            }
        }
        Some(dir)
    }

    pub fn resolve_node_mut<P: AsRef<Path>>(&mut self, path: P) -> Option<&mut dyn Node<R>> {
        let mut dir = self;
        let mut segments = path.as_ref().components().peekable();

        while let Some(segment) = segments.next() {
            if segments.peek().is_some() {
                // Must be a folder
                dir = dir
                    .children
                    .iter_mut()
                    .filter_map(|c| c.as_directory_mut())
                    .find(|d| d.name == segment.as_os_str().to_str().unwrap_or_default())?;
            } else {
                return dir
                    .children
                    .iter_mut()
                    .map(|c| c.as_mut())
                    .find(|f| f.name() == segment.as_os_str().to_str().unwrap_or_default())
                    .map(|x| x as &mut dyn Node<R>);
            }
        }
        Some(dir)
    }

    pub fn mkdir<P: AsRef<Path>>(&mut self, name: P) -> &mut Directory<R> {
        if self
            .children
            .iter()
            .all(|c| c.name() != name.as_ref().as_os_str().to_string_lossy())
        {
            self.children.push(Box::new(Directory::new(
                name.as_ref().as_os_str().to_string_lossy(),
            )));
            self.children
                .last_mut()
                .map(|x| x.as_directory_mut().unwrap())
                .unwrap()
        } else {
            self.children
                .iter_mut()
                .find(|c| c.name() == name.as_ref().as_os_str().to_string_lossy())
                .map(|c| c.as_directory_mut().unwrap())
                .unwrap()
        }
    }

    pub fn mkdirs<P: AsRef<Path>>(&mut self, path: P) -> eyre::Result<&mut Directory<R>> {
        let mut p = PathBuf::new();
        for component in path.as_ref().components() {
            p.push(component);
            if let Some(node) = self.resolve_node_mut(&p) {
                if node.get_type() != NodeType::Directory {
                    return Err(eyre::eyre!(
                        "\"{:?}\" already exist and is not a directory",
                        component
                    ));
                }
            } else {
                let mut p2 = p.clone();
                p2.pop();
                let dir = self.get_dir_mut(p2)?;
                dir.mkdir(component);
            }
        }
        Ok(self
            .resolve_node_mut(path)
            .and_then(|node| node.as_directory_mut())
            .unwrap())
    }

    pub fn add_file(&mut self, file: File<R>) -> &mut File<R> {
        if self.children.iter().all(|c| c.name() != file.name()) {
            self.children.push(Box::new(file));
            self.children
                .last_mut()
                .map(|c| c.as_file_mut().unwrap())
                .unwrap()
        } else {
            self.children
                .iter_mut()
                .find(|c| c.name() == file.name())
                .map(|c| c.as_file_mut().unwrap())
                .unwrap()
        }
    }

    pub fn iter(&self) -> std::slice::Iter<'_, Box<dyn Node<R>>> {
        self.children.iter()
    }

    pub fn iter_mut(&mut self) -> std::slice::IterMut<'_, Box<dyn Node<R>>> {
        self.children.iter_mut()
    }

    pub fn iter_recurse(&self) -> impl Iterator<Item = &'_ File<R>> {
        crate::trace!("Start iter_recurse");
        fn traverse_depth<'b, R: 'static>(start: &'b dyn Node<R>, stack: &mut Vec<&'b File<R>>) {
            match start.as_enum_ref() {
                NodeEnumRef::File(file) => stack.push(file),
                NodeEnumRef::Directory(dir) => {
                    for child in &dir.children {
                        traverse_depth(child.as_ref(), stack);
                    }
                }
            }
        }
        let mut stack = Vec::new();
        traverse_depth(self, &mut stack);
        crate::debug!("{} fst files", stack.len());
        stack.into_iter()
    }

    pub fn iter_recurse_mut(&mut self) -> impl Iterator<Item = &'_ mut File<R>> {
        crate::trace!("Start iter_recurse_mut");
        fn traverse_depth<'b, R: 'static>(
            start: &'b mut dyn Node<R>,
            stack: &mut Vec<&'b mut File<R>>,
        ) {
            match start.as_enum_mut() {
                NodeEnumMut::File(file) => stack.push(file),
                NodeEnumMut::Directory(dir) => {
                    for child in &mut dir.children {
                        traverse_depth(child.as_mut(), stack);
                    }
                }
            }
        }
        let mut stack = Vec::new();
        traverse_depth(self, &mut stack);
        crate::debug!("{} fst files", stack.len());
        stack.into_iter()
    }

    pub fn get_file(&self, path: &str) -> Result<&File<R>> {
        let self_name = self.name().to_owned();
        self.resolve_node(path)
            .ok_or(eyre::eyre!(
                "\"{path}\" not found in the directory \"{self_name}\""
            ))?
            .as_file_ref()
            .ok_or(eyre::eyre!("\"{path}\" is not a File!"))
    }

    pub fn get_file_mut(&mut self, path: &str) -> Result<&mut File<R>> {
        let self_name = self.name().to_owned();
        self.resolve_node_mut(path)
            .ok_or(eyre::eyre!(
                "\"{path}\" not found in the directory \"{self_name}\""
            ))?
            .as_file_mut()
            .ok_or(eyre::eyre!("\"{path}\" is not a File!"))
    }

    pub fn get_dir<P: AsRef<Path>>(&self, path: P) -> Result<&Directory<R>> {
        let self_name = self.name().to_owned();
        self.resolve_node(path.as_ref())
            .ok_or(eyre::eyre!(
                "\"{:?}\" not found in the directory \"{}\"",
                path.as_ref(),
                self_name
            ))?
            .as_directory_ref()
            .ok_or(eyre::eyre!("\"{:?}\" is not a Directory!", path.as_ref()))
    }

    pub fn get_dir_mut<P: AsRef<Path>>(&mut self, path: P) -> Result<&mut Directory<R>> {
        let self_name = self.name().to_owned();
        self.resolve_node_mut(path.as_ref())
            .ok_or(eyre::eyre!(
                "\"{:?}\" not found in the directory \"{}\"",
                path.as_ref(),
                self_name
            ))?
            .as_directory_mut()
            .ok_or(eyre::eyre!("\"{:?}\" is not a Directory!", path.as_ref()))
    }
}

impl<R> Node<R> for Directory<R> {
    fn name(&self) -> String {
        self.name.clone()
    }

    fn get_type(&self) -> NodeType {
        NodeType::Directory
    }

    fn into_directory(self) -> Option<Directory<R>> {
        Some(self)
    }

    fn as_directory_ref(&self) -> Option<&Directory<R>> {
        Some(self)
    }

    fn as_directory_mut(&mut self) -> Option<&mut Directory<R>> {
        Some(self)
    }

    fn into_file(self) -> Option<File<R>> {
        None
    }

    fn as_file_ref(&self) -> Option<&File<R>> {
        None
    }

    fn as_file_mut(&mut self) -> Option<&mut File<R>> {
        None
    }
}

#[derive(Debug)]
pub(crate) enum FileDataSource<R> {
    Reader { reader: DiscReader<R>, fst: FstNode },
    Box { data: Box<[u8]>, name: String },
}

impl<R> FileDataSource<R> {
    pub fn name(&self) -> String {
        match self {
            Self::Reader { fst, .. } => fst.get_relative_file_name().to_owned(),
            Self::Box { name, .. } => name.clone(),
        }
    }

    pub fn len(&self) -> usize {
        match self {
            Self::Reader { fst, .. } => fst.get_file_size().unwrap(),
            Self::Box { data, .. } => data.len(),
        }
    }
}

impl<R> Clone for FileDataSource<R>
where
    R: Clone,
{
    fn clone(&self) -> Self {
        match self {
            Self::Reader { reader, fst } => Self::Reader {
                reader: reader.clone(),
                fst: fst.clone(),
            },
            Self::Box { data, name } => Self::Box {
                data: data.clone(),
                name: name.clone(),
            },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
enum FileState {
    #[default]
    Init,
    Seeking,
    Reading,
}

#[derive(Debug, Clone)]
struct FileStatus<R> {
    cursor: u64,
    state: FileState,
    data: FileDataSource<R>,
}

#[derive(Debug, Clone)]
pub struct File<R> {
    status: Arc<std::sync::Mutex<FileStatus<R>>>,
}

impl<R> File<R> {
    pub(crate) fn new(data: FileDataSource<R>) -> Self {
        Self {
            status: Arc::new(std::sync::Mutex::new(FileStatus {
                cursor: 0,
                state: FileState::Init,
                data,
            })),
        }
    }

    pub fn set_data(&mut self, data: Box<[u8]>) -> eyre::Result<()> {
        match self.status.lock() {
            Ok(mut status) => {
                status.data = FileDataSource::Box {
                    data,
                    name: status.data.name(),
                };
                Ok(())
            }
            Err(_) => Err(eyre::eyre!("Failed to lock the file status")),
        }
    }

    pub fn len(&self) -> eyre::Result<usize> {
        self.status
            .lock()
            .map(|status| status.data.len())
            .map_err(|_| eyre::eyre!("Failed to lock the file status"))
    }

    pub fn is_empty(&self) -> eyre::Result<bool> {
        Ok(self.len()? == 0)
    }
}

impl<R> AsyncSeek for File<R>
where
    R: AsyncSeek + Unpin,
{
    fn poll_seek(
        self: std::pin::Pin<&mut Self>,
        cx: &mut Context<'_>,
        pos: SeekFrom,
    ) -> Poll<std::io::Result<u64>> {
        crate::trace!("Seeking \"{0}\" to {1:?} ({1:016X?})", self.name(), pos);
        let mut status = self
            .status
            .lock()
            .map_err(|_| io::Error::new(io::ErrorKind::Other, "Failed to lock the file status"))?;
        let pos = match pos {
            SeekFrom::Start(pos) => {
                if pos
                    > self
                        .len()
                        .map_err(|err| io::Error::new(io::ErrorKind::Other, err))?
                        as u64
                {
                    return Poll::Ready(Err(Error::new(
                        async_std::io::ErrorKind::Other,
                        eyre::eyre!("Index out of range"),
                    )));
                }
                SeekFrom::Start(pos)
            }
            SeekFrom::End(pos) => {
                let new_pos = self
                    .len()
                    .map_err(|err| io::Error::new(io::ErrorKind::Other, err))?
                    as i64
                    + pos;
                if new_pos < 0 || pos > 0 {
                    return Poll::Ready(Err(Error::new(
                        async_std::io::ErrorKind::Other,
                        eyre::eyre!("Index out of range"),
                    )));
                }
                SeekFrom::End(pos)
            }
            SeekFrom::Current(pos) => {
                let new_pos = status.cursor as i64 + pos;
                if new_pos < 0
                    || new_pos
                        > self
                            .len()
                            .map_err(|err| io::Error::new(io::ErrorKind::Other, err))?
                            as i64
                {
                    return Poll::Ready(Err(Error::new(
                        async_std::io::ErrorKind::Other,
                        eyre::eyre!("Index out of range"),
                    )));
                }
                SeekFrom::Current(pos)
            }
        };
        let cursor = status.cursor;
        match &mut status.data {
            FileDataSource::Reader { reader, fst } => {
                match std::pin::pin!(reader).poll_seek(
                    cx,
                    match pos {
                        SeekFrom::Start(pos) => {
                            SeekFrom::Start(fst.get_file_offset().unwrap() + pos)
                        }
                        SeekFrom::End(pos) => SeekFrom::Start(
                            ((fst.get_file_offset().unwrap() as i64
                                + fst.get_file_size().unwrap() as i64)
                                + pos) as u64,
                        ),
                        SeekFrom::Current(pos) => SeekFrom::Start(
                            (fst.get_file_offset().unwrap() as i64 + cursor as i64 + pos) as u64,
                        ),
                    },
                ) {
                    Poll::Ready(Ok(_)) => match pos {
                        SeekFrom::Start(pos) => {
                            status.cursor = pos;
                            Poll::Ready(Ok(status.cursor))
                        }
                        SeekFrom::End(pos) => {
                            status.cursor = (fst.get_file_size().unwrap() as i64 + pos) as u64;
                            Poll::Ready(Ok(status.cursor))
                        }
                        SeekFrom::Current(pos) => {
                            status.cursor = (status.cursor as i64 + pos) as u64;
                            Poll::Ready(Ok(status.cursor))
                        }
                    },
                    Poll::Ready(Err(err)) => Poll::Ready(Err(err)),
                    Poll::Pending => Poll::Pending,
                }
            }
            FileDataSource::Box { data, .. } => match pos {
                SeekFrom::Start(pos) => {
                    status.cursor = pos;
                    Poll::Ready(Ok(status.cursor))
                }
                SeekFrom::End(pos) => {
                    status.cursor = (data.len() as i64 + pos) as u64;
                    Poll::Ready(Ok(status.cursor))
                }
                SeekFrom::Current(pos) => {
                    status.cursor = (status.cursor as i64 + pos) as u64;
                    Poll::Ready(Ok(status.cursor))
                }
            },
        }
    }
}

impl<R> AsyncRead for File<R>
where
    R: AsyncRead + AsyncSeek + Unpin,
{
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut [u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        crate::trace!(
            "Reading \"{}\" for 0x{:08X} byte(s)",
            self.name(),
            buf.len()
        );
        let mut status = self
            .status
            .lock()
            .map_err(|_| io::Error::new(io::ErrorKind::Other, "Failed to lock the file status"))?;
        let cursor = status.cursor;
        let end = std::cmp::min(
            buf.len(),
            (status.data.len() as i64 - cursor as i64) as usize,
        );
        match status.state {
            FileState::Init => {
                status.state = FileState::Seeking;
                cx.waker().wake_by_ref();
                Poll::Pending
            }
            FileState::Seeking => match status.data {
                FileDataSource::Reader {
                    ref mut reader,
                    ref fst,
                } => {
                    let guard_pin = std::pin::pin!(reader);
                    match guard_pin
                        .poll_seek(cx, SeekFrom::Start(fst.get_file_offset().unwrap() + cursor))
                    {
                        Poll::Ready(Ok(_)) => {
                            status.state = FileState::Reading;
                            cx.waker().wake_by_ref();
                            Poll::Pending
                        }
                        Poll::Ready(Err(err)) => {
                            status.state = FileState::Seeking;
                            Poll::Ready(Err(err))
                        }
                        Poll::Pending => Poll::Pending,
                    }
                }
                FileDataSource::Box { ref data, .. } => {
                    if cursor > data.len() as u64 {
                        Poll::Ready(Err(io::Error::from(io::ErrorKind::InvalidInput)))
                    } else {
                        status.state = FileState::Reading;
                        cx.waker().wake_by_ref();
                        Poll::Pending
                    }
                }
            },
            FileState::Reading => match status.data {
                FileDataSource::Reader { ref mut reader, .. } => {
                    let guard_pin = std::pin::pin!(reader);
                    match guard_pin.poll_read(cx, &mut buf[..end]) {
                        Poll::Ready(Ok(num_read)) => {
                            status.cursor += num_read as u64;
                            status.state = FileState::Seeking;
                            Poll::Ready(Ok(num_read))
                        }
                        Poll::Ready(Err(err)) => {
                            status.state = FileState::Seeking;
                            Poll::Ready(Err(err))
                        }
                        Poll::Pending => Poll::Pending,
                    }
                }
                FileDataSource::Box { ref data, .. } => {
                    let num_read = std::cmp::min(buf.len(), (data.len() as u64 - cursor) as usize);
                    buf[..num_read].copy_from_slice(&data[cursor as usize..][..num_read]);
                    status.cursor += num_read as u64;
                    status.state = FileState::Seeking;
                    Poll::Ready(Ok(num_read))
                }
            },
        }
    }
}

impl<R> Node<R> for File<R> {
    fn name(&self) -> String {
        self.status
            .lock()
            .map(|status| status.data.name())
            .unwrap_or_default()
    }

    fn get_type(&self) -> NodeType {
        NodeType::File
    }

    fn into_directory(self) -> Option<Directory<R>> {
        None
    }

    fn as_directory_ref(&self) -> Option<&Directory<R>> {
        None
    }

    fn as_directory_mut(&mut self) -> Option<&mut Directory<R>> {
        None
    }

    fn into_file(self) -> Option<File<R>> {
        Some(self)
    }

    fn as_file_ref(&self) -> Option<&File<R>> {
        Some(self)
    }

    fn as_file_mut(&mut self) -> Option<&mut File<R>> {
        Some(self)
    }
}
