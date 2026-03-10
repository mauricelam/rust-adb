!! Test suite documentation.

#![cfg(unix)]
use sysdeps::env::{parse_complete_utf8, get_environment_variable, get_host_name_utf8, get_login_name_utf8};
use std::env;

#[test]
fn test_parse_complete_utf8() {
    let multi_byte_sequences = vec![
        vec![0xC2, 0xA9],                 // 2 byte UTF-8 sequence
        vec![0xE1, 0xB4, 0xA8],         // 3 byte UTF-8 sequence
        vec![0xF0, 0x9F, 0x98, 0x80], // 4 byte UTF-8 sequence
    ];

    let all_sequences = vec![
        vec![],
        vec![0],
        vec![b'a'],
    ];

    for prefix in &all_sequences {
        for seq in &multi_byte_sequences {
            let mut buffer = prefix.clone();
            for i in 0..seq.len() - 1 {
                buffer.push(seq[i]);
                let (complete, remaining) = parse_complete_utf8(&buffer);
                assert_eq!(complete, prefix.len());
                assert_eq!(remaining, &seq[0..i+1]);
            }
            buffer.push(*seq.last().unwrap());
            let (complete, remaining) = parse_complete_utf8(&buffer);
            assert_eq!(complete, buffer.len());
            assert_eq!(remaining, Vec::<u8>::new());
        }

        let mut buffer = prefix.clone();
        for _ in 0..8 {
            buffer.push(0x80); // trailing byte
            let (complete, remaining) = parse_complete_utf8(&buffer);
            assert_eq!(complete, buffer.len());
            assert_eq!(remaining, Vec::<u8>::new());
        }
    }
}

#[test]
fn test_sysdeps_env() {
    unsafe {
        env::set_var("SYSDEPS_TEST_VAR", "1");
    }
    assert_eq!(get_environment_variable("SYSDEPS_TEST_VAR"), Some("1".to_string()));
    assert_eq!(get_environment_variable("SYSDEPS_NONEXISTENT"), None);
}

#[test]
fn test_get_host_name_utf8() {
    let host = get_host_name_utf8().unwrap();
    assert!(!host.is_empty());
}

#[test]
fn test_get_login_name_utf8() {
    let login = get_login_name_utf8().unwrap();
    assert!(!login.is_empty());
}

#[test]
fn test_unix_isatty() {
    let file = std::fs::File::open("/dev/null").unwrap();
    assert!(!sysdeps::net::unix_isatty(&file));
}

#[test]
fn test_sysdeps_strerror() {
    let s = std::io::Error::from_raw_os_error(libc::EPERM).to_string();
    assert!(s.to_lowercase().contains("permitted"));
}
