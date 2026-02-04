/*
 * Copyright (C) 2015 The Android Open Source Project
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *      http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

//! This crate provides various utility functions for ADB, ported from the C++ implementation.
//!
//! The functions here are primarily ported from:
//! - `original/adb_utils.h`
//! - `original/adb_utils.cpp`

use adb_types::{Amessage, Apacket};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};

/// Ported from `original/adb_utils.h`: `StripTrailingNulls`
pub fn strip_trailing_nulls(s: &str) -> &str {
    s.trim_end_matches('\0')
}

/// Ported from original/adb_utils.h: ParseUint
pub fn parse_uint<T: std::str::FromStr>(s: &str) -> Option<(T, &str)> {
    let end = s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len());
    if end == 0 {
        return None;
    }
    let val = s[..end].parse::<T>().ok()?;
    Some((val, &s[end..]))
}

/// Ported from original/adb_utils.cpp: directory_exists
pub fn directory_exists<P: AsRef<Path>>(path: P) -> bool {
    path.as_ref().is_dir()
}

/// Ported from original/adb_utils.cpp: escape_arg
pub fn escape_arg(s: &str) -> String {
    let mut result = String::new();
    result.push('\'');
    result.push_str(&s.replace('\'', "'\\''"));
    result.push('\'');
    result
}

/// Ported from original/adb_utils.cpp: mkdirs
pub fn mkdirs<P: AsRef<Path>>(path: P) -> std::io::Result<()> {
    fs::create_dir_all(path)
}

/// Ported from original/adb_utils.cpp: dump_hex
pub fn dump_hex(data: &[u8]) -> String {
    let mut byte_count = data.len();
    let truncate_len = 16;
    let mut truncated = false;
    if byte_count > truncate_len {
        byte_count = truncate_len;
        truncated = true;
    }

    let p = &data[..byte_count];
    let mut line = String::new();
    for &b in p {
        line.push_str(&format!("{:02x}", b));
    }
    line.push(' ');

    for &b in p {
        if b.is_ascii_graphic() || b == b' ' {
            line.push(b as char);
        } else {
            line.push('.');
        }
    }

    if truncated {
        line.push_str(" [truncated]");
    }

    line
}

/// Ported from original/adb_utils.cpp: dump_header
pub fn dump_header(msg: &Amessage) -> String {
    let command = msg.command;
    let len = msg.data_length;
    let mut cmd = String::new();

    for n in 0..4 {
        let b = (command >> (n * 8)) & 255;
        if b < 32 || b >= 127 {
            cmd = format!("{:08x}", command);
            break;
        }
        cmd.push(b as u8 as char);
    }

    let arg0 = if msg.arg0 < 256 {
        format!("{}", msg.arg0)
    } else {
        format!("0x{:x}", msg.arg0)
    };

    let arg1 = if msg.arg1 < 256 {
        format!("{}", msg.arg1)
    } else {
        format!("0x{:x}", msg.arg1)
    };

    format!("[{}] arg0={} arg1={} (len={}) ", cmd, arg0, arg1, len)
}

/// Ported from original/adb_utils.cpp: dump_packet
pub fn dump_packet(name: &str, func: &str, p: &Apacket) -> String {
    let mut result = format!("{}: {}: ", name, func);
    result.push_str(&dump_header(&p.msg));
    result.push_str(&dump_hex(p.payload.get_ref()));
    result
}

/// Ported from original/adb_utils.cpp: forward_targets_are_valid
pub fn forward_targets_are_valid(source: &str, dest: &str) -> Result<(), String> {
    if source.starts_with("tcp:") {
        if let Ok(port) = source[4..].parse::<i32>() {
            if port < 0 {
                return Err(format!("Invalid source port: '{}'", &source[4..]));
            }
        } else {
            return Err(format!("Invalid source port: '{}'", &source[4..]));
        }
    }

    if dest.starts_with("tcp:") {
        if let Ok(port) = dest[4..].parse::<i32>() {
            if port <= 0 {
                return Err(format!("Invalid destination port: '{}'", &dest[4..]));
            }
        } else {
            return Err(format!("Invalid destination port: '{}'", &dest[4..]));
        }
    }

    Ok(())
}

/// Ported from original/adb_utils.cpp: adb_get_homedir_path
pub fn adb_get_homedir_path() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        // On Windows, use the HOME environment variable if it exists,
        // otherwise use the user profile directory.
        if let Ok(home) = std::env::var("HOME") {
            return Some(PathBuf::from(home));
        }
        if let Ok(userprofile) = std::env::var("USERPROFILE") {
            return Some(PathBuf::from(userprofile));
        }
        None
    }
    #[cfg(unix)]
    {
        if let Ok(home) = std::env::var("HOME") {
            return Some(PathBuf::from(home));
        }
        // Fallback to getpwuid_r is not easily available in pure Rust
        // without extra dependencies, but `dirs` crate is often used.
        // Since we want to avoid extra dependencies if possible, we'll
        // stick to env for now or use `libc`.

        // SAFETY: getuid() is a simple syscall that always succeeds.
        let uid = unsafe { libc::getuid() };
        // SAFETY: getpwuid() returns a pointer to a static struct or null.
        // It is thread-safe on most modern systems if we don't call other
        // functions that use the same static buffer, though getpwuid_r
        // would be safer.
        let passwd = unsafe { libc::getpwuid(uid) };
        if !passwd.is_null() {
            // SAFETY: pw_dir is a null-terminated string.
            let dir = unsafe { std::ffi::CStr::from_ptr((*passwd).pw_dir) };
            return Some(PathBuf::from(dir.to_string_lossy().into_owned()));
        }
        None
    }
}

/// Ported from original/adb_utils.cpp: adb_get_android_dir_path
pub fn adb_get_android_dir_path() -> Option<PathBuf> {
    let mut path = adb_get_homedir_path()?;
    path.push(".android");
    if !path.exists() {
        if let Err(e) = fs::create_dir(&path) {
            eprintln!("Cannot mkdir '{:?}': {}", path, e);
            return None;
        }
    }
    Some(path)
}

/// Ported from original/adb_utils.cpp: GetLogFilePath
pub fn get_log_file_path() -> PathBuf {
    if let Ok(path) = std::env::var("ANDROID_ADB_LOG_PATH") {
        return PathBuf::from(path);
    }

    #[cfg(windows)]
    {
        let mut path = std::env::temp_dir();
        path.push("adb.log");
        path
    }
    #[cfg(unix)]
    {
        let tmp_dir = std::env::var("TMPDIR").unwrap_or_else(|_| "/tmp".to_string());
        // SAFETY: getuid() is a simple syscall that always succeeds.
        let uid = unsafe { libc::getuid() };
        PathBuf::from(format!("{}/adb.{}.log", tmp_dir, uid))
    }
}

/// Ported from original/adb_utils.h: BlockingQueue
pub struct BlockingQueue<T> {
    inner: Arc<(Mutex<Vec<T>>, Condvar)>,
}

impl<T> BlockingQueue<T> {
    pub fn new() -> Self {
        Self {
            inner: Arc::new((Mutex::new(Vec::new()), Condvar::new())),
        }
    }

    pub fn push(&self, t: T) {
        let (lock, cvar) = &*self.inner;
        let mut queue = lock.lock().unwrap();
        queue.push(t);
        cvar.notify_one();
    }

    pub fn pop_all<F>(&self, mut f: F)
    where
        F: FnMut(T),
    {
        let (lock, cvar) = &*self.inner;
        let mut queue = lock.lock().expect("mutex poisoned");
        while queue.is_empty() {
            queue = cvar.wait(queue).expect("mutex poisoned");
        }
        let popped = std::mem::take(&mut *queue);
        drop(queue);

        for t in popped {
            f(t);
        }
    }
}

impl<T> Default for BlockingQueue<T> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_strip_trailing_nulls() {
        assert_eq!(strip_trailing_nulls("foo\0\0"), "foo");
        assert_eq!(strip_trailing_nulls("foo"), "foo");
        assert_eq!(strip_trailing_nulls("\0"), "");
    }

    #[test]
    fn test_parse_uint() {
        assert_eq!(parse_uint::<u32>("123foo"), Some((123, "foo")));
        assert_eq!(parse_uint::<u32>("123"), Some((123, "")));
        assert_eq!(parse_uint::<u32>("foo"), None);
        assert_eq!(parse_uint::<u32>(""), None);
    }

    #[test]
    fn test_directory_exists() {
        let dir = tempdir().unwrap();
        assert!(directory_exists(dir.path()));
        assert!(!directory_exists(dir.path().join("does-not-exist")));
    }

    #[test]
    fn test_escape_arg() {
        assert_eq!(escape_arg(""), "''");
        assert_eq!(escape_arg("abc"), "'abc'");
        assert_eq!(escape_arg("'"), "''\\'''");
        assert_eq!(escape_arg("abc abc"), "'abc abc'");
        assert_eq!(escape_arg("abc'abc"), "'abc'\\''abc'");
    }

    #[test]
    fn test_mkdirs() {
        let dir = tempdir().unwrap();
        let sub = dir.path().join("a/b/c");
        assert!(mkdirs(&sub).is_ok());
        assert!(sub.is_dir());
    }

    #[test]
    fn test_dump_hex() {
        let data = b"hello world";
        let dumped = dump_hex(data);
        assert!(dumped.contains("68656c6c6f20776f726c64"));
        assert!(dumped.contains("hello world"));

        let large_data = [0u8; 20];
        let dumped_large = dump_hex(&large_data);
        assert!(dumped_large.contains("[truncated]"));
    }

    #[test]
    fn test_forward_targets_are_valid() {
        assert!(forward_targets_are_valid("tcp:8000", "tcp:9000").is_ok());
        assert!(forward_targets_are_valid("tcp:0", "tcp:9000").is_ok());
        assert!(forward_targets_are_valid("tcp:-1", "tcp:9000").is_err());
        assert!(forward_targets_are_valid("tcp:8000", "tcp:0").is_err());
        assert!(forward_targets_are_valid("tcp:8000", "tcp:-1").is_err());
    }

    #[test]
    fn test_blocking_queue() {
        let queue = BlockingQueue::new();
        queue.push(1);
        queue.push(2);
        let mut results = Vec::new();
        queue.pop_all(|t| results.push(t));
        assert_eq!(results, vec![1, 2]);
    }
}
