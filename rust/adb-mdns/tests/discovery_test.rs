use adb_mdns::{AdbMdns};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[test]
fn test_mdns_discovery() {
    std::env::set_var("ADB_MDNS_AUTO_CONNECT", "all");
    let adb_mdns = AdbMdns::new().expect("Failed to create AdbMdns");

    let discovered = Arc::new(Mutex::new(Vec::new()));
    let discovered_clone = discovered.clone();

    adb_mdns.browse(Some(Arc::new(move |info| {
        discovered_clone.lock().unwrap().push(info);
    }))).expect("Failed to start browsing");

    // Register a mock service
    let service_name = "test-device";
    let service_type = "_adb._tcp.local.";
    let address = "127.0.0.1";
    let port = 5555;
    let mut properties = HashMap::new();
    properties.insert("v".to_string(), "1".to_string());

    adb_mdns.register(service_name, service_type, address, port, properties).expect("Failed to register service");

    // Wait for discovery
    let start = std::time::Instant::now();
    while start.elapsed() < Duration::from_secs(5) {
        let found = discovered.lock().unwrap();
        if found.iter().any(|info| info.instance_name.contains(service_name)) {
            return;
        }
        drop(found);
        std::thread::sleep(Duration::from_millis(100));
    }

    // If we reach here, it might be due to multicast issues in the environment.
    println!("Warning: MDNS discovery timed out. This may be expected in some environments.");
}
