use std::{fs, io::{Read, Seek}};
#[cfg(not(target_os = "unknown"))]
use std::path::PathBuf;
use std::path::Path;
use zip::{read::ZipFile, ZipArchive};

/// A file from an arbitrary source
pub enum File<'a> {
    Zip(Box<ZipFile<'a>>),
    #[cfg(not(target_os = "unknown"))]
    FS(String, std::fs::File),
}

impl<'a> File<'a> {
    pub fn name(&self) -> &str {
        match self {
            File::Zip(zip) => zip.name(),
            #[cfg(not(target_os = "unknown"))]
            File::FS(string, _) => string,
        }
    }
}

impl<'a> Read for File<'a> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            File::Zip(zip_file) => zip_file.read(buf),
            #[cfg(not(target_os = "unknown"))]
            File::FS(_, file) => file.read(buf),
        }
    }
}

/// A source of files for the builder
#[derive(Debug)]
pub enum FSSource<R> {
    Zip(Box<ZipArchive<R>>),
    #[cfg(not(target_os = "unknown"))]
    FS(PathBuf),
}

impl<R> FSSource<R> {
    pub fn with_zip(zip: ZipArchive<R>) -> Self {
        Self::Zip(Box::new(zip))
    }

    #[cfg(not(target_os = "unknown"))]
    pub fn with_fs<P: AsRef<Path>>(path: P) -> Self {
        Self::FS(path.as_ref().to_path_buf())
    }
}

impl<R: Read + Seek> FSSource<R> {
    pub fn exists<P: AsRef<Path>>(&self, path: P) -> bool {
        match self {
            FSSource::Zip(zip) => {
                zip.index_for_path(path).is_some()
            },
            FSSource::FS(inner_path) => {
                let p = inner_path.join(path);
                p.exists()
            },
        }
    }

    pub fn is_dir<P: AsRef<Path>>(&self, path: P) -> bool {
        match self {
            FSSource::Zip(zip) => {
                zip.index_for_path(path)
                    .and_then(|idx| zip.by_index(idx).ok())
                    .map_or(false, |entry| entry.is_dir())
            },
            FSSource::FS(inner_path) => {
                let p = inner_path.join(path);
                p.is_dir()
            },
        }
    }

    pub fn is_file<P: AsRef<Path>>(&self, path: P) -> bool {
        match self {
            FSSource::Zip(zip) => {
                zip.index_for_path(path)
                    .and_then(|idx| zip.by_index(idx).ok())
                    .map_or(false, |entry| entry.is_file())
            },
            FSSource::FS(inner_path) => {
                let p = inner_path.join(path);
                p.is_file()
            },
        }
    }

    pub fn get_file<P: AsRef<Path>>(&mut self, path: P) -> eyre::Result<File> {
        match self {
            FSSource::Zip(zip) => Ok(File::Zip(Box::new(
                zip.by_name(
                    path.as_ref()
                        .to_str()
                        .ok_or(eyre::eyre!("Could not get path as &str"))?,
                )?,
            ))),
            #[cfg(not(target_os = "unknown"))]
            FSSource::FS(inner_path) => {
                let p = inner_path.join(path);
                Ok(File::FS(
                    p.file_name()
                        .ok_or(eyre::eyre!("Could get get path as &OsStr"))?
                        .to_str()
                        .ok_or(eyre::eyre!("Could get get &OsStr as &str"))?
                        .to_string(),
                    std::fs::OpenOptions::new().read(true).open(p)?,
                ))
            }
        }
    }

    pub fn get_names<P: AsRef<Path>>(&self, path: P) -> eyre::Result<Vec<String>> {
        let mut names: Vec<String> = Vec::new();
        match self {
            FSSource::Zip(_) => {
                return Err(eyre::eyre!("Unsupported operation on ZipArchive: get_names"))
            },
            FSSource::FS(inner_path) => {
                let p = inner_path.join(path);
                for entry in fs::read_dir(p)? {
                    let entry = entry?;
                    let entry_path = entry.path();
                    let file_name = entry_path.file_name().expect("Entry has no name");
                    names.push(String::from(file_name.to_str().unwrap()));
                }
            },
        }
        Ok(names)
    }
}
