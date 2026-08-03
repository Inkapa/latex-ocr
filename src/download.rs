//! Shared helpers for downloading and extracting tool archives.

use std::fs::{self, File};
use std::path::{Component, Path, PathBuf};

use log::info;

/// How an archive is compressed.
#[derive(Clone, Copy, PartialEq)]
pub enum ArchiveKind {
    Zip,
    TarGz,
}

/// Downloads `url` into a temporary file and extracts it into `dest`.
pub fn download_and_extract(url: &str, dest: &Path, kind: ArchiveKind) -> Result<(), String> {
    let client = reqwest::blocking::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(20))
        .timeout(std::time::Duration::from_secs(600))
        .build()
        .map_err(|e| format!("cannot build http client: {e}"))?;

    let tmp = tempfile::tempdir().map_err(|e| e.to_string())?;
    let archive_path = tmp.path().join("archive");
    let mut response = client
        .get(url)
        .send()
        .map_err(|e| format!("download failed: {e}"))?
        .error_for_status()
        .map_err(|e| format!("download failed: {e}"))?;
    let mut file = File::create(&archive_path).map_err(|e| e.to_string())?;
    std::io::copy(&mut response, &mut file).map_err(|e| format!("download failed: {e}"))?;
    drop(file);
    info!(
        "downloaded {url} ({} bytes)",
        fs::metadata(&archive_path).map(|m| m.len()).unwrap_or(0)
    );

    match kind {
        ArchiveKind::Zip => extract_zip(&archive_path, dest)?,
        ArchiveKind::TarGz => extract_tar_gz(&archive_path, dest)?,
    }
    Ok(())
}

/// Strips leading separators, `.` and `..` components from an archive path so
/// that extraction cannot escape the destination directory.
pub fn sanitize_path(path: PathBuf) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        if let Component::Normal(part) = component {
            out.push(part);
        }
    }
    out
}

fn extract_zip(archive: &Path, dest: &Path) -> Result<(), String> {
    let file = File::open(archive).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
    fs::create_dir_all(dest).map_err(|e| e.to_string())?;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).map_err(|e| e.to_string())?;
        let out_path = dest.join(sanitize_path(entry.mangled_name()));
        if entry.is_dir() {
            fs::create_dir_all(&out_path).map_err(|e| e.to_string())?;
        } else {
            if let Some(parent) = out_path.parent() {
                fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            let mut out = File::create(&out_path).map_err(|e| e.to_string())?;
            std::io::copy(&mut entry, &mut out).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

fn extract_tar_gz(archive: &Path, dest: &Path) -> Result<(), String> {
    let file = File::open(archive).map_err(|e| e.to_string())?;
    let decoder = flate2::read::GzDecoder::new(file);
    let mut tar = tar::Archive::new(decoder);
    fs::create_dir_all(dest).map_err(|e| e.to_string())?;
    for entry in tar.entries().map_err(|e| e.to_string())? {
        let mut entry = entry.map_err(|e| e.to_string())?;
        entry
            .unpack_in(dest)
            .map_err(|e| format!("cannot extract archive: {e}"))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_path_strips_ancestors() {
        assert_eq!(
            sanitize_path(PathBuf::from("a/../b/./c.txt")),
            PathBuf::from("a/b/c.txt")
        );
        assert_eq!(
            sanitize_path(PathBuf::from("/etc/passwd")),
            PathBuf::from("etc/passwd")
        );
    }
}
