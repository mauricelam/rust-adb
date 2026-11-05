use adb_proto::smart_socket::{
    encode_fail, encode_okay, encode_okay_with_payload, encode_request, parse_protocol_string,
    parse_reply, Decoded, ReplyError, RequestError, ServiceName, SmartSocketReply,
};

#[test]
fn test_encode_request_standard() {
    let service = "host:version";
    let encoded = encode_request(service).expect("should encode valid service");
    assert_eq!(&encoded[0..4], b"000c");
    assert_eq!(&encoded[4..], service.as_bytes());
}

#[test]
fn test_encode_request_empty() {
    let err = encode_request("").unwrap_err();
    assert_eq!(err, RequestError::Empty);
}

#[test]
fn test_encode_request_too_long() {
    let service = "a".repeat(adb_proto::limits::MAX_PAYLOAD + 1);
    let err = encode_request(&service).unwrap_err();
    assert_eq!(
        err,
        RequestError::TooLong(adb_proto::limits::MAX_PAYLOAD + 1)
    );
}

#[test]
fn test_parse_reply_okay() {
    let buf = b"OKAY";
    let decoded = parse_reply(buf).expect("should parse OKAY reply");
    assert_eq!(
        decoded,
        Decoded::Complete {
            value: SmartSocketReply::Okay,
            consumed: 4,
        }
    );
}

#[test]
fn test_parse_reply_fail_standard() {
    let reason = "no devices/emulators found";
    let mut buf = Vec::new();
    buf.extend_from_slice(b"FAIL");
    buf.extend_from_slice(format!("{:04x}", reason.len()).as_bytes());
    buf.extend_from_slice(reason.as_bytes());
    let decoded = parse_reply(&buf).expect("should parse FAIL reply");
    assert_eq!(
        decoded,
        Decoded::Complete {
            value: SmartSocketReply::Fail(reason.to_string()),
            consumed: 8 + reason.len(),
        }
    );
}

#[test]
fn test_parse_reply_fail_zero_length() {
    let buf = b"FAIL0000";
    let decoded = parse_reply(buf).expect("should parse FAIL reply with 0-len reason");
    assert_eq!(
        decoded,
        Decoded::Complete {
            value: SmartSocketReply::Fail("".to_string()),
            consumed: 8,
        }
    );
}

#[test]
fn test_parse_reply_incomplete() {
    for len in 0..4 {
        let buf = &b"OKAY"[0..len];
        let decoded = parse_reply(buf).expect("should return incomplete for <4 bytes");
        assert_eq!(decoded, Decoded::Incomplete);
    }

    for len in 0..8 {
        let buf = &b"FAIL0005hello"[0..len];
        let decoded = parse_reply(buf).expect("should return incomplete for partial FAIL");
        assert_eq!(decoded, Decoded::Incomplete);
    }
}

#[test]
fn test_parse_reply_malformed() {
    let buf = b"XXXX";
    let err = parse_reply(buf).unwrap_err();
    assert_eq!(err, ReplyError::InvalidStatus(*b"XXXX"));

    let buf = b"FAILZZZZ";
    let err = parse_reply(buf).unwrap_err();
    assert_eq!(err, ReplyError::InvalidHexLength("ZZZZ".to_string()));

    let buf = b"FAIL+00c";
    let err = parse_reply(buf).unwrap_err();
    assert_eq!(err, ReplyError::InvalidHexLength("+00c".to_string()));
}

#[test]
fn test_parse_protocol_string_complete() {
    let buf = b"0004001f";
    let decoded = parse_protocol_string(buf).expect("should parse protocol string");
    assert_eq!(
        decoded,
        Decoded::Complete {
            value: "001f".to_string(),
            consumed: 8,
        }
    );
}

#[test]
fn test_parse_protocol_string_incomplete() {
    for len in 0..4 {
        let buf = &b"0005hello"[0..len];
        let decoded = parse_protocol_string(buf).expect("should return incomplete");
        assert_eq!(decoded, Decoded::Incomplete);
    }
}

#[test]
fn test_parse_protocol_string_malformed() {
    let buf = b"ZZZZ";
    let err = parse_protocol_string(buf).unwrap_err();
    assert_eq!(err, ReplyError::InvalidHexLength("ZZZZ".to_string()));

    let buf = b"+005";
    let err = parse_protocol_string(buf).unwrap_err();
    assert_eq!(err, ReplyError::InvalidHexLength("+005".to_string()));
}

#[test]
fn test_encode_okay_fail() {
    assert_eq!(encode_okay(), b"OKAY");
    assert_eq!(encode_okay_with_payload("123"), b"OKAY0003123");
    assert_eq!(encode_fail("err"), b"FAIL0003err");
}

#[test]
fn test_service_name_validation() {
    assert!(ServiceName::new("").is_err());
    assert!(ServiceName::new("host:version").is_ok());
    let sn = ServiceName::new("host:version").unwrap();
    assert_eq!(sn.as_str(), "host:version");
}
