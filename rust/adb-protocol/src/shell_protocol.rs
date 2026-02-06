/*
 * Copyright (C) 2023 The Android Open Source Project
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

use std::io::{Read, Write};

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellId {
    Stdin = 0,
    Stdout = 1,
    Stderr = 2,
    Exit = 3,
    CloseStdin = 4,
    WindowSizeChange = 5,
    Invalid = 255,
}

impl From<u8> for ShellId {
    fn from(id: u8) -> Self {
        match id {
            0 => ShellId::Stdin,
            1 => ShellId::Stdout,
            2 => ShellId::Stderr,
            3 => ShellId::Exit,
            4 => ShellId::CloseStdin,
            5 => ShellId::WindowSizeChange,
            _ => ShellId::Invalid,
        }
    }
}

pub struct ShellProtocol {
    pub id: ShellId,
    pub data: Vec<u8>,
}

impl ShellProtocol {
    pub fn new() -> Self {
        Self {
            id: ShellId::Invalid,
            data: Vec::new(),
        }
    }

    pub fn read<R: Read>(&mut self, reader: &mut R) -> std::io::Result<bool> {
        let mut header = [0u8; 5];
        let n = reader.read(&mut header[0..1])?;
        if n == 0 {
            return Ok(false);
        }
        reader.read_exact(&mut header[1..5])?;

        self.id = ShellId::from(header[0]);
        let length = u32::from_le_bytes(header[1..5].try_into().unwrap()) as usize;

        if length > crate::MAX_PAYLOAD {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Shell packet too large",
            ));
        }

        self.data.resize(length, 0);
        reader.read_exact(&mut self.data)?;

        Ok(true)
    }

    pub fn write<W: Write>(&self, writer: &mut W) -> std::io::Result<()> {
        let mut header = [0u8; 5];
        header[0] = self.id as u8;
        header[1..5].copy_from_slice(&(self.data.len() as u32).to_le_bytes());

        writer.write_all(&header)?;
        writer.write_all(&self.data)?;
        Ok(())
    }

    pub fn write_packet<W: Write>(writer: &mut W, id: ShellId, data: &[u8]) -> std::io::Result<()> {
        let mut header = [0u8; 5];
        header[0] = id as u8;
        header[1..5].copy_from_slice(&(data.len() as u32).to_le_bytes());

        writer.write_all(&header)?;
        writer.write_all(data)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn test_shell_protocol_read_write() {
        let mut data = Vec::new();
        let mut sp = ShellProtocol::new();
        sp.id = ShellId::Stdout;
        sp.data = b"hello".to_vec();
        sp.write(&mut data).unwrap();

        assert_eq!(data.len(), 5 + 5);
        assert_eq!(data[0], ShellId::Stdout as u8);
        assert_eq!(&data[1..5], &5u32.to_le_bytes());
        assert_eq!(&data[5..], b"hello");

        let mut reader = Cursor::new(data);
        let mut sp2 = ShellProtocol::new();
        assert!(sp2.read(&mut reader).unwrap());
        assert_eq!(sp2.id, ShellId::Stdout);
        assert_eq!(sp2.data, b"hello");
    }

    #[test]
    fn test_shell_protocol_eof() {
        let data = Vec::new();
        let mut reader = Cursor::new(data);
        let mut sp = ShellProtocol::new();
        assert!(!sp.read(&mut reader).unwrap());
    }
}
