use compat::{run_client_with_config, Config};
use std::env;
use std::net::TcpListener;

#[test]
fn test_run_client_timeout_panics() {
    // 1. Create a TCP listener that accepts connections and holds them open without replying
    let listener = TcpListener::bind("127.0.0.1:0").expect("Failed to bind TCP listener");
    let port = listener.local_addr().unwrap().port();

    let _accept_thread = std::thread::spawn(move || {
        let mut sockets = Vec::new();
        while let Ok((stream, _)) = listener.accept() {
            sockets.push(stream);
        }
    });

    // 2. Configure Config pointing at reference/client binary with server_port set to listener port
    let mut config = Config::from_env();
    config.server_port = Some(port);

    // 3. Set COMPAT_CMD_TIMEOUT_SECS to 2
    env::set_var("COMPAT_CMD_TIMEOUT_SECS", "2");

    // 4. Run client command expecting it to panic on timeout
    let result = std::panic::catch_unwind(|| {
        run_client_with_config(&config, &["shell", "echo", "hello"]);
    });

    assert!(
        result.is_err(),
        "Expected run_client_with_config to panic on timeout"
    );

    let panic_payload = result.unwrap_err();
    let panic_msg = if let Some(s) = panic_payload.downcast_ref::<String>() {
        s.clone()
    } else if let Some(s) = panic_payload.downcast_ref::<&str>() {
        s.to_string()
    } else {
        panic!("Unknown panic payload type");
    };

    assert!(
        panic_msg.contains("timed out"),
        "Panic message did not contain 'timed out': {panic_msg}"
    );
    assert!(
        panic_msg.contains("shell"),
        "Panic message did not contain command args: {panic_msg}"
    );
}
