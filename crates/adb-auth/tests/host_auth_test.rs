use adb_auth::{fingerprint, read_key_file, AuthAction, AuthEvent, HostAuth};
use adb_proto::{Command, ADB_AUTH_RSAPUBLICKEY, ADB_AUTH_SIGNATURE, MAX_PAYLOAD_V1};
use rsa::Pkcs1v15Sign;
use std::path::PathBuf;

fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

#[test]
fn test_case_1_one_key_valid_token_signature_sent() {
    let key_path = fixture_path("test_key");
    let priv_key = read_key_file(&key_path).expect("failed to load test key");
    let pub_key = priv_key.to_public_key();

    let user_pubkey = "ssh-rsa AAAAB3NzaC1yc2E... user@host".to_string();
    let mut host_auth = HostAuth::new(vec![priv_key], user_pubkey);

    let token = [0x42u8; 20];
    let actions = host_auth.handle_event(AuthEvent::AuthToken(&token));

    assert_eq!(actions.len(), 1);
    match &actions[0] {
        AuthAction::SendPacket {
            command,
            arg0,
            payload,
        } => {
            assert_eq!(*command, Command::Auth);
            assert_eq!(*arg0, ADB_AUTH_SIGNATURE);
            assert_eq!(payload.len(), 256); // 2048-bit key signature size

            // Verify signature against matching public key
            let scheme = Pkcs1v15Sign::new::<sha1::Sha1>();
            pub_key
                .verify(scheme, &token, payload)
                .expect("signature verification against public key failed");
        }
        _ => panic!("Expected SendPacket action"),
    }
}

#[test]
fn test_case_2_three_keys_rejecting_signatures_in_order_then_public_key() {
    let key1 = read_key_file(fixture_path("test_key")).expect("failed to load key1");
    // Generate two additional keys
    let mut rng = rand::thread_rng();
    let key2 = rsa::RsaPrivateKey::new(&mut rng, 2048).expect("failed key2");
    let key3 = rsa::RsaPrivateKey::new(&mut rng, 2048).expect("failed key3");

    let mut keys = vec![key1, key2, key3];
    // Sort keys by fingerprint ascending byte order matching task 2.1b
    keys.sort_by_key(fingerprint);

    let pubkeys: Vec<_> = keys.iter().map(|k| k.to_public_key()).collect();
    let user_pubkey = "test_pubkey_line user@host".to_string();

    let mut host_auth = HostAuth::new(keys, user_pubkey.clone());
    let token = [0x11u8; 20];

    // Response 1: signature from key 0
    let actions1 = host_auth.handle_event(AuthEvent::AuthToken(&token));
    assert_eq!(actions1.len(), 1);
    if let AuthAction::SendPacket {
        command,
        arg0,
        payload,
    } = &actions1[0]
    {
        assert_eq!(*command, Command::Auth);
        assert_eq!(*arg0, ADB_AUTH_SIGNATURE);
        pubkeys[0]
            .verify(Pkcs1v15Sign::new::<sha1::Sha1>(), &token, payload)
            .expect("key 0 signature failed");
    } else {
        panic!("expected SendPacket for key 0");
    }

    // Response 2: signature from key 1
    let actions2 = host_auth.handle_event(AuthEvent::AuthToken(&token));
    assert_eq!(actions2.len(), 1);
    if let AuthAction::SendPacket {
        command,
        arg0,
        payload,
    } = &actions2[0]
    {
        assert_eq!(*command, Command::Auth);
        assert_eq!(*arg0, ADB_AUTH_SIGNATURE);
        pubkeys[1]
            .verify(Pkcs1v15Sign::new::<sha1::Sha1>(), &token, payload)
            .expect("key 1 signature failed");
    } else {
        panic!("expected SendPacket for key 1");
    }

    // Response 3: signature from key 2
    let actions3 = host_auth.handle_event(AuthEvent::AuthToken(&token));
    assert_eq!(actions3.len(), 1);
    if let AuthAction::SendPacket {
        command,
        arg0,
        payload,
    } = &actions3[0]
    {
        assert_eq!(*command, Command::Auth);
        assert_eq!(*arg0, ADB_AUTH_SIGNATURE);
        pubkeys[2]
            .verify(Pkcs1v15Sign::new::<sha1::Sha1>(), &token, payload)
            .expect("key 2 signature failed");
    } else {
        panic!("expected SendPacket for key 2");
    }

    // Response 4: no keys left -> report unauthorized, report established, send public key
    let actions4 = host_auth.handle_event(AuthEvent::AuthToken(&token));
    assert_eq!(actions4.len(), 3);
    assert_eq!(actions4[0], AuthAction::ReportUnauthorized);
    assert_eq!(actions4[1], AuthAction::ReportEstablished);

    if let AuthAction::SendPacket {
        command,
        arg0,
        payload,
    } = &actions4[2]
    {
        assert_eq!(*command, Command::Auth);
        assert_eq!(*arg0, ADB_AUTH_RSAPUBLICKEY);
        // adbd expects null-terminated string
        let mut expected_payload = user_pubkey.as_bytes().to_vec();
        expected_payload.push(0);
        assert_eq!(payload, &expected_payload);
    } else {
        panic!("expected SendPacket for public key");
    }
}

#[test]
fn test_case_3_no_keys_at_all_public_key_goes_out_immediately() {
    let user_pubkey = "user_pubkey_content user@host".to_string();
    let mut host_auth = HostAuth::new(vec![], user_pubkey.clone());

    let token = [0x99u8; 20];
    let actions = host_auth.handle_event(AuthEvent::AuthToken(&token));

    assert_eq!(actions.len(), 3);
    assert_eq!(actions[0], AuthAction::ReportUnauthorized);
    assert_eq!(actions[1], AuthAction::ReportEstablished);

    if let AuthAction::SendPacket {
        command,
        arg0,
        payload,
    } = &actions[2]
    {
        assert_eq!(*command, Command::Auth);
        assert_eq!(*arg0, ADB_AUTH_RSAPUBLICKEY);
        let mut expected = user_pubkey.as_bytes().to_vec();
        expected.push(0);
        assert_eq!(payload, &expected);
    } else {
        panic!("expected SendPacket for public key");
    }
}

#[test]
fn test_case_4_token_wrong_size_nothing_sent_key_is_consumed() {
    // Following C++ original/client/auth.cpp: send_auth_response calls t->NextKey() FIRST,
    // then attempts adb_auth_sign. When adb_auth_sign fails due to token size != 20 bytes,
    // the signing error causes send_auth_response to return empty actions without sending a packet.
    // However, NextKey() was already called, so the key WAS consumed (popped) from the transport key list.
    let key1 = read_key_file(fixture_path("test_key")).expect("failed to load key1");
    let mut rng = rand::thread_rng();
    let key2 = rsa::RsaPrivateKey::new(&mut rng, 2048).expect("failed key2");

    let mut keys = vec![key1, key2];
    keys.sort_by_key(fingerprint);
    let pubkey2 = keys[1].to_public_key();

    let mut host_auth = HostAuth::new(keys, "pubkey user@host".to_string());

    // Send invalid token of length 15 (wrong size)
    let invalid_token = [0x55u8; 15];
    let actions_invalid = host_auth.handle_event(AuthEvent::AuthToken(&invalid_token));
    assert!(
        actions_invalid.is_empty(),
        "nothing sent on invalid token size"
    );

    // Now send valid token (length 20). Because key 0 was consumed during the invalid token call,
    // key 1 should now be used for this token.
    let valid_token = [0x77u8; 20];
    let actions_valid = host_auth.handle_event(AuthEvent::AuthToken(&valid_token));
    assert_eq!(actions_valid.len(), 1);

    if let AuthAction::SendPacket {
        command,
        arg0,
        payload,
    } = &actions_valid[0]
    {
        assert_eq!(*command, Command::Auth);
        assert_eq!(*arg0, ADB_AUTH_SIGNATURE);
        pubkey2
            .verify(Pkcs1v15Sign::new::<sha1::Sha1>(), &valid_token, payload)
            .expect("key 1 (second key) signature should match because key 0 was consumed");
    } else {
        panic!("expected SendPacket signature for key 1");
    }
}

#[test]
fn test_case_5_oversized_public_key_nothing_sent() {
    // In C++ send_auth_publickey: if (key.size() >= MAX_PAYLOAD_V1) { LOG(WARNING) ...; return; }
    // No packet is sent when user public key exceeds MAX_PAYLOAD_V1.
    let oversized_pubkey = "A".repeat(MAX_PAYLOAD_V1);
    let mut host_auth = HostAuth::new(vec![], oversized_pubkey);

    let token = [0x00u8; 20];
    let actions = host_auth.handle_event(AuthEvent::AuthToken(&token));

    // ReportUnauthorized and ReportEstablished are still issued when keys run out,
    // but the SendPacket for public key is suppressed due to size check.
    assert_eq!(actions.len(), 2);
    assert_eq!(actions[0], AuthAction::ReportUnauthorized);
    assert_eq!(actions[1], AuthAction::ReportEstablished);
}

#[test]
fn test_case_6_new_host_auth_after_reconnect_starts_from_first_key() {
    let key1 = read_key_file(fixture_path("test_key")).expect("failed to load key1");
    let mut rng = rand::thread_rng();
    let key2 = rsa::RsaPrivateKey::new(&mut rng, 2048).expect("failed key2");

    let mut keys = [key1, key2];
    keys.sort_by_key(fingerprint);
    let pubkey1 = keys[0].to_public_key();

    let store_keys = vec![keys[0].clone(), keys[1].clone()];

    let mut auth_conn1 = HostAuth::new(store_keys.clone(), "pubkey".to_string());
    let token = [0x33u8; 20];

    // Connection 1 consumes key 0
    let actions1 = auth_conn1.handle_event(AuthEvent::AuthToken(&token));
    assert_eq!(actions1.len(), 1);

    // Reconnect: create new HostAuth for connection 2 using same keys from store
    let mut auth_conn2 = HostAuth::new(store_keys, "pubkey".to_string());
    let actions2 = auth_conn2.handle_event(AuthEvent::AuthToken(&token));
    assert_eq!(actions2.len(), 1);

    if let AuthAction::SendPacket { payload, .. } = &actions2[0] {
        pubkey1
            .verify(Pkcs1v15Sign::new::<sha1::Sha1>(), &token, payload)
            .expect("new HostAuth after reconnect must start from key 0 again");
    } else {
        panic!("expected SendPacket signature for key 0");
    }
}

#[test]
fn test_case_7_cnxn_after_public_key_marks_connection_established() {
    let mut host_auth = HostAuth::new(vec![], "pubkey user@host".to_string());

    let token = [0x88u8; 20];
    let _actions_token = host_auth.handle_event(AuthEvent::AuthToken(&token));

    // After public key sent, user approves on device and device sends CNXN
    let actions_cnxn = host_auth.handle_event(AuthEvent::Cnxn);
    assert_eq!(actions_cnxn.len(), 1);
    assert_eq!(actions_cnxn[0], AuthAction::ReportEstablished);
}
