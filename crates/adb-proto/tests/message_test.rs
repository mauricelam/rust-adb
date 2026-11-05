use adb_proto::{
    AuthMessage, AuthType, ChecksumMode, LocalId, Message, MessageDecodeError, PublicKeyLine,
    RemoteId, TOKEN_SIZE,
};

#[test]
fn test_cnxn_round_trip() {
    let msg = Message::Cnxn {
        version: 0x01000000,
        max_payload: 1048576,
        banner: "device::ro.product.name=test;".to_string(),
    };
    let (header, payload) = msg.encode(ChecksumMode::Compute);
    let decoded = Message::decode(&header, &payload).expect("decode cnxn");
    assert_eq!(msg, decoded);
}

#[test]
fn test_auth_token_round_trip() {
    let token = [42u8; TOKEN_SIZE];
    let msg = Message::Auth(AuthMessage::Token(token));
    let (header, payload) = msg.encode(ChecksumMode::Compute);
    let decoded = Message::decode(&header, &payload).expect("decode auth token");
    assert_eq!(msg, decoded);
}

#[test]
fn test_auth_signature_round_trip() {
    let sig = vec![1, 2, 3, 4, 5, 6, 7, 8];
    let msg = Message::Auth(AuthMessage::Signature(sig));
    let (header, payload) = msg.encode(ChecksumMode::Compute);
    let decoded = Message::decode(&header, &payload).expect("decode auth signature");
    assert_eq!(msg, decoded);
}

#[test]
fn test_auth_public_key_round_trip() {
    let key_line = PublicKeyLine::new("test_key_line user@host");
    let msg = Message::Auth(AuthMessage::PublicKey(key_line));
    let (header, payload) = msg.encode(ChecksumMode::Compute);
    let decoded = Message::decode(&header, &payload).expect("decode auth pubkey");
    assert_eq!(msg, decoded);
}

#[test]
fn test_open_round_trip() {
    let msg = Message::Open {
        remote_id: RemoteId::from_u32(10).unwrap(),
        send_bytes: 4096,
        service: "shell:ls".to_string(),
    };
    let (header, payload) = msg.encode(ChecksumMode::Compute);
    let decoded = Message::decode(&header, &payload).expect("decode open");
    assert_eq!(msg, decoded);
}

#[test]
fn test_okay_round_trip() {
    let msg = Message::Okay {
        local_id: LocalId::from_u32(100).unwrap(),
        remote_id: RemoteId::from_u32(200).unwrap(),
        ack_bytes: Some(1024),
    };
    let (header, payload) = msg.encode(ChecksumMode::Compute);
    let decoded = Message::decode(&header, &payload).expect("decode okay");
    if let Message::Okay {
        local_id,
        remote_id,
        ack_bytes,
    } = decoded
    {
        assert_eq!(local_id.get(), 200);
        assert_eq!(remote_id.get(), 100);
        assert_eq!(ack_bytes, Some(1024));
    } else {
        panic!("expected Okay variant");
    }
}

#[test]
fn test_wrte_direction_mapping() {
    let msg = Message::Wrte {
        local_id: LocalId::from_u32(1).unwrap(),
        remote_id: RemoteId::from_u32(2).unwrap(),
        data: b"hello".to_vec(),
    };
    let (header, payload) = msg.encode(ChecksumMode::Compute);
    let decoded = Message::decode(&header, &payload).expect("decode wrte");
    match decoded {
        Message::Wrte {
            local_id,
            remote_id,
            data,
        } => {
            assert_eq!(local_id.get(), 2);
            assert_eq!(remote_id.get(), 1);
            assert_eq!(data, b"hello");
        }
        _ => panic!("expected Wrte"),
    }
}

#[test]
fn test_clse_open_rejected_arg0_zero() {
    let header = adb_proto::Header::new(adb_proto::Command::Clse, 0, 42, 0, 0);
    let decoded = Message::decode(&header, &[]).expect("decode clse with arg0=0");
    match decoded {
        Message::Clse {
            local_id,
            remote_id,
        } => {
            assert_eq!(local_id.map(|id| id.get()), Some(42));
            assert_eq!(remote_id, None);
        }
        _ => panic!("expected Clse"),
    }
}

#[test]
fn test_invalid_token_length() {
    let header = adb_proto::Header::new(adb_proto::Command::Auth, AuthType::Token as u32, 0, 10, 0);
    let payload = vec![0u8; 10];
    let res = Message::decode(&header, &payload);
    assert!(matches!(
        res,
        Err(MessageDecodeError::InvalidTokenLength { .. })
    ));
}
