use adb_harness::{emulator, mock_server::{self, Traffic}};
use cmd_lib::run_fun;
use std::time::Duration;

fn parse_adb_messages(data: &[u8]) -> Vec<String> {
    let mut messages = Vec::new();
    let mut current_pos = 0;
    while current_pos + 4 <= data.len() {
        if let Ok(len_str) = std::str::from_utf8(&data[current_pos..current_pos + 4]) {
            if let Ok(len) = u32::from_str_radix(len_str, 16) {
                current_pos += 4;
                if current_pos + (len as usize) <= data.len() {
                    let msg_bytes = &data[current_pos..current_pos + len as usize];
                    messages.push(String::from_utf8_lossy(msg_bytes).to_string());
                    current_pos += len as usize;
                } else {
                    break;
                }
            } else {
                break;
            }
        } else {
            break;
        }
    }
    messages
}

#[test]
#[ignore] // This test requires a pre-configured emulator and takes a long time to run.
fn test_emulator_connect_and_shell() {
    // 1. Start the emulator and get its port.
    let emulator = emulator::start_and_get_port().expect("Failed to start emulator");
    let emulator_port = emulator.port();

    // 2. Start the mock server.
    let (mock_port, rx, _jh) =
        mock_server::start_mock_server(emulator_port).expect("Failed to start mock server");

    // Give the server thread a moment to start and bind the port.
    std::thread::sleep(Duration::from_millis(100));

    // 3. Connect to the mock server.
    run_fun!(adb connect localhost:$mock_port).unwrap();

    // 4. Run the shell command.
    let output = run_fun!(adb -s localhost:$mock_port shell echo hello world).unwrap();
    assert_eq!(output.trim(), "hello world");

    // 5. Assert the traffic.
    let mut client_traffic_bytes = Vec::new();
    while let Ok(traffic) = rx.recv_timeout(Duration::from_millis(100)) {
        if let Traffic::FromClient(data) = traffic {
            client_traffic_bytes.extend_from_slice(&data);
        }
    }

    let messages = parse_adb_messages(&client_traffic_bytes);
    assert!(messages.iter().any(|m| m.contains("shell:echo hello world")));
}
