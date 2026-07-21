//! Archive extraction helpers for P04 binary installs.
//!
//! Three shapes are supported:
//!
//! * `extract_tar_gz` — `.tar.gz` / `.tgz` via `flate2 + tar`.
//! * `extract_zip`    — `.zip` via the `zip` crate.
//! * `install_raw`    — single-file asset (no archive) copied into place.
//!
//! After extraction, [`locate_executable`] walks the staging tree and
//! returns the path of the extension's `flox-<name>` binary, promoting its
//! mode to include the executable bits on Unix.

use std::path::{Component, Path, PathBuf};
use std::{fs, io};

use thiserror::Error;

#[derive(Debug, Error)]
pub enum ArchiveError {
    #[error("failed to open archive {path}: {source}")]
    Open {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("tar extraction failed for {path}: {source}")]
    Tar {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("zip extraction failed for {path}: {detail}")]
    Zip { path: PathBuf, detail: String },
    #[error("filesystem error during archive handling: {0}")]
    Io(#[from] io::Error),
    #[error(
        "archive from {archive} did not contain an executable named 'flox-{name}' \
        (or a file matching '{name}')"
    )]
    ExecutableMissing { archive: PathBuf, name: String },
    #[error("archive {archive} contains unsafe entry path '{entry}'")]
    UnsafePath { archive: PathBuf, entry: String },
}

/// Reject absolute paths and any `..` / root / prefix components. Plain
/// `./foo` / `foo/bar` / empty paths are allowed. This is the sole gate
/// between a release-asset archive and the staging directory — any entry
/// that escapes here will unpack outside staging.
fn is_safe_relative(p: &Path) -> bool {
    if p.is_absolute() {
        return false;
    }
    p.components()
        .all(|c| matches!(c, Component::Normal(_) | Component::CurDir))
}

/// Extract `archive` (a `.tar.gz` or `.tgz`) into `staging`. The staging
/// directory must already exist.
///
/// Entries are iterated manually so that every path can be vetted before
/// unpacking: any absolute path or `..` component yields
/// [`ArchiveError::UnsafePath`] and aborts extraction. Non-regular,
/// non-directory entries (symlinks, hardlinks, devices, fifos) are
/// silently dropped — extension payloads never need them, and allowing
/// symlink entries would reopen the tar-slip door by redirecting
/// subsequent writes outside staging.
pub fn extract_tar_gz(archive: &Path, staging: &Path) -> Result<(), ArchiveError> {
    let file = fs::File::open(archive).map_err(|source| ArchiveError::Open {
        path: archive.to_path_buf(),
        source,
    })?;
    let gz = flate2::read::GzDecoder::new(file);
    let mut tar = tar::Archive::new(gz);

    let entries = tar.entries().map_err(|source| ArchiveError::Tar {
        path: archive.to_path_buf(),
        source,
    })?;

    for entry in entries {
        let mut entry = entry.map_err(|source| ArchiveError::Tar {
            path: archive.to_path_buf(),
            source,
        })?;

        let etype = entry.header().entry_type();
        if !(etype.is_file() || etype.is_dir()) {
            continue;
        }

        let entry_path = entry
            .path()
            .map_err(|source| ArchiveError::Tar {
                path: archive.to_path_buf(),
                source,
            })?
            .into_owned();

        if !is_safe_relative(&entry_path) {
            return Err(ArchiveError::UnsafePath {
                archive: archive.to_path_buf(),
                entry: entry_path.display().to_string(),
            });
        }

        let dest = staging.join(&entry_path);
        entry.unpack(&dest).map_err(|source| ArchiveError::Tar {
            path: archive.to_path_buf(),
            source,
        })?;
    }
    Ok(())
}

/// Extract `archive` (a `.zip`) into `staging`. The staging directory must
/// already exist.
///
/// Mirrors [`extract_tar_gz`]: entries are validated via
/// [`zip::read::ZipFile::enclosed_name`] (which rejects absolute paths and
/// `..` escapes), symlinks are dropped, and file contents are written with
/// their recorded unix permissions where present.
pub fn extract_zip(archive: &Path, staging: &Path) -> Result<(), ArchiveError> {
    let file = fs::File::open(archive).map_err(|source| ArchiveError::Open {
        path: archive.to_path_buf(),
        source,
    })?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| ArchiveError::Zip {
        path: archive.to_path_buf(),
        detail: e.to_string(),
    })?;

    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).map_err(|e| ArchiveError::Zip {
            path: archive.to_path_buf(),
            detail: e.to_string(),
        })?;

        if entry.is_symlink() {
            continue;
        }

        let raw_name = entry.name().to_string();
        let Some(rel) = entry.enclosed_name() else {
            return Err(ArchiveError::UnsafePath {
                archive: archive.to_path_buf(),
                entry: raw_name,
            });
        };
        if !is_safe_relative(&rel) {
            return Err(ArchiveError::UnsafePath {
                archive: archive.to_path_buf(),
                entry: raw_name,
            });
        }

        let dest = staging.join(&rel);
        if entry.is_dir() {
            fs::create_dir_all(&dest)?;
            continue;
        }
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut out = fs::File::create(&dest)?;
        io::copy(&mut entry, &mut out)?;

        #[cfg(unix)]
        if let Some(mode) = entry.unix_mode() {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&dest, fs::Permissions::from_mode(mode))?;
        }
    }
    Ok(())
}

/// Copy `src` into `staging` as `flox-<name>` (no archive involved) and
/// ensure it is executable. Returns the destination path.
pub fn install_raw(src: &Path, staging: &Path, name: &str) -> Result<PathBuf, ArchiveError> {
    let dest = staging.join(format!("flox-{name}"));
    fs::copy(src, &dest)?;
    chmod_executable(&dest)?;
    Ok(dest)
}

/// Walk the extracted tree rooted at `staging` and return the path of the
/// extension's executable (a file named `flox-<name>` or `<name>` at any
/// depth). On Unix the file's permissions are promoted to include the
/// executable bits so the dispatch path can exec it.
pub fn locate_executable(staging: &Path, name: &str) -> Result<PathBuf, ArchiveError> {
    let prefixed = format!("flox-{name}");
    let mut candidate: Option<PathBuf> = None;
    for entry in walkdir::WalkDir::new(staging)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        if !entry.file_type().is_file() {
            continue;
        }
        let file_name = match entry.file_name().to_str() {
            Some(s) => s,
            None => continue,
        };
        if file_name == prefixed {
            candidate = Some(entry.path().to_path_buf());
            break;
        }
        if candidate.is_none() && file_name == name {
            candidate = Some(entry.path().to_path_buf());
        }
    }
    let Some(path) = candidate else {
        return Err(ArchiveError::ExecutableMissing {
            archive: staging.to_path_buf(),
            name: name.to_string(),
        });
    };
    chmod_executable(&path)?;
    // Ensure the staging-root binary exists at `flox-<name>` so install_dir
    // can dispatch it without knowing the archive's internal layout. If it's
    // already at the root with the right name, no move is needed.
    let root_exe = staging.join(&prefixed);
    if path != root_exe {
        // Copy (not rename) so we don't disturb on-disk layout if authors
        // ship a bundle of support files alongside the binary.
        fs::copy(&path, &root_exe)?;
        chmod_executable(&root_exe)?;
    }
    Ok(root_exe)
}

#[cfg(unix)]
fn chmod_executable(p: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = fs::metadata(p)?.permissions();
    let mode = perms.mode() | 0o111;
    perms.set_mode(mode);
    fs::set_permissions(p, perms)
}

#[cfg(not(unix))]
fn chmod_executable(_p: &Path) -> io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use pretty_assertions::assert_eq;
    use tempfile::TempDir;

    use super::*;

    fn build_tar_gz(dest: &Path, entries: &[(&str, &[u8])]) {
        let file = fs::File::create(dest).unwrap();
        let gz = flate2::write::GzEncoder::new(file, flate2::Compression::default());
        let mut tar = tar::Builder::new(gz);
        for (name, body) in entries {
            let mut header = tar::Header::new_gnu();
            header.set_path(name).unwrap();
            header.set_size(body.len() as u64);
            header.set_mode(0o755);
            header.set_cksum();
            tar.append(&header, *body).unwrap();
        }
        tar.into_inner().unwrap().finish().unwrap();
    }

    /// Construct a tar.gz with one regular-file entry whose `name` bypasses
    /// the `tar` crate's path validation (which refuses absolute paths and
    /// `..` components up-front). We emit the 512-byte ustar header with
    /// the raw name, then the data, padded to a 512-byte boundary, plus the
    /// two-empty-record end-of-archive trailer.
    fn build_tar_gz_raw(dest: &Path, name: &str, body: &[u8]) {
        let mut record = [0u8; 512];
        let name_bytes = name.as_bytes();
        let n = name_bytes.len().min(100);
        record[..n].copy_from_slice(&name_bytes[..n]);
        record[100..108].copy_from_slice(b"0000644\0");
        record[108..116].copy_from_slice(b"0000000\0");
        record[116..124].copy_from_slice(b"0000000\0");
        let size_octal = format!("{:011o}\0", body.len());
        record[124..136].copy_from_slice(size_octal.as_bytes());
        record[136..148].copy_from_slice(b"00000000000\0");
        record[148..156].copy_from_slice(b"        ");
        record[156] = b'0';
        record[257..263].copy_from_slice(b"ustar\0");
        record[263..265].copy_from_slice(b"00");
        let sum: u32 = record.iter().map(|&b| u32::from(b)).sum();
        let chksum = format!("{sum:06o}\0 ");
        record[148..156].copy_from_slice(chksum.as_bytes());

        let mut raw = Vec::new();
        raw.extend_from_slice(&record);
        raw.extend_from_slice(body);
        let pad = (512 - (body.len() % 512)) % 512;
        raw.extend(std::iter::repeat_n(0u8, pad));
        raw.extend(std::iter::repeat_n(0u8, 1024));

        let file = fs::File::create(dest).unwrap();
        let mut gz = flate2::write::GzEncoder::new(file, flate2::Compression::default());
        gz.write_all(&raw).unwrap();
        gz.finish().unwrap();
    }

    fn build_tar_gz_with_symlink(dest: &Path, link_name: &str, target: &str) {
        let file = fs::File::create(dest).unwrap();
        let gz = flate2::write::GzEncoder::new(file, flate2::Compression::default());
        let mut tar = tar::Builder::new(gz);
        let mut header = tar::Header::new_gnu();
        header.set_size(0);
        header.set_mode(0o777);
        header.set_entry_type(tar::EntryType::Symlink);
        header.set_link_name(target).unwrap();
        header.set_cksum();
        tar.append_data(&mut header, link_name, std::io::empty())
            .unwrap();
        tar.into_inner().unwrap().finish().unwrap();
    }

    fn build_zip(dest: &Path, entries: &[(&str, &[u8])]) {
        let file = fs::File::create(dest).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let options: zip::write::SimpleFileOptions = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .unix_permissions(0o755);
        for (name, body) in entries {
            zip.start_file(*name, options).unwrap();
            zip.write_all(body).unwrap();
        }
        zip.finish().unwrap();
    }

    #[test]
    fn extract_tar_gz_round_trips_entries() {
        let temp = TempDir::new().unwrap();
        let archive = temp.path().join("a.tar.gz");
        let staging = temp.path().join("staging");
        fs::create_dir(&staging).unwrap();
        build_tar_gz(&archive, &[
            ("flox-hello", b"#!/bin/sh\necho hi\n"),
            ("README.md", b"hello\n"),
        ]);

        extract_tar_gz(&archive, &staging).unwrap();
        assert!(staging.join("flox-hello").exists());
        assert!(staging.join("README.md").exists());
    }

    #[test]
    fn extract_zip_round_trips_entries() {
        let temp = TempDir::new().unwrap();
        let archive = temp.path().join("a.zip");
        let staging = temp.path().join("staging");
        fs::create_dir(&staging).unwrap();
        build_zip(&archive, &[
            ("flox-hello", b"#!/bin/sh\necho hi\n"),
            ("README.md", b"hello\n"),
        ]);

        extract_zip(&archive, &staging).unwrap();
        assert!(staging.join("flox-hello").exists());
        assert!(staging.join("README.md").exists());
    }

    #[test]
    fn install_raw_copies_and_chmods() {
        let temp = TempDir::new().unwrap();
        let src = temp.path().join("payload");
        let staging = temp.path().join("staging");
        fs::create_dir(&staging).unwrap();
        fs::write(&src, b"#!/bin/sh\necho hi\n").unwrap();

        let dest = install_raw(&src, &staging, "hello").unwrap();
        assert_eq!(dest, staging.join("flox-hello"));
        assert_eq!(fs::read(&dest).unwrap(), b"#!/bin/sh\necho hi\n");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&dest).unwrap().permissions().mode();
            assert!(mode & 0o111 != 0, "executable bit not set: mode={mode:o}");
        }
    }

    #[test]
    fn locate_executable_finds_nested_flox_prefixed_file() {
        let temp = TempDir::new().unwrap();
        let staging = temp.path().join("s");
        fs::create_dir_all(staging.join("bin")).unwrap();
        fs::write(staging.join("bin").join("flox-hello"), b"#!/bin/sh\n").unwrap();

        let exe = locate_executable(&staging, "hello").unwrap();
        assert_eq!(exe, staging.join("flox-hello"));
        assert!(exe.exists());
    }

    #[test]
    fn locate_executable_falls_back_to_bare_name() {
        let temp = TempDir::new().unwrap();
        let staging = temp.path().join("s");
        fs::create_dir_all(&staging).unwrap();
        fs::write(staging.join("hello"), b"#!/bin/sh\n").unwrap();

        let exe = locate_executable(&staging, "hello").unwrap();
        assert_eq!(exe, staging.join("flox-hello"));
    }

    #[test]
    fn extract_tar_gz_rejects_dotdot_escape() {
        let temp = TempDir::new().unwrap();
        let archive = temp.path().join("evil.tar.gz");
        let staging = temp.path().join("staging");
        fs::create_dir(&staging).unwrap();
        build_tar_gz_raw(&archive, "../escape", b"pwned");

        let err = extract_tar_gz(&archive, &staging).unwrap_err();
        assert!(
            matches!(err, ArchiveError::UnsafePath { .. }),
            "expected UnsafePath, got {err:?}"
        );
        assert!(!temp.path().join("escape").exists());
    }

    #[test]
    fn extract_tar_gz_rejects_absolute_path() {
        let temp = TempDir::new().unwrap();
        let archive = temp.path().join("evil.tar.gz");
        let staging = temp.path().join("staging");
        fs::create_dir(&staging).unwrap();
        build_tar_gz_raw(&archive, "/tmp/flox-absolute-escape", b"pwned");

        let err = extract_tar_gz(&archive, &staging).unwrap_err();
        assert!(
            matches!(err, ArchiveError::UnsafePath { .. }),
            "expected UnsafePath, got {err:?}"
        );
        assert!(!PathBuf::from("/tmp/flox-absolute-escape").exists());
    }

    #[test]
    fn extract_tar_gz_silently_skips_symlink_entries() {
        let temp = TempDir::new().unwrap();
        let archive = temp.path().join("evil.tar.gz");
        let staging = temp.path().join("staging");
        fs::create_dir(&staging).unwrap();
        build_tar_gz_with_symlink(&archive, "flox-hello", "/etc/passwd");

        extract_tar_gz(&archive, &staging).unwrap();
        assert!(
            !staging.join("flox-hello").exists(),
            "symlink must not be materialized"
        );
    }

    #[test]
    fn extract_zip_rejects_dotdot_escape() {
        let temp = TempDir::new().unwrap();
        let archive = temp.path().join("evil.zip");
        let staging = temp.path().join("staging");
        fs::create_dir(&staging).unwrap();
        build_zip(&archive, &[("../escape", b"pwned")]);

        let err = extract_zip(&archive, &staging).unwrap_err();
        assert!(
            matches!(err, ArchiveError::UnsafePath { .. }),
            "expected UnsafePath, got {err:?}"
        );
        assert!(!temp.path().join("escape").exists());
    }

    #[test]
    fn extract_zip_rejects_absolute_path() {
        let temp = TempDir::new().unwrap();
        let archive = temp.path().join("evil.zip");
        let staging = temp.path().join("staging");
        fs::create_dir(&staging).unwrap();
        build_zip(&archive, &[("/tmp/flox-absolute-escape-zip", b"pwned")]);

        let err = extract_zip(&archive, &staging).unwrap_err();
        assert!(
            matches!(err, ArchiveError::UnsafePath { .. }),
            "expected UnsafePath, got {err:?}"
        );
        assert!(!PathBuf::from("/tmp/flox-absolute-escape-zip").exists());
    }

    #[test]
    fn locate_executable_errors_when_no_match() {
        let temp = TempDir::new().unwrap();
        let staging = temp.path().join("s");
        fs::create_dir_all(&staging).unwrap();
        fs::write(staging.join("something-else"), b"").unwrap();
        let err = locate_executable(&staging, "hello").unwrap_err();
        assert!(matches!(err, ArchiveError::ExecutableMissing { .. }));
    }
}
