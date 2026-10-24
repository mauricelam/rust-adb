use adb_proto::shell::{
    ShellDecodeError, ShellDecoder, ShellEncodeError, ShellPacket, ShellPacketId, WindowSize,
    MAX_PAYLOAD,
};

#[test]
fn test_packet_id_conversion() {
    assert_eq!(ShellPacketId::try_from(0), Ok(ShellPacketId::Stdin));
    assert_eq!(ShellPacketId::try_from(1), Ok(ShellPacketId::Stdout));
    assert_eq!(ShellPacketId::try_from(2), Ok(ShellPacketId::Stderr));
    assert_eq!(ShellPacketId::try_from(3), Ok(ShellPacketId::Exit));
    assert_eq!(ShellPacketId::try_from(4), Ok(ShellPacketId::CloseStdin));
    assert_eq!(
        ShellPacketId::try_from(5),
        Ok(ShellPacketId::WindowSizeChange)
    );
    assert_eq!(ShellPacketId::try_from(255), Ok(ShellPacketId::Invalid));
    assert_eq!(ShellPacketId::try_from(42), Err(42));
}

#[test]
fn test_full_packet_ported() {
    // Ported from ShellProtocolTest.FullPacket
    let id = ShellPacketId::Stdout;
    let data = b"abc 123 \0\r\n";

    let packet = ShellPacket::new(id, data.to_vec());
    let encoded = packet.encode().unwrap();

    assert_eq!(encoded.len(), 5 + data.len());
    assert_eq!(encoded[0], 1); // kIdStdout
    assert_eq!(&encoded[1..5], &(data.len() as u32).to_le_bytes());
    assert_eq!(&encoded[5..], data);

    let mut decoder = ShellDecoder::new();
    decoder.feed(&encoded);
    let decoded = decoder.pop_packet().unwrap().expect("packet expected");

    assert_eq!(decoded.id, id);
    assert_eq!(decoded.data, data);
}

#[test]
fn test_zero_length_packet_ported() {
    // Ported from ShellProtocolTest.ZeroLengthPacket
    let id = ShellPacketId::Stderr;

    let packet = ShellPacket::new(id, Vec::new());
    let encoded = packet.encode().unwrap();

    assert_eq!(encoded, vec![2, 0, 0, 0, 0]);

    let mut decoder = ShellDecoder::new();
    decoder.feed(&encoded);
    let decoded = decoder.pop_packet().unwrap().expect("packet expected");

    assert_eq!(decoded.id, id);
    assert!(decoded.data.is_empty());
}

#[test]
fn test_exit_code_packet_ported() {
    // Ported from ShellProtocolTest.ExitCodePacket
    let exit_code = 20u8;
    let packet = ShellPacket::exit_code(exit_code);

    assert_eq!(packet.id, ShellPacketId::Exit);
    assert_eq!(packet.data, vec![20]);
    assert_eq!(packet.parse_exit_code(), Ok(20));

    let encoded = packet.encode().unwrap();
    let mut decoder = ShellDecoder::new();
    decoder.feed(&encoded);
    let decoded = decoder.pop_packet().unwrap().expect("packet expected");

    assert_eq!(decoded.id, ShellPacketId::Exit);
    assert_eq!(decoded.parse_exit_code(), Ok(20));
}

#[test]
fn test_incremental_decoding_one_byte_at_a_time() {
    let packet = ShellPacket::new(ShellPacketId::Stdout, b"hello world".to_vec());
    let encoded = packet.encode().unwrap();

    let mut decoder = ShellDecoder::new();
    for i in 0..encoded.len() - 1 {
        decoder.feed(&encoded[i..i + 1]);
        assert_eq!(
            decoder.pop_packet().unwrap(),
            None,
            "should not decode until full packet received"
        );
    }

    decoder.feed(&encoded[encoded.len() - 1..]);
    let decoded = decoder.pop_packet().unwrap().expect("packet ready");
    assert_eq!(decoded, packet);
}

#[test]
fn test_split_at_every_possible_byte_boundary() {
    let p1 = ShellPacket::new(ShellPacketId::Stdout, b"hello world".to_vec());
    let p2 = ShellPacket::new(ShellPacketId::Stderr, b"error message".to_vec());
    let p3 = ShellPacket::exit_code(0);

    let mut stream = Vec::new();
    stream.extend_from_slice(&p1.encode().unwrap());
    stream.extend_from_slice(&p2.encode().unwrap());
    stream.extend_from_slice(&p3.encode().unwrap());

    for split in 1..stream.len() {
        let mut decoder = ShellDecoder::new();
        let mut decoded_packets = Vec::new();

        decoder.feed(&stream[..split]);
        while let Ok(Some(pkt)) = decoder.pop_packet() {
            decoded_packets.push(pkt);
        }

        decoder.feed(&stream[split..]);
        while let Ok(Some(pkt)) = decoder.pop_packet() {
            decoded_packets.push(pkt);
        }

        assert_eq!(
            decoded_packets,
            vec![p1.clone(), p2.clone(), p3.clone()],
            "failed at split point {split}"
        );
    }
}

#[test]
fn test_multiple_packets_in_one_chunk() {
    let p1 = ShellPacket::new(ShellPacketId::Stdin, b"input data".to_vec());
    let p2 = ShellPacket::exit_code(0);
    let p3 = ShellPacket::new(ShellPacketId::CloseStdin, Vec::new());

    let mut chunk = Vec::new();
    chunk.extend_from_slice(&p1.encode().unwrap());
    chunk.extend_from_slice(&p2.encode().unwrap());
    chunk.extend_from_slice(&p3.encode().unwrap());

    let mut decoder = ShellDecoder::new();
    decoder.feed(&chunk);

    assert_eq!(decoder.pop_packet().unwrap(), Some(p1));
    assert_eq!(decoder.pop_packet().unwrap(), Some(p2));
    assert_eq!(decoder.pop_packet().unwrap(), Some(p3));
    assert_eq!(decoder.pop_packet().unwrap(), None);
}

#[test]
fn test_byte_exact_vectors_all_ids() {
    let test_cases = vec![
        (ShellPacketId::Stdin, 0u8),
        (ShellPacketId::Stdout, 1u8),
        (ShellPacketId::Stderr, 2u8),
        (ShellPacketId::Exit, 3u8),
        (ShellPacketId::CloseStdin, 4u8),
        (ShellPacketId::WindowSizeChange, 5u8),
        (ShellPacketId::Invalid, 255u8),
    ];

    for (id, expected_byte) in test_cases {
        let packet = ShellPacket::new(id, b"X".to_vec());
        let encoded = packet.encode().unwrap();

        assert_eq!(encoded[0], expected_byte);
        assert_eq!(&encoded[1..5], &[1, 0, 0, 0]);
        assert_eq!(encoded[5], b'X');

        let mut decoder = ShellDecoder::new();
        decoder.feed(&encoded);
        let decoded = decoder.pop_packet().unwrap().unwrap();
        assert_eq!(decoded.id, id);
        assert_eq!(decoded.data, b"X");
    }
}

#[test]
fn test_invalid_and_unknown_packet_ids() {
    // C++ `ShellProtocol::Read()` in `original/shell_service_protocol.cpp` reads 1 byte into buffer_[0]
    // without rejecting `kIdInvalid = 255` (defined in `original/shell_protocol.h`).
    // Therefore, packet ID 255 (`ShellPacketId::Invalid`) is accepted as a valid packet ID on wire,
    // whereas unknown packet IDs (such as 9) produce `ShellDecodeError::UnknownPacketId(9)`.

    // Packet with ID 255 (Invalid) is accepted
    let mut invalid_pkt = vec![255u8];
    invalid_pkt.extend_from_slice(&4u32.to_le_bytes()); // length 4
    invalid_pkt.extend_from_slice(b"test");

    let mut decoder = ShellDecoder::new();
    decoder.feed(&invalid_pkt);
    let pkt = decoder.pop_packet().unwrap().expect("packet expected");
    assert_eq!(pkt.id, ShellPacketId::Invalid);
    assert_eq!(pkt.data, b"test");

    // Unknown ID 9 produces UnknownPacketId error
    let mut unknown_pkt = vec![9u8];
    unknown_pkt.extend_from_slice(&0u32.to_le_bytes());

    let mut decoder = ShellDecoder::new();
    decoder.feed(&unknown_pkt);
    assert_eq!(
        decoder.pop_packet(),
        Err(ShellDecodeError::UnknownPacketId(9))
    );
}

#[test]
fn test_window_size_change_helpers() {
    let ws = WindowSize {
        rows: 24,
        cols: 80,
        x_pixels: 0,
        y_pixels: 0,
    };

    let packet = ShellPacket::window_size_change(ws);
    assert_eq!(packet.id, ShellPacketId::WindowSizeChange);
    // Standard format: "24x80,0x0\0"
    assert_eq!(packet.parse_window_size(), Ok(ws));

    // Also test parsing without NUL byte
    let packet_no_nul = ShellPacket::new(ShellPacketId::WindowSizeChange, b"24x80,0x0".to_vec());
    assert_eq!(packet_no_nul.parse_window_size(), Ok(ws));

    // Test with non-zero pixels
    let ws_pixel = WindowSize {
        rows: 40,
        cols: 120,
        x_pixels: 1024,
        y_pixels: 768,
    };
    let packet_pixel = ShellPacket::window_size_change(ws_pixel);
    assert_eq!(packet_pixel.parse_window_size(), Ok(ws_pixel));
}

#[test]
fn test_payload_exceeding_max_payload() {
    let oversized = vec![0u8; MAX_PAYLOAD + 1];
    let packet = ShellPacket::new(ShellPacketId::Stdout, oversized);

    assert_eq!(
        packet.encode(),
        Err(ShellEncodeError::PayloadTooLarge {
            length: MAX_PAYLOAD + 1,
            max: MAX_PAYLOAD
        })
    );

    // Test decoding header claiming payload > MAX_PAYLOAD
    let mut bad_header = vec![1u8]; // kIdStdout
    let bad_len = ((MAX_PAYLOAD + 1) as u32).to_le_bytes();
    bad_header.extend_from_slice(&bad_len);

    let mut decoder = ShellDecoder::new();
    decoder.feed(&bad_header);
    assert_eq!(
        decoder.pop_packet(),
        Err(ShellDecodeError::PayloadTooLarge {
            length: MAX_PAYLOAD + 1,
            max: MAX_PAYLOAD
        })
    );
}

#[test]
fn test_oversized_declared_length_fails_before_large_allocation() {
    // 2 GiB declared payload length with only 5 header bytes fed
    let mut bad_header = vec![1u8]; // kIdStdout
    let bad_len = 0x8000_0000u32.to_le_bytes();
    bad_header.extend_from_slice(&bad_len);

    let mut decoder = ShellDecoder::new();
    decoder.feed(&bad_header);

    // Should immediately error without attempting to allocate 2 GiB
    assert_eq!(
        decoder.pop_packet(),
        Err(ShellDecodeError::PayloadTooLarge {
            length: 0x8000_0000,
            max: MAX_PAYLOAD
        })
    );
}

#[test]
fn test_sticky_error_handling() {
    let mut bad_header = vec![9u8]; // Unknown ID 9
    bad_header.extend_from_slice(&0u32.to_le_bytes());

    let mut decoder = ShellDecoder::new();
    decoder.feed(&bad_header);

    let err = ShellDecodeError::UnknownPacketId(9);
    assert_eq!(decoder.pop_packet(), Err(err.clone()));

    // Subsequent calls to pop_packet should report the same sticky error
    assert_eq!(decoder.pop_packet(), Err(err.clone()));

    // Feeding more data still yields sticky error
    decoder.feed(&[1, 0, 0, 0, 0]);
    assert_eq!(decoder.pop_packet(), Err(err));

    // After clear(), the error is cleared
    decoder.clear();
    assert_eq!(decoder.pop_packet(), Ok(None));
}
