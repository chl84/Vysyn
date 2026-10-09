//! Bounded, local-only text/uri-list decoding for Wayland file drops.

use std::os::unix::ffi::OsStringExt;
use std::path::PathBuf;

pub(super) const MAX_DROP_BYTES: usize = 1024 * 1024;
const MAX_DROP_FILES: usize = 1024;

pub(super) fn paths_from_uri_list(bytes: &[u8]) -> Vec<PathBuf> {
    if bytes.len() > MAX_DROP_BYTES {
        return Vec::new();
    }
    bytes
        .split(|byte| *byte == b'\n')
        .filter_map(local_path)
        .take(MAX_DROP_FILES)
        .collect()
}

fn local_path(line: &[u8]) -> Option<PathBuf> {
    let line = line.strip_suffix(b"\r").unwrap_or(line);
    if line.len() < 6 || !line[..5].eq_ignore_ascii_case(b"file:") {
        return None;
    }
    let mut path = &line[5..];
    if let Some(rest) = path.strip_prefix(b"//") {
        let slash = rest.iter().position(|byte| *byte == b'/')?;
        let host = &rest[..slash];
        if !host.is_empty() && !host.eq_ignore_ascii_case(b"localhost") {
            return None;
        }
        path = &rest[slash..];
    }
    if !path.starts_with(b"/") || path.iter().any(|byte| matches!(byte, b'?' | b'#')) {
        return None;
    }
    let mut decoded = Vec::with_capacity(path.len());
    let mut i = 0;
    while i < path.len() {
        let byte = if path[i] == b'%' {
            let high = hex(*path.get(i + 1)?)?;
            let low = hex(*path.get(i + 2)?)?;
            i += 3;
            high * 16 + low
        } else {
            let byte = path[i];
            i += 1;
            byte
        };
        if byte == 0 {
            return None;
        }
        decoded.push(byte);
    }
    Some(PathBuf::from(std::ffi::OsString::from_vec(decoded)))
}

fn hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}
