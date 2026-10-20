use adb_proto::{negotiate_max_payload, Banner, Command, Features, Header, A_VERSION, MAX_PAYLOAD};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;

pub struct MockAdbdServer {
    local_addr: SocketAddr,
    handle: JoinHandle<()>,
}

impl MockAdbdServer {
    pub async fn bind(addr: &str) -> std::io::Result<Self> {
        let listener = TcpListener::bind(addr).await?;
        let local_addr = listener.local_addr()?;

        let handle = tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                tokio::spawn(async move {
                    let _ = handle_connection(stream).await;
                });
            }
        });

        Ok(Self { local_addr, handle })
    }

    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    pub fn shutdown(self) {
        self.handle.abort();
    }
}

pub async fn read_packet<R: AsyncRead + Unpin>(
    reader: &mut R,
) -> std::io::Result<Option<(Header, Vec<u8>)>> {
    let mut header_buf = [0u8; Header::SIZE];
    match reader.read_exact(&mut header_buf).await {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }

    let header = Header::decode(&header_buf)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;

    let mut payload = vec![0u8; header.data_length as usize];
    if header.data_length > 0 {
        match reader.read_exact(&mut payload).await {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
            Err(e) => return Err(e),
        }
    }

    Ok(Some((header, payload)))
}

pub async fn write_packet<W: AsyncWrite + Unpin>(
    writer: &mut W,
    command: Command,
    arg0: u32,
    arg1: u32,
    payload: &[u8],
) -> std::io::Result<()> {
    let header = Header::new(command, arg0, arg1, payload.len() as u32, 0);
    writer.write_all(&header.encode()).await?;
    if !payload.is_empty() {
        writer.write_all(payload).await?;
    }
    writer.flush().await?;
    Ok(())
}

fn parse_echo_command(cmd: &str) -> Option<String> {
    let raw_text = if cmd == "echo" {
        ""
    } else {
        cmd.strip_prefix("echo ")?
    };

    let text = raw_text.trim();
    let text = if (text.starts_with('"') && text.ends_with('"') && text.len() >= 2)
        || (text.starts_with('\'') && text.ends_with('\'') && text.len() >= 2)
    {
        &text[1..text.len() - 1]
    } else {
        text
    };

    Some(format!("{text}\n"))
}

pub async fn handle_connection(stream: TcpStream) -> std::io::Result<()> {
    let (mut reader, mut writer) = stream.into_split();

    // 1. Handshake: Read host CNXN packet
    let host_cnxn = loop {
        match read_packet(&mut reader).await {
            Ok(Some((header, payload))) => {
                if header.command == Command::Cnxn {
                    break (header, payload);
                }
                // Both sides send CNXN first and ignore everything before it
            }
            Ok(None) => return Ok(()),
            Err(_) => return Ok(()),
        }
    };

    let host_max_payload = host_cnxn.0.arg1 as usize;
    let max_payload = negotiate_max_payload(host_max_payload, MAX_PAYLOAD);

    let banner = Banner {
        system_type: "device".to_string(),
        serial: "".to_string(),
        properties: vec![
            ("ro.product.name".to_string(), "mock_device".to_string()),
            ("ro.product.model".to_string(), "mock_model".to_string()),
            ("ro.product.device".to_string(), "mock_device".to_string()),
        ],
        features: Features::new(),
        ends_with_semicolon: true,
    };
    let banner_bytes = banner.to_bytes();

    if write_packet(
        &mut writer,
        Command::Cnxn,
        A_VERSION,
        max_payload as u32,
        &banner_bytes,
    )
    .await
    .is_err()
    {
        return Ok(());
    }

    let next_stream_id = Arc::new(AtomicU32::new(1));

    // 2. Stream loop
    loop {
        let (header, payload) = match read_packet(&mut reader).await {
            Ok(Some(pkt)) => pkt,
            Ok(None) => break,
            Err(_) => break,
        };

        match header.command {
            Command::Open => {
                let host_local_id = header.arg0;
                let service_raw = String::from_utf8_lossy(&payload);
                let service = service_raw.trim_matches('\0');

                if let Some(cmd) = service.strip_prefix("shell:") {
                    let mock_local_id = next_stream_id.fetch_add(1, Ordering::SeqCst);

                    // Accept stream
                    if write_packet(
                        &mut writer,
                        Command::Okay,
                        mock_local_id,
                        host_local_id,
                        &[],
                    )
                    .await
                    .is_err()
                    {
                        break;
                    }

                    // Process shell command
                    let output = parse_echo_command(cmd);

                    let response_payload = match output {
                        Some(text) => text.into_bytes(),
                        None => b"mock-adbd: unsupported command\n".to_vec(),
                    };

                    // Send WRTE
                    if write_packet(
                        &mut writer,
                        Command::Wrte,
                        mock_local_id,
                        host_local_id,
                        &response_payload,
                    )
                    .await
                    .is_err()
                    {
                        break;
                    }

                    // Wait for OKAY from host (1-WRTE-in-flight rule)
                    let mut got_okay = false;
                    while !got_okay {
                        match read_packet(&mut reader).await {
                            Ok(Some((ack_header, _))) => {
                                if ack_header.command == Command::Okay
                                    && ack_header.arg0 == host_local_id
                                    && ack_header.arg1 == mock_local_id
                                {
                                    got_okay = true;
                                }
                            }
                            Ok(None) => return Ok(()),
                            Err(_) => return Ok(()),
                        }
                    }

                    // Send CLSE
                    if write_packet(
                        &mut writer,
                        Command::Clse,
                        mock_local_id,
                        host_local_id,
                        &[],
                    )
                    .await
                    .is_err()
                    {
                        break;
                    }
                } else {
                    // Refuse unknown service with CLSE(0, host_local_id)
                    if write_packet(&mut writer, Command::Clse, 0, host_local_id, &[])
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
            }
            Command::Clse => {
                // Host requested close
            }
            _ => {
                // Ignore other packets
            }
        }
    }

    Ok(())
}
