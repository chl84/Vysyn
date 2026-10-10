use anyhow::{Context, Result, bail};
use std::{
    fs,
    path::{Path, PathBuf},
    time::SystemTime,
};

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub struct FileKey {
    pub path: PathBuf,
    pub len: u64,
    pub modified: Option<SystemTime>,
}

impl FileKey {
    pub fn read(path: &Path) -> Result<Self> {
        let m = fs::metadata(path).with_context(|| format!("cannot read {}", path.display()))?;
        if !m.is_file() {
            bail!("{} is not a regular file", path.display());
        }
        Ok(Self {
            path: path.to_path_buf(),
            len: m.len(),
            modified: m.modified().ok(),
        })
    }
}

pub fn supported_extension(path: &Path) -> bool {
    path.extension().and_then(|x| x.to_str()).is_some_and(|x| {
        matches!(
            x.to_ascii_lowercase().as_str(),
            "jpg"
                | "jpeg"
                | "png"
                | "webp"
                | "gif"
                | "bmp"
                | "tif"
                | "tiff"
                | "heic"
                | "heif"
                | "avif"
                | "svg"
                | "ico"
                | "pnm"
                | "pbm"
                | "pgm"
                | "ppm"
                | "pam"
                | "tga"
                | "targa"
                | "icb"
                | "vda"
                | "vst"
                | "tpic"
                | "ff"
                | "farbfeld"
                | "dds"
                | "hdr"
                | "rgbe"
                | "psd"
        )
    })
}

pub fn scan(directory: &Path, cancelled: &dyn Fn() -> bool) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    let mut path_bytes = 0;
    for (count, entry) in fs::read_dir(directory)
        .with_context(|| format!("cannot open directory {}", directory.display()))?
        .enumerate()
    {
        if cancelled() {
            bail!("request superseded");
        }
        if count >= 100_000 {
            bail!("directory exceeds the 100,000 entry limit");
        }
        let Ok(entry) = entry else {
            continue;
        };
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        // Sniff only a bounded header. This includes extensionless and mislabeled files.
        if crate::decode::detect_file(&path).is_ok() {
            path_bytes += path.as_os_str().len() + 64;
            if path_bytes > 16 * 1024 * 1024 {
                bail!("directory image list exceeds 16 MiB");
            }
            files.push(path);
        }
    }
    files.sort_by(|a, b| a.file_name().cmp(&b.file_name()));
    Ok(files)
}

pub fn neighbor(files: &[PathBuf], current: &Path, direction: i32) -> Option<PathBuf> {
    if files.is_empty() {
        return None;
    }
    let index = files.binary_search_by(|p| p.file_name().cmp(&current.file_name()));
    let next = match (index, direction >= 0) {
        (Ok(i), true) => (i + 1) % files.len(),
        (Ok(i), false) => (i + files.len() - 1) % files.len(),
        (Err(i), true) => i % files.len(),
        (Err(i), false) => (i + files.len() - 1) % files.len(),
    };
    Some(files[next].clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn neighbors_wrap_and_recover_after_deletion() {
        let files: Vec<_> = ["a.png", "c.png", "d.png"].map(PathBuf::from).into();
        assert_eq!(
            neighbor(&files, Path::new("d.png"), 1),
            Some("a.png".into())
        );
        assert_eq!(
            neighbor(&files, Path::new("a.png"), -1),
            Some("d.png".into())
        );
        assert_eq!(
            neighbor(&files, Path::new("b.png"), 1),
            Some("c.png".into())
        );
        assert_eq!(
            neighbor(&files, Path::new("b.png"), -1),
            Some("a.png".into())
        );
        assert!(neighbor(&[], Path::new("x"), 1).is_none());
    }
    #[test]
    fn fingerprint_changes_when_file_is_modified() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("x");
        fs::write(&p, [1]).unwrap();
        let a = FileKey::read(&p).unwrap();
        fs::write(&p, [1, 2]).unwrap();
        assert_ne!(a, FileKey::read(&p).unwrap());
    }
}
