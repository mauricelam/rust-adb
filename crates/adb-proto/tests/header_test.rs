use adb_proto::{Command, DecodeError, EncodeError, Header};

#[test]
fn test_command_numeric_values() {
    assert_eq!(Command::Sync as u32, 0x434e5953);
    assert_eq!(Command::Cnxn as u32, 0x4e584e43);
    assert_eq!(Command::Open as u32, 0x4e45504f);
    assert_eq!(Command::Okay as u32, 0x59414b4f);
    assert_eq!(Command::Clse as u32, 0x45534c43);
    assert_eq!(Command::Wrte as u32, 0x45545257);
    assert_eq!(Command::Auth as u32, 0x48545541);
    assert_eq!(Command::Stls as u32, 0x534c5453);
}

#[test]
fn test_command_byte_layout() {
    let commands = [
        (Command::Sync, *b"SYNC"),
        (Command::Cnxn, *b"CNXN"),
        (Command::Open, *b"OPEN"),
        (Command::Okay, *b"OKAY"),
        (Command::Clse, *b"CLSE"),
        (Command::Wrte, *b"WRTE"),
        (Command::Auth, *b"AUTH"),
        (Command::Stls, *b"STLS"),
    ];

    for (cmd, expected_ascii) in commands {
        let header = Header::new(cmd, 0x01020304, 0x05060708, 0x090a0b0c, 0x0d0e0f10);
        let encoded = header.encode();

        assert_eq!(&encoded[0..4], &expected_ascii);
        assert_eq!(&encoded[4..8], &0x01020304u32.to_le_bytes());
        assert_eq!(&encoded[8..12], &0x05060708u32.to_le_bytes());
        assert_eq!(&encoded[12..16], &0x090a0b0cu32.to_le_bytes());
        assert_eq!(&encoded[16..20], &0x0d0e0f10u32.to_le_bytes());

        let expected_magic = (cmd as u32) ^ 0xffff_ffff;
        assert_eq!(&encoded[20..24], &expected_magic.to_le_bytes());
    }
}

#[test]
fn test_round_trip() {
    let commands = [
        Command::Sync,
        Command::Cnxn,
        Command::Open,
        Command::Okay,
        Command::Clse,
        Command::Wrte,
        Command::Auth,
        Command::Stls,
    ];

    for cmd in commands {
        let original = Header::new(cmd, 0x01000001, 1024 * 1024, 42, 0x12345678);
        let encoded = original.encode();
        let decoded = Header::decode(&encoded).expect("failed to decode encoded header");

        assert_eq!(original, decoded);
    }
}

#[test]
fn test_short_input_handling() {
    let mut buf = [0u8; 24];
    let header = Header::new(Command::Cnxn, 1, 2, 3, 4);
    buf.copy_from_slice(&header.encode());

    for len in 0..24 {
        let result = Header::decode(&buf[..len]);
        assert_eq!(
            result,
            Err(DecodeError::BufferTooShort {
                expected: 24,
                actual: len,
            })
        );
    }
}

#[test]
fn test_long_input_handling() {
    let header = Header::new(Command::Open, 100, 200, 0, 0);
    let mut long_buf = Vec::new();
    long_buf.extend_from_slice(&header.encode());
    long_buf.extend_from_slice(b"extra trailing data payload");

    let decoded = Header::decode(&long_buf).expect("should decode successfully from longer buffer");
    assert_eq!(decoded, header);
}

#[test]
fn test_invalid_magic() {
    let header = Header::new(Command::Cnxn, 0, 0, 0, 0);
    let mut encoded = header.encode();
    // Corrupt magic field
    encoded[20] ^= 0xff;

    let expected_magic = (Command::Cnxn as u32) ^ 0xffff_ffff;
    let actual_magic = u32::from_le_bytes(encoded[20..24].try_into().unwrap());

    let result = Header::decode(&encoded);
    assert_eq!(
        result,
        Err(DecodeError::InvalidMagic {
            expected: expected_magic,
            actual: actual_magic,
        })
    );
}

#[test]
fn test_unknown_command() {
    let mut encoded = [0u8; 24];
    let bad_cmd: u32 = 0xdeadbeef;
    encoded[0..4].copy_from_slice(&bad_cmd.to_le_bytes());
    let magic = bad_cmd ^ 0xffff_ffff;
    encoded[20..24].copy_from_slice(&magic.to_le_bytes());

    let result = Header::decode(&encoded);
    assert_eq!(result, Err(DecodeError::UnknownCommand(0xdeadbeef)));
}

#[test]
fn test_encode_to_slice() {
    let header = Header::new(Command::Okay, 1, 2, 0, 0);
    let mut short_buf = [0u8; 10];
    assert_eq!(
        header.encode_to_slice(&mut short_buf),
        Err(EncodeError::BufferTooShort {
            expected: 24,
            actual: 10,
        })
    );

    let mut full_buf = [0u8; 24];
    assert!(header.encode_to_slice(&mut full_buf).is_ok());
    assert_eq!(full_buf, header.encode());
}
