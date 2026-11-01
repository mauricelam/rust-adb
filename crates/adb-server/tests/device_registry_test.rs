use std::sync::Arc;
use std::thread;

use adb_server::{ConnectionState, DeviceRegistry, RegistrationError, TransportType};

#[test]
fn test_registry_register_and_lookup() {
    let registry = DeviceRegistry::new();

    let id1 = registry
        .register("serial-1", TransportType::Usb)
        .expect("registration should succeed");
    assert_eq!(id1, 1);

    let id2 = registry
        .register("serial-2", TransportType::Local)
        .expect("registration should succeed");
    assert_eq!(id2, 2);

    // Lookup by serial
    let dev1 = registry
        .get_by_serial("serial-1")
        .expect("should find serial-1");
    assert_eq!(dev1.transport_id, 1);
    assert_eq!(dev1.serial, "serial-1");
    assert_eq!(dev1.transport_type, TransportType::Usb);
    assert_eq!(dev1.state, ConnectionState::Offline);

    // Lookup by transport_id
    let dev2 = registry.get_by_id(2).expect("should find id 2");
    assert_eq!(dev2.serial, "serial-2");
    assert_eq!(dev2.transport_type, TransportType::Local);
    assert_eq!(dev2.state, ConnectionState::Offline);

    // Non-existent lookup
    assert!(registry.get_by_serial("serial-3").is_none());
    assert!(registry.get_by_id(99).is_none());

    // List all
    let list = registry.list_devices();
    assert_eq!(list.len(), 2);
}

#[test]
fn test_registry_duplicate_serial_rejected() {
    let registry = DeviceRegistry::new();

    let id1 = registry
        .register("emulator-5554", TransportType::Local)
        .expect("first registration should succeed");
    assert_eq!(id1, 1);

    // Attempt registering duplicate serial
    let err = registry
        .register("emulator-5554", TransportType::Local)
        .expect_err("duplicate registration must fail");

    assert_eq!(err, RegistrationError::AlreadyRegistered);

    // Confirm registry still has only 1 device
    let list = registry.list_devices();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].transport_id, 1);
}

#[test]
fn test_registry_unregister() {
    let registry = DeviceRegistry::new();

    let id = registry
        .register("test-serial", TransportType::Usb)
        .expect("registration should succeed");

    assert!(registry.get_by_id(id).is_some());
    assert!(registry.get_by_serial("test-serial").is_some());

    // Unregister by id
    let removed = registry.unregister_by_id(id);
    assert!(removed.is_some());
    assert_eq!(removed.unwrap().serial, "test-serial");

    assert!(registry.get_by_id(id).is_none());
    assert!(registry.get_by_serial("test-serial").is_none());
    assert!(registry.list_devices().is_empty());

    // Now re-registering the same serial succeeds
    let new_id = registry
        .register("test-serial", TransportType::Usb)
        .expect("re-registration after unregister should succeed");
    assert_eq!(new_id, 2); // TransportId is monotonic
}

#[test]
fn test_registry_update_state_and_properties() {
    let registry = DeviceRegistry::new();

    let id = registry
        .register("device-1", TransportType::Usb)
        .expect("register succeeds");

    assert_eq!(
        registry.get_by_id(id).unwrap().state,
        ConnectionState::Offline
    );

    // Update state
    let ok = registry.update_state(id, ConnectionState::Authorizing);
    assert!(ok);
    assert_eq!(
        registry.get_by_id(id).unwrap().state,
        ConnectionState::Authorizing
    );

    // Update properties
    let ok_prop = registry.set_device_properties(
        id,
        Some("walleye".to_string()),
        Some("Pixel 2".to_string()),
        Some("walleye".to_string()),
    );
    assert!(ok_prop);

    let dev = registry.get_by_id(id).unwrap();
    assert_eq!(dev.product.as_deref(), Some("walleye"));
    assert_eq!(dev.model.as_deref(), Some("Pixel 2"));
    assert_eq!(dev.device.as_deref(), Some("walleye"));

    // Updating non-existent id returns false
    assert!(!registry.update_state(999, ConnectionState::Device));
    assert!(!registry.set_device_properties(999, None, None, None));
}

#[test]
fn test_registry_concurrent_access() {
    let registry = Arc::new(DeviceRegistry::new());
    let mut handles = Vec::new();

    for i in 0..10 {
        let reg = Arc::clone(&registry);
        let handle = thread::spawn(move || {
            let serial = format!("serial-worker-{}", i);
            let id = reg
                .register(&serial, TransportType::Usb)
                .expect("worker registration should succeed");

            reg.update_state(id, ConnectionState::Device);
            let dev = reg.get_by_id(id).expect("should find device");
            assert_eq!(dev.state, ConnectionState::Device);
        });
        handles.push(handle);
    }

    for h in handles {
        h.join().expect("thread panicked");
    }

    let devices = registry.list_devices();
    assert_eq!(devices.len(), 10);
}
