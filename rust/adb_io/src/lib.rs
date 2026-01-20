use std::io::{Read, Write, Error, ErrorKind};

pub fn send_protocol_string<W: Write>(writer: &mut W, s: &str) -> std::io::Result<()> {
    let msg = format!("{:04x}{}", s.len(), s);
    writer.write_all(msg.as_bytes())
}

pub fn read_protocol_string<R: Read>(reader: &mut R) -> std::io::Result<String> {
    let mut len_buf = [0; 4];
    reader.read_exact(&mut len_buf)?;
    let len_str = std::str::from_utf8(&len_buf)
        .map_err(|e| Error::new(ErrorKind::InvalidData, e))?;
    let len = usize::from_str_radix(len_str, 16)
        .map_err(|e| Error::new(ErrorKind::InvalidData, e))?;

    let mut msg_buf = vec![0; len];
    reader.read_exact(&mut msg_buf)?;
    String::from_utf8(msg_buf)
        .map_err(|e| Error::new(ErrorKind::InvalidData, e))
}

pub fn send_okay<W: Write>(writer: &mut W) -> std::io::Result<()> {
    writer.write_all(b"OKAY")
}

pub fn send_fail<W: Write>(writer: &mut W, reason: &str) -> std::io::Result<()> {
    writer.write_all(b"FAIL")?;
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

    #[test]
    fn test_read_fd_exactly_whole() {
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(b"Foobar").unwrap();
        file.seek(std::io::SeekFrom::Start(0)).unwrap();

        let mut buf = [0; 6];
        file.read_exact(&mut buf).unwrap();
        assert_eq!(&buf, b"Foobar");
    }

    #[test]
    fn test_read_fd_exactly_eof() {
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(b"Foobar").unwrap();
        file.seek(std::io::SeekFrom::Start(0)).unwrap();

        let mut buf = [0; 7];
        assert!(file.read_exact(&mut buf).is_err());
    }

    #[test]
    fn test_read_fd_exactly_partial() {
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(b"Foobar").unwrap();
        file.seek(std::io::SeekFrom::Start(0)).unwrap();

        let mut buf = [0; 5];
        file.read_exact(&mut buf).unwrap();
        assert_eq!(&buf, b"Fooba");
    }

    #[test]
    fn test_write_fd_exactly_whole() {
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(b"Foobar").unwrap();

        file.seek(std::io::SeekFrom::Start(0)).unwrap();
        let mut buf = Vec::new();
        file.read_to_end(&mut buf).unwrap();
        assert_eq!(&buf, b"Foobar");
    }

    #[test]
    fn test_write_fd_exactly_partial() {
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(&b"Foobar"[..5]).unwrap();

        file.seek(std::io::SeekFrom::Start(0)).unwrap();
        let mut buf = Vec::new();
        file.read_to_end(&mut buf).unwrap();
        assert_eq!(&buf, b"Fooba");
    }
}
