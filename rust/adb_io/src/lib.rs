//! This crate provides functionality for the ADB protocol, and is intended to be
//! shared between the adb client and the adbd daemon.
//!
//! The functions in this crate are ported from the C++ implementation in
//! `original/adb_io.cpp`.

use std::io::{Error, ErrorKind, Read, Write};

/// Reads exactly `buf.len()` bytes from the reader.
///
/// This is a wrapper around `std::io::Read::read_exact`. This function is
/// provided to maintain a clear mapping to the original C++ codebase.
///
/// Corresponds to the C++ function `ReadFdExactly` in `original/adb_io.cpp`.
pub fn read_exactly<R: Read>(reader: &mut R, buf: &mut [u8]) -> std::io::Result<()> {
    reader.read_exact(buf)
}

/// Writes the entire contents of `buf` to the writer.
///
/// This is a wrapper around `std::io::Write::write_all`. This function is
/// provided to maintain a clear mapping to the original C++ codebase.
///
/// Corresponds to the C++ function `WriteFdExactly` in `original/adb_io.cpp`.
pub fn write_exactly<W: Write>(writer: &mut W, buf: &[u8]) -> std::io::Result<()> {
    writer.write_all(buf)
}

/// Same as `write_exactly`, but for string slices.
pub fn write_exactly_str<W: Write>(writer: &mut W, s: &str) -> std::io::Result<()> {
    write_exactly(writer, s.as_bytes())
}

/// Formats a string and writes it to the writer.
///
/// Corresponds to the C++ function `WriteFdFmt` in `original/adb_io.cpp`.
#[macro_export]
macro_rules! write_fd_fmt {
    ($writer:expr, $($arg:tt)*) => {
        $crate::write_exactly($writer, format!($($arg)*).as_bytes())
    };
}

/// Given a client socket, wait for orderly/graceful shutdown.
///
/// Returns `Ok(true)` if orderly/graceful shutdown has occurred with no additional data.
/// Returns `Ok(false)` if data was unexpectedly received.
/// Returns `Err` if a read error occurred.
///
/// Corresponds to the C++ function `ReadOrderlyShutdown` in `original/adb_io.cpp`.
pub fn read_orderly_shutdown<R: Read>(reader: &mut R) -> std::io::Result<bool> {
    let mut buf = [0; 16];
    match reader.read(&mut buf) {
        Ok(0) => Ok(true),
        Ok(_) => Ok(false),
        Err(e) => Err(e),
    }
}

/// Sends a protocol string, which is a 4-byte hex length followed by the string data.
/// The total length of the string cannot exceed 65535 bytes (0xFFFF).
///
/// Corresponds to the C++ function `SendProtocolString` in `original/adb_io.cpp`.
pub fn send_protocol_string<W: Write>(writer: &mut W, s: &str) -> std::io::Result<()> {
    if s.len() > 0xFFFF {
        return Err(Error::new(
            ErrorKind::InvalidInput,
            "string too long for protocol",
        ));
    }
    let msg = format!("{:04x}{}", s.len(), s);
    write_exactly(writer, msg.as_bytes())
}

/// Reads a protocol string; a four-hex-digit length followed by the string data.
///
/// Corresponds to the C++ function `ReadProtocolString` in `original/adb_io.cpp`.
pub fn read_protocol_string<R: Read>(reader: &mut R) -> std::io::Result<String> {
    let mut len_buf = [0; 4];
    read_exactly(reader, &mut len_buf)?;
    let len_str =
        std::str::from_utf8(&len_buf).map_err(|e| Error::new(ErrorKind::InvalidData, e))?;
    let len =
        usize::from_str_radix(len_str, 16).map_err(|e| Error::new(ErrorKind::InvalidData, e))?;

    let mut msg_buf = vec![0; len];
    read_exactly(reader, &mut msg_buf)?;
    String::from_utf8(msg_buf).map_err(|e| Error::new(ErrorKind::InvalidData, e))
}

/// Sends the protocol "OKAY" message.
///
/// Corresponds to the C++ function `SendOkay` in `original/adb_io.cpp`.
pub fn send_okay<W: Write>(writer: &mut W) -> std::io::Result<()> {
    write_exactly(writer, b"OKAY")
}

/// Sends the protocol "FAIL" message, with the given failure reason.
///
/// Corresponds to the C++ function `SendFail` in `original/adb_io.cpp`.
pub fn send_fail<W: Write>(writer: &mut W, reason: &str) -> std::io::Result<()> {
    write_exactly(writer, b"FAIL")?;
    send_protocol_string(writer, reason)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Cursor, Seek};
    use tempfile::NamedTempFile;

    #[test]
    fn test_send_protocol_string() {
        let mut writer = Vec::new();
        send_protocol_string(&mut writer, "hello").unwrap();
        assert_eq!(writer, b"0005hello");
    }

    #[test]
    fn test_send_protocol_string_too_long() {
        let mut writer = Vec::new();
        let s = "a".repeat(0x10000);
        let result = send_protocol_string(&mut writer, &s);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().kind(), ErrorKind::InvalidInput);
    }

    #[test]
    fn test_read_protocol_string() {
        let mut reader = Cursor::new(b"0005hello");
        let s = read_protocol_string(&mut reader).unwrap();
        assert_eq!(s, "hello");
    }

    #[test]
    fn test_read_protocol_string_invalid_len() {
        let mut reader = Cursor::new(b"xxxxhello");
        assert!(read_protocol_string(&mut reader).is_err());
    }

    #[test]
    fn test_read_protocol_string_short_read() {
        let mut reader = Cursor::new(b"0005he");
        assert!(read_protocol_string(&mut reader).is_err());
    }

    #[test]
    fn test_send_okay() {
        let mut writer = Vec::new();
        send_okay(&mut writer).unwrap();
        assert_eq!(writer, b"OKAY");
    }

    #[test]
    fn test_send_fail() {
        let mut writer = Vec::new();
        send_fail(&mut writer, "error").unwrap();
        assert_eq!(writer, b"FAIL0005error");
    }

    // These tests verify that the standard library functions behave as expected
    // and are a suitable replacement for the C++ `ReadFdExactly` function.
    #[test]
    fn test_read_exactly_stdlib_whole() {
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(b"Foobar").unwrap();
        file.seek(std::io::SeekFrom::Start(0)).unwrap();

        let mut buf = [0; 6];
        read_exactly(file.as_file_mut(), &mut buf).unwrap();
        assert_eq!(&buf, b"Foobar");
    }

    #[test]
    fn test_read_exactly_stdlib_eof() {
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(b"Foobar").unwrap();
        file.seek(std::io::SeekFrom::Start(0)).unwrap();

        let mut buf = [0; 7];
        assert!(read_exactly(file.as_file_mut(), &mut buf).is_err());
    }

    #[test]
    fn test_read_exactly_stdlib_partial() {
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(b"Foobar").unwrap();
        file.seek(std::io::SeekFrom::Start(0)).unwrap();

        let mut buf = [0; 5];
        read_exactly(file.as_file_mut(), &mut buf).unwrap();
        assert_eq!(&buf, b"Fooba");
    }

    // These tests verify that the standard library functions behave as expected
    // and are a suitable replacement for the C++ `WriteFdExactly` function.
    #[test]
    fn test_write_all_stdlib_whole() {
        let mut file = NamedTempFile::new().unwrap();
        write_exactly(file.as_file_mut(), b"Foobar").unwrap();

        file.seek(std::io::SeekFrom::Start(0)).unwrap();
        let mut buf = Vec::new();
        file.read_to_end(&mut buf).unwrap();
        assert_eq!(&buf, b"Foobar");
    }

    #[test]
    fn test_write_all_stdlib_partial() {
        let mut file = NamedTempFile::new().unwrap();
        write_exactly(file.as_file_mut(), &b"Foobar"[..5]).unwrap();

        file.seek(std::io::SeekFrom::Start(0)).unwrap();
        let mut buf = Vec::new();
        file.read_to_end(&mut buf).unwrap();
        assert_eq!(&buf, b"Fooba");
    }

    #[test]
    fn test_write_exactly_str() {
        let mut writer = Vec::new();
        write_exactly_str(&mut writer, "hello").unwrap();
        assert_eq!(writer, b"hello");
    }

    #[test]
    fn test_write_fd_fmt() {
        let mut writer = Vec::new();
        write_fd_fmt!(&mut writer, "foo {} {}", "bar", 123).unwrap();
        assert_eq!(writer, b"foo bar 123");
    }

    #[test]
    fn test_read_orderly_shutdown_success() {
        let mut reader = Cursor::new(b"");
        assert_eq!(read_orderly_shutdown(&mut reader).unwrap(), true);
    }

    #[test]
    fn test_read_orderly_shutdown_unexpected_data() {
        let mut reader = Cursor::new(b"data");
        assert_eq!(read_orderly_shutdown(&mut reader).unwrap(), false);
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn test_write_exactly_enospc() {
        use std::fs::OpenOptions;
        if let Ok(mut file) = OpenOptions::new().write(true).open("/dev/full") {
            let result = write_exactly(&mut file, b"foo");
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().raw_os_error(), Some(libc::ENOSPC));
        }
    }

    #[test]
    fn test_pipe_read_write() {
        use std::os::unix::net::UnixStream;
        let (mut s1, mut s2) = UnixStream::pair().unwrap();

        let data = b"some data to send through pipe";
        write_exactly(&mut s1, data).unwrap();

        let mut buf = vec![0u8; data.len()];
        read_exactly(&mut s2, &mut buf).unwrap();
        assert_eq!(buf, data);
    }
}
