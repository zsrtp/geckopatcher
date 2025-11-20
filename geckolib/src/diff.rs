use bsdiff::{diff as bs_diff, patch as bs_patch};
use futures::{AsyncRead, AsyncSeek};
use sha3::Digest;
use std::io::{BufRead, Write};

use crate::vfs::GeckoFS;

#[doc = "Extract the diff of two u8 slices and compresses them. If the diff is empty, returns None"]
pub fn diff<R1: AsRef<[u8]>, R2: AsRef<[u8]>>(
    original: R1,
    patched: R2,
) -> std::io::Result<Option<Vec<u8>>> {
    if original.as_ref().len() == patched.as_ref().len() && original.as_ref() == patched.as_ref() {
        return Ok(None);
    }
    let mut out_diff = Vec::new();
    bs_diff(original.as_ref(), patched.as_ref(), &mut out_diff)?;
    let mut encoder =
        deko::AnyEncoder::new(Vec::new(), deko::Format::Bz, deko::write::Compression::Best)?;
    encoder.write_all(&out_diff)?;
    let out = encoder.finish()?;
    Ok(Some(out))
}

pub fn patch<R1: AsRef<[u8]>, R2: BufRead>(
    original: R1,
    patch: R2,
) -> std::io::Result<Vec<u8>> {
    let mut decoder = deko::AnyDecoder::new(patch);
    let mut out = Vec::new();
    bs_patch(original.as_ref(), &mut decoder, &mut out)?;
    Ok(out)
}

pub async fn calculate_checksum<R>(fs: &mut GeckoFS<R>) -> Result<[u8; 32], futures::io::Error>
where
    R: AsyncRead + AsyncSeek + Unpin,
{
    let mut checksum = setsum::Setsum::default();
    for (root, node) in std::iter::repeat(fs.root)
        .zip(fs.iter_path_dfs(fs.root))
        .collect::<Vec<_>>()
        .iter()
        .chain(
            std::iter::repeat(fs.sys)
                .zip(fs.iter_path_dfs(fs.sys))
                .collect::<Vec<_>>()
                .iter(),
        )
    {
        match fs.get_file_mut(*root, node) {
            Some(file) => {
                use futures::{AsyncReadExt, AsyncSeekExt};

                file.seek(std::io::SeekFrom::Start(0)).await?;
                let mut buf = Vec::new();
                file.read_to_end(&mut buf).await?;
                let mut hasher = sha3::Sha3_256::new();
                hasher.update(buf);
                checksum.insert(hasher.finalize().as_slice());
            }
            None => continue,
        }
    }
    Ok(checksum.digest())
}