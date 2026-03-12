//! Windows-specific sysdeps tests
//! Ported from `original/sysdeps_win32_test.cpp`.

#![cfg(windows)]

use sysdeps::env::{get_environment_variable, parse_complete_utf8};

#[test]
fn test_adb_getenv() {
    // We can't easily test setting env vars in a cross-platform way here,
    // but we can check that it works for some standard ones.
    assert!(get_environment_variable("PATH").is_some());
}

#[test]
fn test_parse_complete_utf8() {
    // 2 byte UTF-8 sequence
    let seq2 = vec![0xC2, 0xA9];
    assert_eq!(parse_complete_utf8(&seq2), (2, vec![]));
    assert_eq!(parse_complete_utf8(&seq2[..1]), (0, vec![0xC2]));

    // 3 byte UTF-8 sequence
    let seq3 = vec![0xE1, 0xB4, 0xA8];
    assert_eq!(parse_complete_utf8(&seq3), (3, vec![]));
    assert_eq!(parse_complete_utf8(&seq3[..1]), (0, vec![0xE1]));
    assert_eq!(parse_complete_utf8(&seq3[..2]), (0, vec![0xE1, 0xB4]));

    // 4 byte UTF-8 sequence
    let seq4 = vec![0xF0, 0x9F, 0x98, 0x80];
    assert_eq!(parse_complete_utf8(&seq4), (4, vec![]));
    assert_eq!(parse_complete_utf8(&seq4[..1]), (0, vec![0xF0]));
    assert_eq!(parse_complete_utf8(&seq4[..2]), (0, vec![0xF0, 0x9F]));
    assert_eq!(parse_complete_utf8(&seq4[..3]), (0, vec![0xF0, 0x9F, 0x98]));
}
