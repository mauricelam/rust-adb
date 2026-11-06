use adb_auth::HostAuth;
use adb_proto::{Command, ADB_AUTH_RSAPUBLICKEY, ADB_AUTH_SIGNATURE};
use adb_server::{ConnectionState, DeviceStateMachine, DeviceStateMachineEvent};
use rsa::RsaPrivateKey;

fn generate_test_key() -> RsaPrivateKey {
    let mut rng = rand::thread_rng();
    RsaPrivateKey::new(&mut rng, 2048).expect("failed to generate test RSA key")
}

#[test]
fn test_state_machine_initial_state() {
    let key = generate_test_key();
    let auth = HostAuth::new(vec![key], "test_pubkey user@host".to_string());
    let sm = DeviceStateMachine::new(auth);

    assert_eq!(sm.state(), ConnectionState::Offline);
    assert!(!sm.is_online());
}

#[test]
fn test_state_machine_auth_success() {
    let key = generate_test_key();
    let auth = HostAuth::new(vec![key], "test_pubkey user@host".to_string());
    let mut sm = DeviceStateMachine::new(auth);

    let token = [42u8; 20];
    let output = sm.handle_event(DeviceStateMachineEvent::AuthToken(&token));

    // After receiving AuthToken, state transitions to Authorizing
    assert_eq!(sm.state(), ConnectionState::Authorizing);
    assert!(!sm.is_online());

    // Should have emitted a signature packet
    assert_eq!(output.packets.len(), 1);
    assert_eq!(output.packets[0].command, Command::Auth);
    assert_eq!(output.packets[0].arg0, ADB_AUTH_SIGNATURE);

    // Device accepts signature and responds with CNXN
    let output_cnxn = sm.handle_event(DeviceStateMachineEvent::Cnxn {
        state: ConnectionState::Device,
    });

    assert_eq!(sm.state(), ConnectionState::Device);
    assert!(sm.is_online());
    assert!(output_cnxn.packets.is_empty());
}

#[test]
fn test_state_machine_auth_failure_to_unauthorized_and_subsequent_approval() {
    let key = generate_test_key();
    let pubkey_str = "test_pubkey user@host";
    let auth = HostAuth::new(vec![key], pubkey_str.to_string());
    let mut sm = DeviceStateMachine::new(auth);

    let token = [1u8; 20];
    // First token consumed the only private key
    let output1 = sm.handle_event(DeviceStateMachineEvent::AuthToken(&token));
    assert_eq!(sm.state(), ConnectionState::Authorizing);
    assert_eq!(output1.packets.len(), 1);

    // Second token: no private keys left, falls back to public key and unauthorized
    let token2 = [2u8; 20];
    let output2 = sm.handle_event(DeviceStateMachineEvent::AuthToken(&token2));
    assert_eq!(sm.state(), ConnectionState::Unauthorized);
    assert!(!sm.is_online());

    // Public key packet sent
    assert_eq!(output2.packets.len(), 1);
    assert_eq!(output2.packets[0].command, Command::Auth);
    assert_eq!(output2.packets[0].arg0, ADB_AUTH_RSAPUBLICKEY);

    // User subsequently taps "Allow" on the device screen -> CNXN
    let output_cnxn = sm.handle_event(DeviceStateMachineEvent::Cnxn {
        state: ConnectionState::Device,
    });
    assert_eq!(sm.state(), ConnectionState::Device);
    assert!(sm.is_online());
    assert!(output_cnxn.packets.is_empty());
}

#[test]
fn test_state_machine_disconnection() {
    let key = generate_test_key();
    let auth = HostAuth::new(vec![key], "test_pubkey user@host".to_string());
    let mut sm = DeviceStateMachine::new(auth);

    // Start offline, move to device
    sm.handle_event(DeviceStateMachineEvent::Cnxn {
        state: ConnectionState::Device,
    });
    assert_eq!(sm.state(), ConnectionState::Device);

    // Disconnect event
    sm.handle_event(DeviceStateMachineEvent::Disconnected);
    assert_eq!(sm.state(), ConnectionState::Offline);
    assert!(!sm.is_online());
}

#[test]
fn test_state_machine_recovery_and_sideload_service_types() {
    let key = generate_test_key();
    let auth = HostAuth::new(vec![key], "test_pubkey user@host".to_string());
    let mut sm = DeviceStateMachine::new(auth);

    sm.handle_event(DeviceStateMachineEvent::cnxn_from_service_type("recovery"));
    assert_eq!(sm.state(), ConnectionState::Recovery);
    assert!(sm.is_online());

    sm.handle_event(DeviceStateMachineEvent::cnxn_from_service_type("sideload"));
    assert_eq!(sm.state(), ConnectionState::Sideload);
    assert!(sm.is_online());

    sm.handle_event(DeviceStateMachineEvent::cnxn_from_service_type(
        "bootloader",
    ));
    assert_eq!(sm.state(), ConnectionState::Bootloader);
    assert!(sm.is_online());

    sm.handle_event(DeviceStateMachineEvent::cnxn_from_service_type("rescue"));
    assert_eq!(sm.state(), ConnectionState::Rescue);
    assert!(sm.is_online());
}
