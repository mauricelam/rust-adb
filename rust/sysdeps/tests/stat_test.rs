//! Tests for sysdeps

#![cfg(unix)]
use libc;
use std::fs;
use std::io::Write;
use tempfile::tempdir;

#[test]
fn test_sysdeps_stat() {
    let td = tempdir().unwrap();
    let tf_path = td.path().join("test_file");
    {
        let mut tf = fs::File::create(&tf_path).unwrap();
        tf.write_all(b"hello").unwrap();
    }

    let st = fs::metadata(td.path()).unwrap();
    assert!(st.is_dir());

    // Rust's metadata on a directory with a trailing slash
    let dir_path_str = td.path().to_str().unwrap().to_string() + "/";
    let st = fs::metadata(&dir_path_str).unwrap();
    assert!(st.is_dir());

    let nonexistent_path = td.path().join("nonexistent");
    assert!(fs::metadata(&nonexistent_path).is_err());

    let nonexistent_path_slash = nonexistent_path.to_str().unwrap().to_string() + "/";
    let res = fs::metadata(&nonexistent_path_slash);
    assert!(res.is_err());
    assert_eq!(res.unwrap_err().raw_os_error(), Some(libc::ENOENT));

    let st = fs::metadata(&tf_path).unwrap();
    assert!(st.is_file());

    let tf_path_slash = tf_path.to_str().unwrap().to_string() + "/";
    // On Unix, stat() on a file with a trailing slash should fail with ENOTDIR
    let res = fs::metadata(&tf_path_slash);
    assert!(res.is_err());
    assert_eq!(res.unwrap_err().raw_os_error(), Some(libc::ENOTDIR));
}
