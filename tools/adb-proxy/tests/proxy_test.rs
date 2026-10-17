use adb_proxy::{Direction, ProxyServer, Record};
use std::io::{BufRead, BufReader};
use std::process::Stdio;
use tempfile::NamedTempFile;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::process::Command;
use tokio::sync::oneshot;

async fn start_echo_server() -> (String, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap().to_string();

    let handle = tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            tokio::spawn(async move {
                let mut buf = [0u8; 4096];
                loop {
                    match stream.read(&mut buf).await {
                        Ok(0) => break,
                        Ok(n) => {
                            if stream.write_all(&buf[..n]).await.is_err() {
                                break;
                            }
                        }
                        Err(_) => break,
                    }
                }
            });
        }
    });

    (addr, handle)
}

#[tokio::test]
async fn test_proxy_forwarding_and_recording() {
    let (echo_addr, _echo_handle) = start_echo_server().await;

    let temp_file = NamedTempFile::new().unwrap();
    let out_path = temp_file.path().to_path_buf();

    let proxy = ProxyServer::bind("127.0.0.1:0", &echo_addr, &out_path)
        .await
        .unwrap();
    let proxy_addr = proxy.local_addr().unwrap();

    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    let proxy_handle = tokio::spawn(async move {
        proxy
            .run_until(async {
                let _ = shutdown_rx.await;
            })
            .await
            .unwrap();
    });

    let mut client = TcpStream::connect(proxy_addr).await.unwrap();

    let payload1 = b"0012host:version";
    client.write_all(payload1).await.unwrap();

    let mut response1 = vec![0u8; payload1.len()];
    client.read_exact(&mut response1).await.unwrap();
    assert_eq!(&response1, payload1);

    let payload2 = b"000c0004OKAY0004";
    client.write_all(payload2).await.unwrap();

    let mut response2 = vec![0u8; payload2.len()];
    client.read_exact(&mut response2).await.unwrap();
    assert_eq!(&response2, payload2);

    drop(client);

    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    let _ = shutdown_tx.send(());
    proxy_handle.await.unwrap();

    let file = std::fs::File::open(&out_path).unwrap();
    let reader = BufReader::new(file);
    let mut records: Vec<Record> = Vec::new();

    for line in reader.lines() {
        let line = line.unwrap();
        if line.trim().is_empty() {
            continue;
        }
        let record: Record = serde_json::from_str(&line).unwrap();
        records.push(record);
    }

    assert!(
        !records.is_empty(),
        "Expected recorded records in fixture file"
    );

    let mut c2s_bytes = Vec::new();
    let mut s2c_bytes = Vec::new();
    let mut prev_timestamp = 0u64;

    for record in &records {
        assert!(
            record.timestamp_us >= prev_timestamp,
            "Timestamps must be monotonic"
        );
        prev_timestamp = record.timestamp_us;

        let decoded = hex::decode(&record.bytes).expect("Bytes should be valid hex");
        match record.direction {
            Direction::ClientToServer => c2s_bytes.extend_from_slice(&decoded),
            Direction::ServerToClient => s2c_bytes.extend_from_slice(&decoded),
        }
    }

    let mut expected_all = Vec::new();
    expected_all.extend_from_slice(payload1);
    expected_all.extend_from_slice(payload2);

    assert_eq!(c2s_bytes, expected_all);
    assert_eq!(s2c_bytes, expected_all);
}

#[tokio::test]
async fn test_proxy_cli_binary() {
    let (echo_addr, _echo_handle) = start_echo_server().await;

    let temp_file = NamedTempFile::new().unwrap();
    let out_path = temp_file.path().to_path_buf();

    let bin_path = env!("CARGO_BIN_EXE_adb-proxy");

    let mut child = Command::new(bin_path)
        .arg("--listen")
        .arg("127.0.0.1:0")
        .arg("--target")
        .arg(&echo_addr)
        .arg("--out")
        .arg(&out_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("Failed to spawn adb-proxy binary");

    let stdout = child.stdout.take().expect("Failed to open stdout");
    let mut reader = tokio::io::BufReader::new(stdout).lines();

    let mut proxy_addr = None;
    while let Ok(Some(line)) = reader.next_line().await {
        if line.starts_with("Listening on ") {
            let addr_str = line.trim_start_matches("Listening on ").trim();
            proxy_addr = Some(addr_str.to_string());
            break;
        }
    }

    let proxy_addr = proxy_addr.expect("Proxy binary should output Listening on <addr>");

    let mut client = TcpStream::connect(&proxy_addr).await.unwrap();

    let payload = b"HELLO_ADB_CLI_PROXY";
    client.write_all(payload).await.unwrap();

    let mut response = vec![0u8; payload.len()];
    client.read_exact(&mut response).await.unwrap();
    assert_eq!(&response, payload);

    drop(client);

    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    let _ = child.kill().await;

    let file = std::fs::File::open(&out_path).unwrap();
    let buf_reader = BufReader::new(file);
    let mut c2s_bytes = Vec::new();
    let mut s2c_bytes = Vec::new();

    for line in buf_reader.lines() {
        let line = line.unwrap();
        if line.trim().is_empty() {
            continue;
        }
        let record: Record = serde_json::from_str(&line).unwrap();
        let decoded = hex::decode(&record.bytes).unwrap();
        match record.direction {
            Direction::ClientToServer => c2s_bytes.extend_from_slice(&decoded),
            Direction::ServerToClient => s2c_bytes.extend_from_slice(&decoded),
        }
    }

    assert_eq!(c2s_bytes, payload);
    assert_eq!(s2c_bytes, payload);
}
