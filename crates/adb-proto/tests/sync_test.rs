use adb_proto::sync::{
    DataChunk, FileMode, FileType, RemotePath, SyncDecodeError, SyncDentV1, SyncDentV2, SyncId,
    SyncRecvV2, SyncRequest, SyncRequestDecoder, SyncResponse, SyncResponseDecoder, SyncSendV2,
    SyncStatV1, SyncStatV2, UnixTime, ID_DATA, ID_DENT_V1, ID_DONE, ID_FAIL, ID_LIST_V1,
    ID_LSTAT_V1, ID_OKAY, ID_QUIT, ID_RECV_V1, ID_SEND_V1,
};

#[test]
fn test_sync_id_conversion() {
    assert_eq!(SyncId::try_from(ID_LIST_V1), Ok(SyncId::ListV1));
    assert_eq!(SyncId::try_from(ID_DENT_V1), Ok(SyncId::DentV1));
    assert_eq!(SyncId::try_from(ID_LSTAT_V1), Ok(SyncId::LstatV1));
    assert_eq!(SyncId::try_from(ID_SEND_V1), Ok(SyncId::SendV1));
    assert_eq!(SyncId::try_from(ID_RECV_V1), Ok(SyncId::RecvV1));
    assert_eq!(SyncId::try_from(ID_DATA), Ok(SyncId::Data));
    assert_eq!(SyncId::try_from(ID_DONE), Ok(SyncId::Done));
    assert_eq!(SyncId::try_from(ID_OKAY), Ok(SyncId::Okay));
    assert_eq!(SyncId::try_from(ID_FAIL), Ok(SyncId::Fail));
    assert_eq!(SyncId::try_from(ID_QUIT), Ok(SyncId::Quit));
    assert_eq!(SyncId::try_from(0x12345678), Err(0x12345678));
}

#[test]
fn test_file_mode_and_file_type() {
    let reg = FileMode::new(0o100644);
    assert_eq!(reg.file_type(), FileType::Regular);
    assert!(reg.is_file());
    assert!(!reg.is_dir());

    let dir = FileMode::new(0o040755);
    assert_eq!(dir.file_type(), FileType::Directory);
    assert!(dir.is_dir());

    let lnk = FileMode::new(0o120777);
    assert_eq!(lnk.file_type(), FileType::Symlink);
    assert!(lnk.is_symlink());
}

#[test]
fn test_sync_request_round_trip() {
    let mut decoder = SyncRequestDecoder::new();

    let list_req = SyncRequest::List(RemotePath::new(b"/sdcard".to_vec()).unwrap());
    decoder.feed(&list_req.encode());
    assert_eq!(decoder.pop_request().unwrap(), Some(list_req));

    let stat_req = SyncRequest::Stat(RemotePath::new(b"/data/local/tmp".to_vec()).unwrap());
    decoder.feed(&stat_req.encode());
    assert_eq!(decoder.pop_request().unwrap(), Some(stat_req));

    let send_req = SyncRequest::Send {
        path: RemotePath::new(b"/sdcard/foo.txt".to_vec()).unwrap(),
        mode: FileMode::new(0o644),
    };
    decoder.feed(&send_req.encode());
    assert_eq!(decoder.pop_request().unwrap(), Some(send_req));

    let data_req = SyncRequest::Data(DataChunk::new(b"hello world".to_vec()).unwrap());
    decoder.feed(&data_req.encode());
    assert_eq!(decoder.pop_request().unwrap(), Some(data_req));

    let done_req = SyncRequest::Done {
        mtime: UnixTime::from_secs(123456),
    };
    decoder.feed(&done_req.encode());
    assert_eq!(decoder.pop_request().unwrap(), Some(done_req));

    let quit_req = SyncRequest::Quit;
    decoder.feed(&quit_req.encode());
    assert_eq!(decoder.pop_request().unwrap(), Some(quit_req));
}

#[test]
fn test_sync_response_round_trip() {
    let mut decoder = SyncResponseDecoder::new();

    let dent_resp = SyncResponse::Dent {
        mode: FileMode::new(0o100644),
        size: 42,
        mtime: UnixTime::from_secs(100),
        name: b"file.txt".to_vec(),
    };
    decoder.feed(&dent_resp.encode());
    assert_eq!(decoder.pop_response().unwrap(), Some(dent_resp));

    let stat_resp = SyncResponse::StatReply {
        mode: FileMode::new(0o100644),
        size: 1024,
        mtime: UnixTime::from_secs(200),
    };
    decoder.feed(&stat_resp.encode());
    assert_eq!(decoder.pop_response().unwrap(), Some(stat_resp));

    let okay_resp = SyncResponse::Okay;
    decoder.feed(&okay_resp.encode());
    assert_eq!(decoder.pop_response().unwrap(), Some(okay_resp));

    let fail_resp = SyncResponse::Fail {
        message: "No such file".to_string(),
    };
    decoder.feed(&fail_resp.encode());
    let err = decoder.pop_response().unwrap_err();
    assert_eq!(
        err,
        SyncDecodeError::FailMessage("No such file".to_string())
    );
}

#[test]
fn test_non_utf8_path() {
    let non_utf8_bytes = vec![0xff, 0xfe, 0xfd];
    let path = RemotePath::new(non_utf8_bytes.clone()).unwrap();
    assert_eq!(path.as_bytes(), &non_utf8_bytes);

    let req = SyncRequest::List(path);
    let encoded = req.encode();
    let mut decoder = SyncRequestDecoder::new();
    decoder.feed(&encoded);
    let decoded = decoder.pop_request().unwrap().unwrap();
    if let SyncRequest::List(p) = decoded {
        assert_eq!(p.as_bytes(), &non_utf8_bytes);
    } else {
        panic!("unexpected request type");
    }
}

#[test]
fn test_wire_struct_sizes() {
    assert_eq!(std::mem::size_of::<SyncStatV1>(), 16);
    assert_eq!(std::mem::size_of::<SyncStatV2>(), 72);
    assert_eq!(std::mem::size_of::<SyncDentV1>(), 20);
    assert_eq!(std::mem::size_of::<SyncDentV2>(), 76);
    assert_eq!(std::mem::size_of::<SyncSendV2>(), 12);
    assert_eq!(std::mem::size_of::<SyncRecvV2>(), 8);
}
