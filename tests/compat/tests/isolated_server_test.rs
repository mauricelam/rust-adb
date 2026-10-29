use compat::IsolatedServer;
use std::env;
use std::net::TcpStream;
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::Arc;

#[test]
fn test_isolated_server_environment_isolation() {
    // Set bogus parent environment variables
    env::set_var("ANDROID_SERIAL", "bogus_serial_1234");
    env::set_var("ADB_SERVER_SOCKET", "tcp:localhost:59999");

    let server = IsolatedServer::new();
    // Verify server port is distinct from parent socket
    assert_ne!(server.port, 59999);

    // Running kill-server on isolated server when no server running should succeed/complete cleanly targeting its own port
    let output = server.run_client(&["kill-server"]);
    assert!(
        output.stderr.contains(&server.port.to_string())
            || output.stderr.contains("cannot connect to daemon")
            || output.stdout.is_empty(),
        "Output should reflect isolated port or clean execution"
    );

    // Clean up env vars
    env::remove_var("ANDROID_SERIAL");
    env::remove_var("ADB_SERVER_SOCKET");
}

#[test]
fn test_isolated_server_free_port() {
    let server1 = IsolatedServer::new();
    let server2 = IsolatedServer::new();

    assert_ne!(
        server1.port, server2.port,
        "IsolatedServer instances should pick different free ports"
    );
}

#[test]
fn test_isolated_server_drop_cleanup_on_panic() {
    let captured_port = Arc::new(AtomicU16::new(0));
    let captured_port_clone = captured_port.clone();

    let result = std::panic::catch_unwind(move || {
        let server = IsolatedServer::new();
        captured_port_clone.store(server.port, Ordering::SeqCst);

        let start_out = server.run_client(&["start-server"]);
        assert!(
            start_out.success(),
            "start-server failed: stdout='{}', stderr='{}'",
            start_out.stdout,
            start_out.stderr
        );

        // Confirm server is running on port
        assert!(
            TcpStream::connect(("127.0.0.1", server.port)).is_ok(),
            "Server should be running on port {}",
            server.port
        );

        // Panic while `server` is in scope
        panic!("Simulated panic inside test closure");
    });

    assert!(result.is_err(), "Expected closure to panic");

    let port = captured_port.load(Ordering::SeqCst);
    assert!(port > 0, "Port should have been captured before panic");

    // Check that port is closed after `server` was dropped during unwinding
    assert!(
        TcpStream::connect(("127.0.0.1", port)).is_err(),
        "Server port {} should be closed after drop on panic",
        port
    );
}
