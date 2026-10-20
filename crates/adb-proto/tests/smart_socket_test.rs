use adb_proto::{
    encode_request, parse_protocol_string, parse_reply, ProtocolString, SmartSocketError,
    SmartSocketReply,
};

#[test]
fn test_encode_request_standard() {
    let encoded = encode_request("host:version").expect("should encode host:version");
    assert_eq!(encoded, b"000chost:version");

    let encoded = encode_request("host:devices").expect("should encode host:devices");
    assert_eq!(encoded, b"000chost:devices");

    let encoded = encode_request("").expect("should encode empty service");
    assert_eq!(encoded, b"0000");
}

#[test]
fn test_encode_request_max_length() {
    let max_service = "a".repeat(0xffff);
    let encoded = encode_request(&max_service).expect("should encode max length request");
    assert_eq!(&encoded[..4], b"ffff");
    assert_eq!(encoded.len(), 4 + 0xffff);

    let overflow_service = "a".repeat(0xffff + 1);
    let err = encode_request(&overflow_service).unwrap_err();
    assert_eq!(err, SmartSocketError::RequestTooLong(0x10000));
}

#[test]
fn test_parse_reply_okay() {
    let buf = b"OKAYextra_payload";
    let (reply, consumed) = parse_reply(buf).expect("should parse OKAY reply");
    assert_eq!(reply, SmartSocketReply::Okay);
    assert_eq!(consumed, 4);
}

#[test]
fn test_parse_reply_fail_standard() {
    let buf = b"FAIL001ano devices/emulators foundrest_of_buffer";
    let (reply, consumed) = parse_reply(buf).expect("should parse FAIL reply");
    assert_eq!(
        reply,
        SmartSocketReply::Fail("no devices/emulators found".to_string())
    );
    assert_eq!(consumed, 4 + 4 + 26);
}

#[test]
fn test_parse_reply_fail_zero_length() {
    let buf = b"FAIL0000tail";
    let (reply, consumed) = parse_reply(buf).expect("should parse FAIL reply with 0-len reason");
    assert_eq!(reply, SmartSocketReply::Fail("".to_string()));
    assert_eq!(consumed, 8);
}

#[test]
fn test_parse_reply_incomplete() {
    // Less than 4 bytes
    for i in 0..4 {
        let buf = &b"OKAY"[..i];
        let (reply, consumed) = parse_reply(buf).expect("should return incomplete for <4 bytes");
        assert_eq!(reply, SmartSocketReply::Incomplete);
        assert_eq!(consumed, 0);
    }

    // FAIL with missing length or reason bytes
    let partial_fail = b"FAIL0005hel";
    let (reply, consumed) =
        parse_reply(partial_fail).expect("should return incomplete for truncated FAIL");
    assert_eq!(reply, SmartSocketReply::Incomplete);
    assert_eq!(consumed, 0);
}

#[test]
fn test_parse_reply_split_reads() {
    let full = b"FAIL001ano devices/emulators found";
    for i in 0..full.len() {
        let slice = &full[..i];
        let (reply, consumed) = parse_reply(slice).expect("split read should not error");
        assert_eq!(reply, SmartSocketReply::Incomplete);
        assert_eq!(consumed, 0);
    }
    let (reply, consumed) = parse_reply(full).expect("full read should succeed");
    assert_eq!(
        reply,
        SmartSocketReply::Fail("no devices/emulators found".to_string())
    );
    assert_eq!(consumed, full.len());
}

#[test]
fn test_parse_reply_malformed() {
    // Non OKAY/FAIL status
    let buf = b"XXXX0000";
    let err = parse_reply(buf).unwrap_err();
    assert_eq!(err, SmartSocketError::InvalidStatus(*b"XXXX"));

    // Invalid hex length in FAIL
    let buf = b"FAILZZZZreason";
    let err = parse_reply(buf).unwrap_err();
    assert_eq!(err, SmartSocketError::InvalidHexLength("ZZZZ".to_string()));
}

#[test]
fn test_parse_protocol_string_complete() {
    let buf = b"0004001ftrail";
    let (parsed, consumed) =
        parse_protocol_string(buf).expect("should parse protocol string successfully");
    assert_eq!(parsed, ProtocolString::Complete("001f".to_string()));
    assert_eq!(consumed, 8);

    let empty_buf = b"0000trailer";
    let (parsed, consumed) =
        parse_protocol_string(empty_buf).expect("should parse empty protocol string");
    assert_eq!(parsed, ProtocolString::Complete("".to_string()));
    assert_eq!(consumed, 4);
}

#[test]
fn test_parse_protocol_string_incomplete() {
    for i in 0..4 {
        let buf = &b"0004"[..i];
        let (parsed, consumed) =
            parse_protocol_string(buf).expect("should return incomplete for <4 bytes");
        assert_eq!(parsed, ProtocolString::Incomplete);
        assert_eq!(consumed, 0);
    }

    let truncated = b"000400";
    let (parsed, consumed) =
        parse_protocol_string(truncated).expect("should return incomplete for truncated payload");
    assert_eq!(parsed, ProtocolString::Incomplete);
    assert_eq!(consumed, 0);
}

#[test]
fn test_parse_protocol_string_split_reads() {
    let full = b"001a12345678901234567890123456";
    for i in 0..full.len() {
        let slice = &full[..i];
        let (parsed, consumed) = parse_protocol_string(slice).expect("split read should not error");
        assert_eq!(parsed, ProtocolString::Incomplete);
        assert_eq!(consumed, 0);
    }
    let (parsed, consumed) = parse_protocol_string(full).expect("full read should succeed");
    assert_eq!(
        parsed,
        ProtocolString::Complete("12345678901234567890123456".to_string())
    );
    assert_eq!(consumed, full.len());
}

#[test]
fn test_parse_protocol_string_malformed() {
    let buf = b"ZZZZpayload";
    let err = parse_protocol_string(buf).unwrap_err();
    assert_eq!(err, SmartSocketError::InvalidHexLength("ZZZZ".to_string()));
}

#[test]
fn test_chained_reply_and_protocol_string() {
    // Simulating response to host:version => OKAY + 0004 + 0020
    let response_bytes = b"OKAY00040020";

    let (reply, consumed_reply) = parse_reply(response_bytes).expect("should parse OKAY");
    assert_eq!(reply, SmartSocketReply::Okay);
    assert_eq!(consumed_reply, 4);

    let (payload, consumed_payload) = parse_protocol_string(&response_bytes[consumed_reply..])
        .expect("should parse version string");
    assert_eq!(payload, ProtocolString::Complete("0020".to_string()));
    assert_eq!(consumed_payload, 8);

    assert_eq!(consumed_reply + consumed_payload, response_bytes.len());
}
