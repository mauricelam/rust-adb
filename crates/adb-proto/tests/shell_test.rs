use adb_proto::{
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
    assert_eq!(ShellPacketId::try_from(255), Err(255));
    assert_eq!(ShellPacketId::try_from(42), Err(42));
}

#[test]
fn test_full_packet_ported() {
    let mut decoder = ShellDecoder::new();
    let id = ShellPacketId::Stdout;
    let data = b"hello world";

    let packet = ShellPacket::new(id, data.to_vec());
    let encoded = packet.encode().expect("encoding should succeed");

    assert_eq!(encoded.len(), ShellPacket::HEADER_SIZE + data.len());
    assert_eq!(encoded[0], id as u8);
    let payload_len = u32::from_le_bytes(encoded[1..5].try_into().unwrap());
    assert_eq!(payload_len, data.len() as u32);
    assert_eq!(&encoded[5..], data);

    decoder.feed(&encoded);
    let decoded = decoder.pop_packet().unwrap().expect("packet expected");
    assert_eq!(decoded.id, id);
    assert_eq!(decoded.data, data);
}

#[test]
fn test_zero_length_packet_ported() {
    let mut decoder = ShellDecoder::new();
    let id = ShellPacketId::Stderr;

    let packet = ShellPacket::new(id, Vec::new());
    let encoded = packet.encode().expect("encoding should succeed");

    assert_eq!(encoded.len(), ShellPacket::HEADER_SIZE);
    assert_eq!(encoded[0], id as u8);
    let payload_len = u32::from_le_bytes(encoded[1..5].try_into().unwrap());
    assert_eq!(payload_len, 0);

    decoder.feed(&encoded);
    let decoded = decoder.pop_packet().unwrap().expect("packet expected");
    assert_eq!(decoded.id, id);
    assert!(decoded.data.is_empty());
}

#[test]
fn test_exit_code_packet_ported() {
    let mut decoder = ShellDecoder::new();
    let exit_code = 42u8;

    let packet = ShellPacket::exit_code(exit_code);
    let encoded = packet.encode().expect("encoding should succeed");
    assert_eq!(packet.id, ShellPacketId::Exit);
    assert_eq!(packet.data, vec![exit_code]);
    assert_eq!(packet.parse_exit_code().unwrap(), exit_code);

    decoder.feed(&encoded);
    let decoded = decoder.pop_packet().unwrap().expect("packet expected");
    assert_eq!(decoded.id, ShellPacketId::Exit);
    assert_eq!(decoded.parse_exit_code().unwrap(), exit_code);
}

#[test]
fn test_incremental_decoding_one_byte_at_a_time() {
    let mut decoder = ShellDecoder::new();
    let packet = ShellPacket::new(ShellPacketId::Stdout, b"hello world".to_vec());
    let encoded = packet.encode().unwrap();

    for &b in &encoded[..encoded.len() - 1] {
        decoder.feed(&[b]);
        assert_eq!(
            decoder.pop_packet().unwrap(),
            None,
            "packet shouldn't be ready yet"
        );
    }

    decoder.feed(&[*encoded.last().unwrap()]);
    let decoded = decoder.pop_packet().unwrap().expect("packet ready");
    assert_eq!(decoded, packet);
}

#[test]
fn test_multiple_packets_in_one_chunk() {
    let mut decoder = ShellDecoder::new();
    let p1 = ShellPacket::new(ShellPacketId::Stdout, b"hello world".to_vec());
    let p2 = ShellPacket::new(ShellPacketId::Stderr, b"error message".to_vec());
    let p3 = ShellPacket::exit_code(0);

    let mut chunk = Vec::new();
    chunk.extend_from_slice(&p1.encode().unwrap());
    chunk.extend_from_slice(&p2.encode().unwrap());
    chunk.extend_from_slice(&p3.encode().unwrap());

    decoder.feed(&chunk);

    let d1 = decoder.pop_packet().unwrap().expect("p1 ready");
    assert_eq!(d1, p1);

    let d2 = decoder.pop_packet().unwrap().expect("p2 ready");
    assert_eq!(d2, p2);

    let d3 = decoder.pop_packet().unwrap().expect("p3 ready");
    assert_eq!(d3, p3);

    assert_eq!(decoder.pop_packet().unwrap(), None);
}

#[test]
fn test_split_at_every_possible_byte_boundary() {
    let p1 = ShellPacket::new(ShellPacketId::Stdout, b"stream 1 data".to_vec());
    let p2 = ShellPacket::new(ShellPacketId::Stderr, b"stream 2 err".to_vec());
    let mut chunk = p1.encode().unwrap();
    chunk.extend_from_slice(&p2.encode().unwrap());

    for split_idx in 1..chunk.len() {
        let mut decoder = ShellDecoder::new();
        decoder.feed(&chunk[..split_idx]);

        let mut received = Vec::new();
        while let Ok(Some(pkt)) = decoder.pop_packet() {
            received.push(pkt);
        }

        decoder.feed(&chunk[split_idx..]);
        while let Ok(Some(pkt)) = decoder.pop_packet() {
            received.push(pkt);
        }

        assert_eq!(received.len(), 2, "failed at split index {split_idx}");
        assert_eq!(received[0], p1);
        assert_eq!(received[1], p2);
    }
}

#[test]
fn test_byte_exact_vectors_all_ids() {
    let test_cases = [
        (ShellPacketId::Stdin, 0u8),
        (ShellPacketId::Stdout, 1u8),
        (ShellPacketId::Stderr, 2u8),
        (ShellPacketId::Exit, 3u8),
        (ShellPacketId::CloseStdin, 4u8),
        (ShellPacketId::WindowSizeChange, 5u8),
    ];

    for (id, expected_byte) in test_cases {
        let packet = ShellPacket::new(id, b"X".to_vec());
        let encoded = packet.encode().unwrap();

        assert_eq!(encoded[0], expected_byte);
        assert_eq!(&encoded[1..5], &1u32.to_le_bytes());
        assert_eq!(&encoded[5..], b"X");

        let mut decoder = ShellDecoder::new();
        decoder.feed(&encoded);
        let decoded = decoder.pop_packet().unwrap().unwrap();
        assert_eq!(decoded.id, id);
        assert_eq!(decoded.data, b"X");
    }
}

#[test]
fn test_invalid_and_unknown_packet_ids() {
    let mut invalid_pkt = vec![255u8];
    invalid_pkt.extend_from_slice(&4u32.to_le_bytes()); // length 4
    invalid_pkt.extend_from_slice(b"test");

    let mut decoder = ShellDecoder::new();
    decoder.feed(&invalid_pkt);
    assert_eq!(
        decoder.pop_packet(),
        Err(ShellDecodeError::UnknownPacketId(255))
    );

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
    assert_eq!(packet.data, b"24x80,0x0\0");
    assert_eq!(packet.parse_window_size().unwrap(), ws);

    let packet_no_nul = ShellPacket::new(ShellPacketId::WindowSizeChange, b"24x80,0x0".to_vec());
    assert_eq!(packet_no_nul.parse_window_size().unwrap(), ws);

    let ws_pixel = WindowSize {
        rows: 40,
        cols: 120,
        x_pixels: 800,
        y_pixels: 600,
    };
    let packet_pixel = ShellPacket::window_size_change(ws_pixel);
    assert_eq!(packet_pixel.parse_window_size().unwrap(), ws_pixel);
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
}

#[test]
fn test_oversized_declared_length_fails_before_large_allocation() {
    let mut bad_header = vec![ShellPacketId::Stdout as u8];
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
fn test_sticky_error_handling() {
    let mut bad_header = vec![ShellPacketId::Stdout as u8];
    let bad_len = ((MAX_PAYLOAD + 1) as u32).to_le_bytes();
    bad_header.extend_from_slice(&bad_len);

    let mut decoder = ShellDecoder::new();
    decoder.feed(&bad_header);

    let err = ShellDecodeError::PayloadTooLarge {
        length: MAX_PAYLOAD + 1,
        max: MAX_PAYLOAD,
    };

    assert_eq!(decoder.pop_packet(), Err(err.clone()));
    assert_eq!(decoder.pop_packet(), Err(err.clone()));

    decoder.feed(b"more data");
    assert_eq!(decoder.pop_packet(), Err(err));

    decoder.clear();
    assert_eq!(decoder.pop_packet(), Ok(None));
}
