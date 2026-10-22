use adb_proto::{
    negotiate_max_payload, negotiate_version, Banner, Command, Feature, Features, Header,
    ShellDecoder, ShellPacket, ShellPacketId, A_VERSION, A_VERSION_SKIP_CHECKSUM, MAX_PAYLOAD,
};
use std::collections::{HashMap, VecDeque};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;

fn mock_log(msg: &str) {
    if std::env::var_os("MOCK_ADBD_LOG").is_some() {
        eprintln!("[mock-adbd] {msg}");
    }
}

#[derive(Debug, Clone)]
pub struct MockAdbdOptions {
    pub enable_shell_v2: bool,
}

impl Default for MockAdbdOptions {
    fn default() -> Self {
        Self {
            enable_shell_v2: true,
        }
    }
}

pub struct MockAdbdServer {
    local_addr: SocketAddr,
    accept_handle: JoinHandle<()>,
    tasks: Arc<Mutex<Vec<JoinHandle<()>>>>,
}

impl MockAdbdServer {
    pub async fn bind(addr: &str) -> std::io::Result<Self> {
        Self::bind_with_options(addr, MockAdbdOptions::default()).await
    }

    pub async fn bind_with_options(addr: &str, options: MockAdbdOptions) -> std::io::Result<Self> {
        let listener = TcpListener::bind(addr).await?;
        let local_addr = listener.local_addr()?;
        let tasks = Arc::new(Mutex::new(Vec::new()));

        let tasks_clone = tasks.clone();
        let accept_handle = tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let options = options.clone();
                let handle = tokio::spawn(async move {
                    if let Err(e) = handle_connection_with_options(stream, options).await {
                        mock_log(&format!("Connection error: {e}"));
                    }
                });
                if let Ok(mut guard) = tasks_clone.lock() {
                    guard.push(handle);
                }
            }
        });

        Ok(Self {
            local_addr,
            accept_handle,
            tasks,
        })
    }

    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    pub fn shutdown(self) {
        self.accept_handle.abort();
        if let Ok(mut guard) = self.tasks.lock() {
            for handle in guard.drain(..) {
                handle.abort();
            }
        }
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
    write_packet_versioned(writer, command, arg0, arg1, payload, A_VERSION).await
}

pub async fn write_packet_versioned<W: AsyncWrite + Unpin>(
    writer: &mut W,
    command: Command,
    arg0: u32,
    arg1: u32,
    payload: &[u8],
    version: u32,
) -> std::io::Result<()> {
    let crc = if version < A_VERSION_SKIP_CHECKSUM {
        payload.iter().map(|&b| b as u32).sum()
    } else {
        0
    };
    let header = Header::new(command, arg0, arg1, payload.len() as u32, crc);
    writer.write_all(&header.encode()).await?;
    if !payload.is_empty() {
        writer.write_all(payload).await?;
    }
    writer.flush().await?;
    Ok(())
}

fn parse_text_command(cmd: &str, prefix: &str) -> Option<String> {
    let raw_text = if cmd == prefix {
        ""
    } else {
        let p = format!("{prefix} ");
        cmd.strip_prefix(&p)?
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

fn parse_exit_command(cmd: &str) -> Option<u8> {
    if cmd == "exit" {
        return Some(0);
    }
    let arg = cmd.strip_prefix("exit ")?.trim();
    if arg.is_empty() {
        return Some(0);
    }
    arg.parse::<u8>()
        .ok()
        .or_else(|| arg.parse::<u32>().ok().map(|n| (n % 256) as u8))
}

fn format_winsz_payload(data: &[u8]) -> Vec<u8> {
    let text = String::from_utf8_lossy(data)
        .trim_end_matches('\0')
        .to_string();
    ShellPacket::new(
        ShellPacketId::Stdout,
        format!("WINSZ {text}\n").into_bytes(),
    )
    .encode_to_vec()
}

enum StreamKind {
    ShellV1,
    ShellV2NonCat { decoder: ShellDecoder, is_pty: bool },
    ShellV2Cat { decoder: ShellDecoder, is_pty: bool },
}

struct StreamState {
    mock_local_id: u32,
    host_local_id: u32,
    kind: StreamKind,
    write_in_flight: bool,
    pending_writes: VecDeque<Vec<u8>>,
    close_pending: bool,
}

pub async fn handle_connection(stream: TcpStream) -> std::io::Result<()> {
    handle_connection_with_options(stream, MockAdbdOptions::default()).await
}

pub async fn handle_connection_with_options(
    stream: TcpStream,
    options: MockAdbdOptions,
) -> std::io::Result<()> {
    let (mut reader, mut writer) = stream.into_split();

    // 1. Handshake: Read host CNXN packet
    let host_cnxn = loop {
        match read_packet(&mut reader).await {
            Ok(Some((header, payload))) => {
                if header.command == Command::Cnxn {
                    break (header, payload);
                }
            }
            Ok(None) => return Ok(()),
            Err(e) => {
                mock_log(&format!("Read error during handshake: {e}"));
                return Ok(());
            }
        }
    };

    let host_version = host_cnxn.0.arg0;
    let host_max_payload = host_cnxn.0.arg1 as usize;

    let negotiated_version = negotiate_version(host_version, A_VERSION);
    let negotiated_max_payload = negotiate_max_payload(host_max_payload, MAX_PAYLOAD);

    let mut features = Features::new();
    if options.enable_shell_v2 {
        features.push(Feature::Shell2);
    }

    let banner = Banner {
        system_type: "device".to_string(),
        serial: "".to_string(),
        properties: vec![
            ("ro.product.name".to_string(), "mock_device".to_string()),
            ("ro.product.model".to_string(), "mock_model".to_string()),
            ("ro.product.device".to_string(), "mock_device".to_string()),
        ],
        features,
        ends_with_semicolon: true,
    };
    let banner_bytes = banner.to_bytes();

    if write_packet_versioned(
        &mut writer,
        Command::Cnxn,
        negotiated_version,
        negotiated_max_payload as u32,
        &banner_bytes,
        negotiated_version,
    )
    .await
    .is_err()
    {
        mock_log("Failed to send CNXN reply");
        return Ok(());
    }

    let next_stream_id = AtomicU32::new(1);
    let mut streams: HashMap<u32, StreamState> = HashMap::new();

    // 2. Stream loop
    loop {
        let (header, payload) = match read_packet(&mut reader).await {
            Ok(Some(pkt)) => pkt,
            Ok(None) => break,
            Err(e) => {
                mock_log(&format!("Connection read error: {e}"));
                break;
            }
        };

        match header.command {
            Command::Open => {
                let host_local_id = header.arg0;
                let service_raw = String::from_utf8_lossy(&payload);
                let service = service_raw.trim_matches('\0');

                if let Some(rest) = service.strip_prefix("shell,") {
                    let mock_local_id = next_stream_id.fetch_add(1, Ordering::SeqCst);

                    let (args_str, cmd_str) = match rest.find(':') {
                        Some(idx) => (&rest[..idx], &rest[idx + 1..]),
                        None => (rest, ""),
                    };

                    let args: Vec<&str> = args_str.split(',').collect();
                    if args.contains(&"v2") {
                        let is_pty = args.contains(&"pty");
                        let term_val = args.iter().find_map(|a| a.strip_prefix("TERM="));

                        if write_packet_versioned(
                            &mut writer,
                            Command::Okay,
                            mock_local_id,
                            host_local_id,
                            &[],
                            negotiated_version,
                        )
                        .await
                        .is_err()
                        {
                            mock_log("Failed to write OKAY for OPEN");
                            break;
                        }

                        if cmd_str == "cat" {
                            streams.insert(
                                mock_local_id,
                                StreamState {
                                    mock_local_id,
                                    host_local_id,
                                    kind: StreamKind::ShellV2Cat {
                                        decoder: ShellDecoder::new(),
                                        is_pty,
                                    },
                                    write_in_flight: false,
                                    pending_writes: VecDeque::new(),
                                    close_pending: false,
                                },
                            );
                        } else {
                            let (output_pkt, exit_code) = if let Some(echo_text) =
                                parse_text_command(cmd_str, "echo")
                            {
                                (
                                    Some(ShellPacket::new(
                                        ShellPacketId::Stdout,
                                        echo_text.into_bytes(),
                                    )),
                                    0,
                                )
                            } else if let Some(stderr_text) = parse_text_command(cmd_str, "stderr")
                            {
                                (
                                    Some(ShellPacket::new(
                                        ShellPacketId::Stderr,
                                        stderr_text.into_bytes(),
                                    )),
                                    0,
                                )
                            } else if let Some(code) = parse_exit_command(cmd_str) {
                                (None, code)
                            } else if cmd_str == "env-term" {
                                let term_text = format!("{}\n", term_val.unwrap_or("unset"));
                                (
                                    Some(ShellPacket::new(
                                        ShellPacketId::Stdout,
                                        term_text.into_bytes(),
                                    )),
                                    0,
                                )
                            } else {
                                (
                                    Some(ShellPacket::new(
                                        ShellPacketId::Stderr,
                                        b"mock-adbd: unsupported command\n".to_vec(),
                                    )),
                                    127,
                                )
                            };

                            let mut pending_writes = VecDeque::new();
                            if let Some(pkt) = output_pkt {
                                pending_writes.push_back(pkt.encode_to_vec());
                            }
                            pending_writes
                                .push_back(ShellPacket::exit_code(exit_code).encode_to_vec());

                            streams.insert(
                                mock_local_id,
                                StreamState {
                                    mock_local_id,
                                    host_local_id,
                                    kind: StreamKind::ShellV2NonCat {
                                        decoder: ShellDecoder::new(),
                                        is_pty,
                                    },
                                    write_in_flight: false,
                                    pending_writes,
                                    close_pending: true,
                                },
                            );
                        }
                    } else {
                        // Refuse non-v2 shell, prefix with CLSE
                        let _ = write_packet_versioned(
                            &mut writer,
                            Command::Clse,
                            0,
                            host_local_id,
                            &[],
                            negotiated_version,
                        )
                        .await;
                    }
                } else if let Some(cmd) = service.strip_prefix("shell:") {
                    let mock_local_id = next_stream_id.fetch_add(1, Ordering::SeqCst);

                    if write_packet_versioned(
                        &mut writer,
                        Command::Okay,
                        mock_local_id,
                        host_local_id,
                        &[],
                        negotiated_version,
                    )
                    .await
                    .is_err()
                    {
                        mock_log("Failed to write OKAY for shell: OPEN");
                        break;
                    }

                    let output = parse_text_command(cmd, "echo");
                    let response_payload = match output {
                        Some(text) => text.into_bytes(),
                        None => b"mock-adbd: unsupported command\n".to_vec(),
                    };

                    let mut pending_writes = VecDeque::new();
                    pending_writes.push_back(response_payload);

                    streams.insert(
                        mock_local_id,
                        StreamState {
                            mock_local_id,
                            host_local_id,
                            kind: StreamKind::ShellV1,
                            write_in_flight: false,
                            pending_writes,
                            close_pending: true,
                        },
                    );
                } else {
                    // Refuse unknown service with CLSE(0, host_local_id)
                    let _ = write_packet_versioned(
                        &mut writer,
                        Command::Clse,
                        0,
                        host_local_id,
                        &[],
                        negotiated_version,
                    )
                    .await;
                }
            }
            Command::Wrte => {
                let mock_local_id = header.arg1;
                let host_local_id = header.arg0;

                if let Some(stream_state) = streams.get_mut(&mock_local_id) {
                    if write_packet_versioned(
                        &mut writer,
                        Command::Okay,
                        mock_local_id,
                        host_local_id,
                        &[],
                        negotiated_version,
                    )
                    .await
                    .is_err()
                    {
                        mock_log("Failed to write OKAY response to host WRTE");
                        break;
                    }

                    match &mut stream_state.kind {
                        StreamKind::ShellV1 => {}
                        StreamKind::ShellV2NonCat { decoder, is_pty } => {
                            decoder.feed(&payload);
                            while let Ok(Some(pkt)) = decoder.pop_packet() {
                                if pkt.id == ShellPacketId::WindowSizeChange && *is_pty {
                                    let winsz_bytes = format_winsz_payload(&pkt.data);
                                    if stream_state.pending_writes.len() > 1 {
                                        stream_state.pending_writes.insert(
                                            stream_state.pending_writes.len() - 1,
                                            winsz_bytes,
                                        );
                                    } else {
                                        stream_state.pending_writes.push_back(winsz_bytes);
                                    }
                                }
                            }
                        }
                        StreamKind::ShellV2Cat { decoder, is_pty } => {
                            decoder.feed(&payload);
                            while let Ok(Some(pkt)) = decoder.pop_packet() {
                                match pkt.id {
                                    ShellPacketId::Stdin => {
                                        let stdout_bytes =
                                            ShellPacket::new(ShellPacketId::Stdout, pkt.data)
                                                .encode_to_vec();
                                        stream_state.pending_writes.push_back(stdout_bytes);
                                    }
                                    ShellPacketId::CloseStdin => {
                                        let exit_bytes = ShellPacket::exit_code(0).encode_to_vec();
                                        stream_state.pending_writes.push_back(exit_bytes);
                                        stream_state.close_pending = true;
                                    }
                                    ShellPacketId::WindowSizeChange if *is_pty => {
                                        let winsz_bytes = format_winsz_payload(&pkt.data);
                                        stream_state.pending_writes.push_back(winsz_bytes);
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                } else {
                    mock_log(&format!(
                        "WRTE received for unknown stream id {mock_local_id}"
                    ));
                    let _ = write_packet_versioned(
                        &mut writer,
                        Command::Clse,
                        0,
                        host_local_id,
                        &[],
                        negotiated_version,
                    )
                    .await;
                }
            }
            Command::Okay => {
                let mock_local_id = header.arg1;
                if let Some(stream_state) = streams.get_mut(&mock_local_id) {
                    stream_state.write_in_flight = false;
                } else {
                    mock_log(&format!(
                        "OKAY received for unknown stream id {mock_local_id}"
                    ));
                }
            }
            Command::Clse => {
                let mock_local_id = header.arg1;
                streams.remove(&mock_local_id);
            }
            _ => {
                mock_log(&format!("Unexpected packet command: {:?}", header.command));
                let mock_local_id = header.arg1;
                let host_local_id = header.arg0;
                if streams.contains_key(&mock_local_id) {
                    streams.remove(&mock_local_id);
                    let _ = write_packet_versioned(
                        &mut writer,
                        Command::Clse,
                        mock_local_id,
                        host_local_id,
                        &[],
                        negotiated_version,
                    )
                    .await;
                } else {
                    let _ = write_packet_versioned(
                        &mut writer,
                        Command::Clse,
                        0,
                        host_local_id,
                        &[],
                        negotiated_version,
                    )
                    .await;
                }
            }
        }

        // Writer Pump: Flush pending writes for streams
        let mut streams_to_remove = Vec::new();
        for (mock_local_id, stream_state) in streams.iter_mut() {
            if !stream_state.write_in_flight {
                if let Some(front) = stream_state.pending_writes.pop_front() {
                    let (to_send, remainder) = if front.len() > negotiated_max_payload {
                        let (head, tail) = front.split_at(negotiated_max_payload);
                        (head.to_vec(), Some(tail.to_vec()))
                    } else {
                        (front, None)
                    };

                    if let Some(tail) = remainder {
                        stream_state.pending_writes.push_front(tail);
                    }

                    if write_packet_versioned(
                        &mut writer,
                        Command::Wrte,
                        stream_state.mock_local_id,
                        stream_state.host_local_id,
                        &to_send,
                        negotiated_version,
                    )
                    .await
                    .is_err()
                    {
                        mock_log("Failed to send WRTE packet");
                        return Ok(());
                    }

                    stream_state.write_in_flight = true;
                } else if stream_state.close_pending {
                    if write_packet_versioned(
                        &mut writer,
                        Command::Clse,
                        stream_state.mock_local_id,
                        stream_state.host_local_id,
                        &[],
                        negotiated_version,
                    )
                    .await
                    .is_err()
                    {
                        mock_log("Failed to send CLSE packet");
                        return Ok(());
                    }
                    streams_to_remove.push(*mock_local_id);
                }
            }
        }

        for id in streams_to_remove {
            streams.remove(&id);
        }
    }

    Ok(())
}
