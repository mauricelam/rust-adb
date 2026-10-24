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
    let encoded = packet.encode_to_vec();

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
    let encoded = packet.encode_to_vec();

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

    let encoded = packet.encode_to_vec();
    let mut decoder = ShellDecoder::new();
    decoder.feed(&encoded);
    let decoded = decoder.pop_packet().unwrap().expect("packet expected");

    assert_eq!(decoded.id, ShellPacketId::Exit);
    assert_eq!(decoded.parse_exit_code(), Ok(20));
}

#[test]
fn test_incremental_decoding_one_byte_at_a_time() {
    let packet = ShellPacket::new(ShellPacketId::Stdout, b"hello world".to_vec());
    let encoded = packet.encode_to_vec();

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
fn test_multiple_packets_in_one_chunk() {
    let p1 = ShellPacket::new(ShellPacketId::Stdin, b"input data".to_vec());
    let p2 = ShellPacket::exit_code(0);
    let p3 = ShellPacket::new(ShellPacketId::CloseStdin, Vec::new());

    let mut chunk = Vec::new();
    chunk.extend_from_slice(&p1.encode_to_vec());
    chunk.extend_from_slice(&p2.encode_to_vec());
    chunk.extend_from_slice(&p3.encode_to_vec());

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
        let encoded = packet.encode_to_vec();

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
        packet.encode_to_vec_checked(),
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
fn test_unknown_packet_id() {
    let mut bad_header = vec![42u8]; // unknown id 42
    bad_header.extend_from_slice(&0u32.to_le_bytes());

    let mut decoder = ShellDecoder::new();
    decoder.feed(&bad_header);
    assert_eq!(
        decoder.pop_packet(),
        Err(ShellDecodeError::UnknownPacketId(42))
    );
}
