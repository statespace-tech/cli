//! Variant bundles: a reproducible gzipped tar of a ready-to-run directory.

use std::{
    fs::{self, File},
    io::{self, BufWriter, Seek, Write},
    path::Path,
};

use anyhow::bail;
use flate2::{Compression, GzBuilder};
use sha2::{Digest, Sha256};
use tempfile::NamedTempFile;

/// A packed bundle in a temporary file.
pub struct Packed {
    pub file: NamedTempFile,
    pub sha256: String,
    pub size: u64,
}

/// Pack `directory` without its `.git` directory. Entries are sorted and
/// carry no timestamps or owners, so the same files give the same hash.
pub fn pack(directory: &Path) -> anyhow::Result<Packed> {
    if !directory.is_dir() {
        bail!("{} is not a directory", directory.display());
    }
    let mut file = NamedTempFile::new()?;
    {
        let gzip = GzBuilder::new()
            .mtime(0)
            .write(BufWriter::new(file.as_file_mut()), Compression::default());
        let mut archive = tar::Builder::new(gzip);
        archive.follow_symlinks(false);
        append(&mut archive, directory, Path::new(""))?;
        archive.into_inner()?.finish()?.flush()?;
    }
    let mut hasher = Sha256::new();
    let handle = file.as_file_mut();
    handle.rewind()?;
    let size = io::copy(handle, &mut hasher)?;
    Ok(Packed {
        file,
        sha256: format!("{:x}", hasher.finalize()),
        size,
    })
}

fn append<W: Write>(
    archive: &mut tar::Builder<W>,
    root: &Path,
    relative: &Path,
) -> anyhow::Result<()> {
    let mut entries = fs::read_dir(root.join(relative))?
        .map(|entry| entry.map(|entry| entry.file_name()))
        .collect::<io::Result<Vec<_>>>()?;
    entries.sort();
    for name in entries {
        if relative.as_os_str().is_empty() && name == ".git" {
            continue;
        }
        let path = relative.join(&name);
        let full = root.join(&path);
        let metadata = fs::symlink_metadata(&full)?;
        let mut header = tar::Header::new_gnu();
        header.set_mtime(0);
        header.set_uid(0);
        header.set_gid(0);
        header.set_mode(mode(&metadata));
        if metadata.is_symlink() {
            header.set_entry_type(tar::EntryType::Symlink);
            header.set_size(0);
            archive.append_link(&mut header, &path, fs::read_link(&full)?)?;
        } else if metadata.is_dir() {
            header.set_entry_type(tar::EntryType::Directory);
            header.set_size(0);
            archive.append_data(&mut header, &path, io::empty())?;
            append(archive, root, &path)?;
        } else if metadata.is_file() {
            header.set_entry_type(tar::EntryType::Regular);
            header.set_size(metadata.len());
            archive.append_data(&mut header, &path, File::open(&full)?)?;
        }
    }
    Ok(())
}

#[cfg(unix)]
fn mode(metadata: &fs::Metadata) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    // Keep only whether the owner may execute, so umasks do not change hashes.
    match (
        metadata.is_dir(),
        metadata.permissions().mode() & 0o100 != 0,
    ) {
        (true, _) | (false, true) => 0o755,
        (false, false) => 0o644,
    }
}

#[cfg(not(unix))]
fn mode(metadata: &fs::Metadata) -> u32 {
    if metadata.is_dir() { 0o755 } else { 0o644 }
}

#[cfg(test)]
mod tests {
    use std::io::Read;

    use super::*;

    fn hash(mut reader: impl Read) -> anyhow::Result<(String, u64)> {
        let mut hasher = Sha256::new();
        let size = io::copy(&mut reader, &mut hasher)?;
        Ok((format!("{:x}", hasher.finalize()), size))
    }

    #[test]
    fn packing_is_reproducible() {
        let source = tempfile::tempdir().unwrap();
        fs::create_dir_all(source.path().join("app/.git")).unwrap();
        fs::create_dir_all(source.path().join(".git")).unwrap();
        fs::write(source.path().join(".git/HEAD"), "ref").unwrap();
        fs::write(source.path().join("app/main.py"), "print('hi')").unwrap();
        fs::write(source.path().join("run.sh"), "#!/bin/sh").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(
                source.path().join("run.sh"),
                fs::Permissions::from_mode(0o700),
            )
            .unwrap();
            std::os::unix::fs::symlink("app/main.py", source.path().join("link.py")).unwrap();
        }
        let first = pack(source.path()).unwrap();
        let second = pack(source.path()).unwrap();
        assert_eq!(first.sha256, second.sha256);
        assert_eq!(
            hash(File::open(first.file.path()).unwrap()).unwrap(),
            (first.sha256.clone(), first.size)
        );
    }
}
