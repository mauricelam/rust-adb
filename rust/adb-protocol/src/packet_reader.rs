/*
 * Copyright (C) 2024 The Android Open Source Project
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

use adb_types::{Amessage, Apacket, Block};
use crate::MAX_PAYLOAD;

/// Error returned when adding bytes to the packet reader fails.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddError {
    OversizedPayload,
}

/// A utility to read ADB packets from a stream of blocks.
///
/// Ported from `original/apacket_reader.h` and `original/apacket_reader.cpp`.
pub struct APacketReader {
    header: Block,
    packet: Option<Apacket>,
    packets: Vec<Apacket>,
}

impl Default for APacketReader {
    fn default() -> Self {
        Self::new()
    }
}

impl APacketReader {
    /// Creates a new `APacketReader`.
    pub fn new() -> Self {
        let mut reader = Self {
            header: Block::new(std::mem::size_of::<Amessage>()),
            packet: None,
            packets: Vec::new(),
        };
        reader.prepare_for_next_packet();
        reader
    }

    /// Adds bytes to the reader.
    ///
    /// This method can handle fragmented and merged packets.
    pub fn add_bytes(&mut self, mut block: Block) -> Result<(), AddError> {
        while block.remaining() > 0 || self.header.is_full() {
            if !self.header.is_full() {
                self.header.fill_from(&mut block);
                if !self.header.is_full() {
                    // We don't have a full header. Wait for more bytes.
                    return Ok(());
                }
            }

            // We have a full header. Peek to see how much payload is expected.
            // SAFETY: Amessage is repr(C) and we've verified the header block is full and has the correct size.
            // We use read_unaligned to be safe regarding potential alignment issues.
            let m = unsafe {
                std::ptr::read_unaligned(self.header.get_ref().as_ptr() as *const Amessage)
            };

            if m.data_length as usize > MAX_PAYLOAD {
                self.prepare_for_next_packet();
                return Err(AddError::OversizedPayload);
            }

            if m.data_length == 0 {
                let mut packet = Apacket::default();
                packet.msg = m;
                packet.payload = Block::new(0);
                self.add_packet(packet);
                continue;
            }

            if block.remaining() == 0 && self.packet.is_none() {
                return Ok(());
            }

            if self.packet.is_none() {
                let mut p = Apacket::default();
                p.msg = m;

                if block.position() == 0 && block.remaining() == p.msg.data_length as usize {
                    // Zero-copy: move the whole block as payload.
                    p.payload = block;
                    self.add_packet(p);
                    return Ok(());
                } else {
                    p.payload = Block::new(p.msg.data_length as usize);
                }
                self.packet = Some(p);
            }

            let p = self.packet.as_mut().expect("packet must be present");
            p.payload.fill_from(&mut block);

            if p.payload.is_full() {
                p.payload.rewind();
                let p = self.packet.take().expect("packet must be present");
                self.add_packet(p);
            } else {
                // We need more bytes for the payload.
                return Ok(());
            }
        }

        Ok(())
    }

    /// Returns all packets parsed so far, emptying the internal storage.
    pub fn get_packets(&mut self) -> Vec<Apacket> {
        std::mem::take(&mut self.packets)
    }

    /// Prepares the reader for the next packet.
    pub fn prepare_for_next_packet(&mut self) {
        self.header.rewind();
        self.packet = None;
    }

    fn add_packet(&mut self, packet: Apacket) {
        self.packets.push(packet);
        self.prepare_for_next_packet();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::A_SYNC;

    fn create_header_bytes(cmd: u32, data_len: u32) -> Vec<u8> {
        let msg = Amessage {
            command: cmd,
            data_length: data_len,
            ..Default::default()
        };
        unsafe {
            let ptr = &msg as *const Amessage as *const u8;
            std::slice::from_raw_parts(ptr, std::mem::size_of::<Amessage>()).to_vec()
        }
    }

    #[test]
    fn test_single_packet() {
        let mut reader = APacketReader::new();
        let mut data = create_header_bytes(A_SYNC, 4);
        data.extend_from_slice(b"test");

        let result = reader.add_bytes(Block::from_vec(data));
        assert_eq!(result, Ok(()));

        let packets = reader.get_packets();
        assert_eq!(packets.len(), 1);
        assert_eq!(packets[0].msg.command, A_SYNC);
        assert_eq!(packets[0].payload.get_ref(), b"test");
    }

    #[test]
    fn test_fragmented_header() {
        let mut reader = APacketReader::new();
        let data = create_header_bytes(A_SYNC, 0);

        let mid = data.len() / 2;
        reader.add_bytes(Block::from_vec(data[..mid].to_vec())).unwrap();
        assert_eq!(reader.get_packets().len(), 0);

        reader.add_bytes(Block::from_vec(data[mid..].to_vec())).unwrap();
        let packets = reader.get_packets();
        assert_eq!(packets.len(), 1);
        assert_eq!(packets[0].msg.command, A_SYNC);
    }

    #[test]
    fn test_fragmented_payload() {
        let mut reader = APacketReader::new();
        let mut data = create_header_bytes(A_SYNC, 4);
        data.extend_from_slice(b"test");

        let header_len = std::mem::size_of::<Amessage>();
        reader.add_bytes(Block::from_vec(data[..header_len + 2].to_vec())).unwrap();
        assert_eq!(reader.get_packets().len(), 0);

        reader.add_bytes(Block::from_vec(data[header_len + 2..].to_vec())).unwrap();
        let packets = reader.get_packets();
        assert_eq!(packets.len(), 1);
        assert_eq!(packets[0].payload.get_ref(), b"test");
    }

    #[test]
    fn test_merged_packets() {
        let mut reader = APacketReader::new();
        let mut data = create_header_bytes(A_SYNC, 4);
        data.extend_from_slice(b"test");
        let mut data2 = create_header_bytes(A_SYNC, 0);
        data.append(&mut data2);

        let result = reader.add_bytes(Block::from_vec(data));
        assert_eq!(result, Ok(()));

        let packets = reader.get_packets();
        assert_eq!(packets.len(), 2);
    }

    #[test]
    fn test_oversized_payload() {
        let mut reader = APacketReader::new();
        let data = create_header_bytes(A_SYNC, (MAX_PAYLOAD + 1) as u32);

        let result = reader.add_bytes(Block::from_vec(data));
        assert_eq!(result, Err(AddError::OversizedPayload));
    }

    #[test]
    fn test_no_payload_packet() {
        let mut reader = APacketReader::new();
        let data = create_header_bytes(A_SYNC, 0);

        let result = reader.add_bytes(Block::from_vec(data));
        assert_eq!(result, Ok(()));

        let packets = reader.get_packets();
        assert_eq!(packets.len(), 1);
        assert_eq!(packets[0].msg.command, A_SYNC);
        assert_eq!(packets[0].msg.data_length, 0);
        assert_eq!(packets[0].payload.len(), 0);
    }
}
