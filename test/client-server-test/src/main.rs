use adb_client_server_test::mock_server;
use anyhow::anyhow;

fn main() -> anyhow::Result<()> {
    let (port, rx, jh) = mock_server::start_mock_server()?;
    println!("Mock server started on port {port}");
    for msg in rx {
        println!("Received message: {msg}");
    }
    jh.join()
        .map_err(|e| anyhow!("Failed to join thread: {e:?}"))?;
    Ok(())
}
