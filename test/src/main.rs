use adb_harness::mock_server;
use anyhow::Result;

fn main() -> Result<()> {
    let (port, rx, jh) = mock_server::start_mock_server(5037)?;
    println!("Mock server started on port {port}");
    for msg in rx {
        println!("Received message: {msg:?}");
    }
    jh.join().expect("Handler thread panicked");
    Ok(())
}
