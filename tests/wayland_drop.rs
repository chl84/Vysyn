#![cfg(target_os = "linux")]

#[path = "../vendor/winit/src/platform_impl/linux/wayland/seat/drop_paths.rs"]
mod drop_paths;

use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;

use drop_paths::{MAX_DROP_BYTES, paths_from_uri_list};

#[test]
fn handles_comments_crlf_localhost_spaces_and_unicode() {
    let paths = paths_from_uri_list(
        b"# file manager export\r\n\r\nfile:///tmp/a%20b.png\r\nfile://localhost/tmp/bl%C3%A5.jpg\r\nFILE:/tmp/folder\n",
    );
    assert_eq!(
        paths,
        [
            PathBuf::from("/tmp/a b.png"),
            PathBuf::from("/tmp/blå.jpg"),
            PathBuf::from("/tmp/folder")
        ]
    );
}

#[test]
fn preserves_unix_filename_bytes_and_encoded_delimiters() {
    let paths = paths_from_uri_list(b"file:///tmp/non-utf8-%FF%23%3F.png\n");
    assert_eq!(paths[0].as_os_str().as_bytes(), b"/tmp/non-utf8-\xff#?.png");
}

#[test]
fn rejects_remote_relative_malformed_and_nul_paths() {
    for uri in [
        "https://example.com/image.png",
        "file://server/tmp/a.png",
        "file:relative.png",
        "file://localhost",
        "file:///tmp/a%00.png",
        "file:///tmp/a%Q1.png",
        "file:///tmp/a%2",
        "file:///tmp/a%",
        "file:///tmp/a.png?query",
        "file:///tmp/a.png#fragment",
        "file://user@localhost/tmp/a.png",
    ] {
        assert!(
            paths_from_uri_list(uri.as_bytes()).is_empty(),
            "accepted {uri}"
        );
    }
}

#[test]
fn invalid_entry_does_not_hide_later_local_file() {
    assert_eq!(
        paths_from_uri_list(b"file://server/a\nfile:///tmp/ok.png\n"),
        [PathBuf::from("/tmp/ok.png")]
    );
}

#[test]
fn bounds_complete_payload_and_number_of_files() {
    let mut oversized = b"file:///tmp/ok.png\n".to_vec();
    oversized.resize(MAX_DROP_BYTES + 1, b' ');
    assert!(paths_from_uri_list(&oversized).is_empty());
    let many = b"file:///tmp/ok.png\n".repeat(2048);
    assert_eq!(paths_from_uri_list(&many).len(), 1024);
}
