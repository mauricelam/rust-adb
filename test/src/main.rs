use adb_harness::mock_server;
use anyhow::anyhow;

fn main() -> anyhow::Result<()> {
    let (port, rx) = mock_server::start_mock_server()?;
    println!("Mock server started on port {port}");
    for msg in rx {
        println!("Received message: {msg}");
    }
    Ok(())
}
