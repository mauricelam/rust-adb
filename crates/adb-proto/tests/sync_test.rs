use adb_proto::sync::{
    SyncDecodeError, SyncDecoder, SyncDecoderMode, SyncEncodeError, SyncMessage, SYNC_DATA_MAX,
    SYNC_PATH_MAX,
};

#[test]
fn test_byte_exact_vectors_v1_messages() {
    // 1. LIST request
    let list_msg = SyncMessage::List {
        path: "/sdcard".to_string(),
    };
    let list_expected = vec![
        b'L', b'I', b'S', b'T', // ID_LIST_V1
        0x07, 0x00, 0x00, 0x00, // path_length = 7
        b'/', b's', b'd', b'c', b'a', b'r', b'd',
    ];
    assert_eq!(list_msg.encode_to_vec(), list_expected);

    let mut decoder = SyncDecoder::new(SyncDecoderMode::Request);
    decoder.feed(&list_expected);
    assert_eq!(decoder.pop_message().unwrap(), Some(list_msg));

    // 2. DENT reply
    let dent_msg = SyncMessage::Dent {
        mode: 0o100644,
        size: 1234,
        mtime: 1600000000,
        name: "test.txt".to_string(),
    };
    let dent_expected = vec![
        b'D', b'E', b'N', b'T', // ID_DENT_V1
        0xa4, 0x81, 0x00, 0x00, // mode = 33188 (0o100644)
        0xd2, 0x04, 0x00, 0x00, // size = 1234
        0x00, 0x10, 0x5e, 0x5f, // mtime = 1600000000
        0x08, 0x00, 0x00, 0x00, // namelen = 8
        b't', b'e', b's', b't', b'.', b't', b'x', b't',
    ];
    assert_eq!(dent_msg.encode_to_vec(), dent_expected);

    let mut decoder = SyncDecoder::new(SyncDecoderMode::Response);
    decoder.feed(&dent_expected);
    assert_eq!(decoder.pop_message().unwrap(), Some(dent_msg));

    // 3. STAT request
    let stat_req = SyncMessage::Stat {
        path: "/etc/hosts".to_string(),
    };
    let stat_req_expected = vec![
        b'S', b'T', b'A', b'T', // ID_LSTAT_V1
        0x0a, 0x00, 0x00, 0x00, // path_length = 10
        b'/', b'e', b't', b'c', b'/', b'h', b'o', b's', b't', b's',
    ];
    assert_eq!(stat_req.encode_to_vec(), stat_req_expected);

    let mut decoder = SyncDecoder::new(SyncDecoderMode::Request);
    decoder.feed(&stat_req_expected);
    assert_eq!(decoder.pop_message().unwrap(), Some(stat_req));

    // 4. STAT reply (sync_stat_v1)
    let stat_reply = SyncMessage::StatReply {
        mode: 0o100644,
        size: 2048,
        mtime: 1600000000,
    };
    let stat_reply_expected = vec![
        b'S', b'T', b'A', b'T', // ID_LSTAT_V1
        0xa4, 0x81, 0x00, 0x00, // mode = 33188
        0x00, 0x08, 0x00, 0x00, // size = 2048
        0x00, 0x10, 0x5e, 0x5f, // mtime = 1600000000
    ];
    assert_eq!(stat_reply.encode_to_vec(), stat_reply_expected);

    let mut decoder = SyncDecoder::new(SyncDecoderMode::Response);
    decoder.feed(&stat_reply_expected);
    assert_eq!(decoder.pop_message().unwrap(), Some(stat_reply));

    // 5. SEND request
    let send_msg = SyncMessage::Send {
        path: "/tmp/app.apk".to_string(),
        mode: 33188,
    };
    let send_expected = vec![
        b'S', b'E', b'N', b'D', // ID_SEND_V1
        0x12, 0x00, 0x00, 0x00, // path_length = 18 ("/tmp/app.apk,33188".len())
        b'/', b't', b'm', b'p', b'/', b'a', b'p', b'p', b'.', b'a', b'p', b'k', b',', b'3', b'3',
        b'1', b'8', b'8',
    ];
    assert_eq!(send_msg.encode_to_vec(), send_expected);

    let mut decoder = SyncDecoder::new(SyncDecoderMode::Request);
    decoder.feed(&send_expected);
    assert_eq!(decoder.pop_message().unwrap(), Some(send_msg));

    // 6. RECV request
    let recv_msg = SyncMessage::Recv {
        path: "/sdcard/a".to_string(),
    };
    let recv_expected = vec![
        b'R', b'E', b'C', b'V', // ID_RECV_V1
        0x09, 0x00, 0x00, 0x00, // path_length = 9
        b'/', b's', b'd', b'c', b'a', b'r', b'd', b'/', b'a',
    ];
    assert_eq!(recv_msg.encode_to_vec(), recv_expected);

    let mut decoder = SyncDecoder::new(SyncDecoderMode::Request);
    decoder.feed(&recv_expected);
    assert_eq!(decoder.pop_message().unwrap(), Some(recv_msg));

    // 7. DATA chunk
    let data_msg = SyncMessage::Data {
        data: vec![0x01, 0x02, 0x03, 0x04],
    };
    let data_expected = vec![
        b'D', b'A', b'T', b'A', // ID_DATA
        0x04, 0x00, 0x00, 0x00, // size = 4
        0x01, 0x02, 0x03, 0x04,
    ];
    assert_eq!(data_msg.encode_to_vec(), data_expected);

    let mut decoder = SyncDecoder::new(SyncDecoderMode::Response);
    decoder.feed(&data_expected);
    assert_eq!(decoder.pop_message().unwrap(), Some(data_msg));

    // 8. DONE message
    let done_msg = SyncMessage::Done { mtime: 1600000000 };
    let done_expected = vec![
        b'D', b'O', b'N', b'E', // ID_DONE
        0x00, 0x10, 0x5e, 0x5f, // mtime = 1600000000
    ];
    assert_eq!(done_msg.encode_to_vec(), done_expected);

    let mut decoder = SyncDecoder::new(SyncDecoderMode::Response);
    decoder.feed(&done_expected);
    assert_eq!(decoder.pop_message().unwrap(), Some(done_msg));

    // 9. OKAY message
    let okay_msg = SyncMessage::Okay;
    let okay_expected = vec![
        b'O', b'K', b'A', b'Y', // ID_OKAY
        0x00, 0x00, 0x00, 0x00, // length = 0
    ];
    assert_eq!(okay_msg.encode_to_vec(), okay_expected);

    let mut decoder = SyncDecoder::new(SyncDecoderMode::Response);
    decoder.feed(&okay_expected);
    assert_eq!(decoder.pop_message().unwrap(), Some(okay_msg));

    // 10. FAIL message (encoded by server, decoded as error by SyncDecoder)
    let fail_msg = SyncMessage::Fail {
        message: "error".to_string(),
    };
    let fail_expected = vec![
        b'F', b'A', b'I', b'L', // ID_FAIL
        0x05, 0x00, 0x00, 0x00, // msglen = 5
        b'e', b'r', b'r', b'o', b'r',
    ];
    assert_eq!(fail_msg.encode_to_vec(), fail_expected);

    let mut decoder = SyncDecoder::new(SyncDecoderMode::Response);
    decoder.feed(&fail_expected);
    assert_eq!(
        decoder.pop_message(),
        Err(SyncDecodeError::FailMessage("error".to_string()))
    );

    // 11. QUIT message
    let quit_msg = SyncMessage::Quit;
    let quit_expected = vec![
        b'Q', b'U', b'I', b'T', // ID_QUIT
        0x00, 0x00, 0x00, 0x00, // length = 0
    ];
    assert_eq!(quit_msg.encode_to_vec(), quit_expected);

    let mut decoder = SyncDecoder::new(SyncDecoderMode::Request);
    decoder.feed(&quit_expected);
    assert_eq!(decoder.pop_message().unwrap(), Some(quit_msg));
}

#[test]
fn test_dent_multibyte_utf8_roundtrip() {
    let dent_msg = SyncMessage::Dent {
        mode: 0o100644,
        size: 999,
        mtime: 12345678,
        name: "测试-图片-🦀.png".to_string(),
    };

    let encoded = dent_msg.encode_to_vec();
    let mut decoder = SyncDecoder::new(SyncDecoderMode::Response);
    decoder.feed(&encoded);

    let decoded = decoder
        .pop_message()
        .unwrap()
        .expect("should decode dent message");
    assert_eq!(decoded, dent_msg);
}

#[test]
fn test_incremental_decoding_one_byte_at_a_time() {
    let msg = SyncMessage::Dent {
        mode: 0o100755,
        size: 4096,
        mtime: 1700000000,
        name: "executable.sh".to_string(),
    };
    let encoded = msg.encode_to_vec();

    let mut decoder = SyncDecoder::new(SyncDecoderMode::Response);
    for i in 0..encoded.len() - 1 {
        decoder.feed(&encoded[i..i + 1]);
        assert_eq!(
            decoder.pop_message().unwrap(),
            None,
            "incomplete message should return None"
        );
    }

    decoder.feed(&encoded[encoded.len() - 1..]);
    let decoded = decoder
        .pop_message()
        .unwrap()
        .expect("complete message ready");
    assert_eq!(decoded, msg);
}

#[test]
fn test_multiple_messages_in_chunk() {
    let msg1 = SyncMessage::Data {
        data: b"hello ".to_vec(),
    };
    let msg2 = SyncMessage::Data {
        data: b"world".to_vec(),
    };
    let msg3 = SyncMessage::Done { mtime: 12345 };
    let msg4 = SyncMessage::Okay;

    let mut chunk = Vec::new();
    chunk.extend_from_slice(&msg1.encode_to_vec());
    chunk.extend_from_slice(&msg2.encode_to_vec());
    chunk.extend_from_slice(&msg3.encode_to_vec());
    chunk.extend_from_slice(&msg4.encode_to_vec());

    let mut decoder = SyncDecoder::new(SyncDecoderMode::Response);
    decoder.feed(&chunk);

    assert_eq!(decoder.pop_message().unwrap(), Some(msg1));
    assert_eq!(decoder.pop_message().unwrap(), Some(msg2));
    assert_eq!(decoder.pop_message().unwrap(), Some(msg3));
    assert_eq!(decoder.pop_message().unwrap(), Some(msg4));
    assert_eq!(decoder.pop_message().unwrap(), None);
}

#[test]
fn test_send_mode_parsing() {
    // Octal mode format: "/sdcard/script,0755" (19 bytes)
    let spec_octal = b"SEND\x13\x00\x00\x00/sdcard/script,0755";
    let mut decoder = SyncDecoder::new(SyncDecoderMode::Request);
    decoder.feed(spec_octal);
    assert_eq!(
        decoder.pop_message().unwrap(),
        Some(SyncMessage::Send {
            path: "/sdcard/script".to_string(),
            mode: 0o755,
        })
    );

    // Decimal mode format: "/sdcard/file,33188"
    let spec_dec = b"SEND\x12\x00\x00\x00/sdcard/file,33188";
    let mut decoder = SyncDecoder::new(SyncDecoderMode::Request);
    decoder.feed(spec_dec);
    assert_eq!(
        decoder.pop_message().unwrap(),
        Some(SyncMessage::Send {
            path: "/sdcard/file".to_string(),
            mode: 33188,
        })
    );
}

#[test]
fn test_error_handling() {
    // 1. Unknown ID
    let bad_id_bytes = b"BADI\x00\x00\x00\x00";
    let mut decoder = SyncDecoder::new(SyncDecoderMode::Request);
    decoder.feed(bad_id_bytes);
    let err = decoder.pop_message().unwrap_err();
    assert_eq!(
        err,
        SyncDecodeError::UnknownId(u32::from_le_bytes(*b"BADI"))
    );

    // 2. Oversized DATA payload (> SYNC_DATA_MAX = 65536)
    let bad_data_len = (SYNC_DATA_MAX + 1) as u32;
    let mut bad_data_hdr = Vec::new();
    bad_data_hdr.extend_from_slice(b"DATA");
    bad_data_hdr.extend_from_slice(&bad_data_len.to_le_bytes());

    let mut decoder = SyncDecoder::new(SyncDecoderMode::Response);
    decoder.feed(&bad_data_hdr);
    assert_eq!(
        decoder.pop_message(),
        Err(SyncDecodeError::OversizedLength {
            length: SYNC_DATA_MAX + 1,
            max: SYNC_DATA_MAX,
        })
    );

    // 3. Oversized path length (> SYNC_PATH_MAX = 1024)
    let bad_path_len = (SYNC_PATH_MAX + 1) as u32;
    let mut bad_path_hdr = Vec::new();
    bad_path_hdr.extend_from_slice(b"LIST");
    bad_path_hdr.extend_from_slice(&bad_path_len.to_le_bytes());

    let mut decoder = SyncDecoder::new(SyncDecoderMode::Request);
    decoder.feed(&bad_path_hdr);
    assert_eq!(
        decoder.pop_message(),
        Err(SyncDecodeError::OversizedLength {
            length: SYNC_PATH_MAX + 1,
            max: SYNC_PATH_MAX,
        })
    );

    // 4. Malformed SEND spec (no comma)
    let bad_send_bytes = b"SEND\x0a\x00\x00\x00/nocomma/a";
    let mut decoder = SyncDecoder::new(SyncDecoderMode::Request);
    decoder.feed(bad_send_bytes);
    assert!(matches!(
        decoder.pop_message(),
        Err(SyncDecodeError::InvalidPayload(_))
    ));
}

#[test]
fn test_oversized_encode_check() {
    let huge_data = vec![0u8; SYNC_DATA_MAX + 10];
    let msg = SyncMessage::Data { data: huge_data };
    assert_eq!(
        msg.encode_to_vec_checked(),
        Err(SyncEncodeError::PayloadTooLarge {
            length: SYNC_DATA_MAX + 10,
            max: SYNC_DATA_MAX,
        })
    );
}
