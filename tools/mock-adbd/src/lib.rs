use adb_proto::{
    negotiate_max_payload, negotiate_version, sync::*, Banner, BannerEntry, Command, Feature,
    Features, Header, ShellDecoder, ShellPacket, ShellPacketId, A_VERSION, A_VERSION_SKIP_CHECKSUM,
    MAX_PAYLOAD,
};
use std::collections::{HashMap, VecDeque};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;

pub mod fs;
pub use fs::InMemoryFs;

fn mock_log(msg: &str) {
    if std::env::var_os("MOCK_ADBD_LOG").is_some() {
        eprintln!("[mock-adbd] {msg}");
    }
}

pub const ID_STAT_V2: u32 = u32::from_le_bytes(*b"STA2");
pub const ID_LSTAT_V2: u32 = u32::from_le_bytes(*b"LST2");
pub const ID_LIST_V2: u32 = u32::from_le_bytes(*b"LIS2");
pub const ID_DENT_V2: u32 = u32::from_le_bytes(*b"DNT2");
pub const ID_SEND_V2: u32 = u32::from_le_bytes(*b"SND2");
pub const ID_RECV_V2: u32 = u32::from_le_bytes(*b"RCV2");

#[derive(Debug, Clone)]
pub struct MockAdbdOptions {
    pub enable_shell_v2: bool,
    pub enable_stat_v2: bool,
    pub enable_ls_v2: bool,
    pub enable_sendrecv_v2: bool,
    pub fixed_push_mkdir: bool,
    pub fs: InMemoryFs,
}

impl Default for MockAdbdOptions {
    fn default() -> Self {
        Self {
            enable_shell_v2: true,
            enable_stat_v2: true,
            enable_ls_v2: true,
            enable_sendrecv_v2: true,
            fixed_push_mkdir: false,
            fs: InMemoryFs::default(),
        }
    }
}

pub struct MockAdbdServer {
    local_addr: SocketAddr,
    accept_handle: JoinHandle<()>,
    tasks: Arc<Mutex<Vec<JoinHandle<()>>>>,
    fs: InMemoryFs,
}

impl MockAdbdServer {
    pub async fn bind(addr: &str) -> std::io::Result<Self> {
        Self::bind_with_options(addr, MockAdbdOptions::default()).await
    }

    pub async fn bind_with_options(addr: &str, options: MockAdbdOptions) -> std::io::Result<Self> {
        let listener = TcpListener::bind(addr).await?;
        let local_addr = listener.local_addr()?;
        let tasks = Arc::new(Mutex::new(Vec::new()));
        let fs = options.fs.clone();

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
            fs,
        })
    }

    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    pub fn fs(&self) -> &InMemoryFs {
        &self.fs
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

    let trimmed = raw_text.trim();
    if trimmed.is_empty() {
        return Some("\n".to_string());
    }

    let mut parts = Vec::new();
    let mut current = String::new();
    let mut in_single_quote = false;
    let mut in_double_quote = false;

    let chars: Vec<char> = trimmed.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '\'' && !in_double_quote {
            // Check for escaped single quote '\'
            if i + 3 < chars.len() && chars[i..i + 4] == ['\'', '\\', '\'', '\''] {
                current.push('\'');
                i += 4;
                continue;
            }
            in_single_quote = !in_single_quote;
        } else if c == '"' && !in_single_quote {
            in_double_quote = !in_double_quote;
        } else if c == ' ' && !in_single_quote && !in_double_quote {
            if !current.is_empty() {
                parts.push(current.clone());
                current.clear();
            }
        } else {
            current.push(c);
        }
        i += 1;
    }
    if !current.is_empty() {
        parts.push(current);
    }

    Some(format!("{}\n", parts.join(" ")))
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
    .encode()
    .expect("shell packet encoding failed")
}

fn parse_mode(s: &str) -> Option<u32> {
    if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        u32::from_str_radix(hex, 16).ok()
    } else if s.len() > 1 && s.starts_with('0') {
        u32::from_str_radix(&s[1..], 8).ok()
    } else {
        s.parse::<u32>().ok()
    }
}

fn encode_sync_fail(msg: &str) -> Vec<u8> {
    let msg_b = msg.as_bytes();
    let mut buf = Vec::with_capacity(8 + msg_b.len());
    buf.extend_from_slice(&ID_FAIL.to_le_bytes());
    buf.extend_from_slice(&(msg_b.len() as u32).to_le_bytes());
    buf.extend_from_slice(msg_b);
    buf
}

#[derive(Debug)]
enum SyncState {
    Idle,
    ReceivingSend {
        path: String,
        mode: u32,
        data: Vec<u8>,
    },
}

#[derive(Debug)]
struct SyncStreamState {
    decoder_buf: VecDeque<u8>,
    state: SyncState,
}

enum StreamKind {
    ShellV1,
    ShellV1Cat,
    ShellV2NonCat { decoder: ShellDecoder, is_pty: bool },
    ShellV2Cat { decoder: ShellDecoder, is_pty: bool },
    Sync(SyncStreamState),
}

struct StreamState {
    mock_local_id: u32,
    host_local_id: u32,
    kind: StreamKind,
    write_in_flight: bool,
    pending_writes: VecDeque<Vec<u8>>,
    close_pending: bool,
}

fn process_sync_buffer(
    sync_state: &mut SyncStreamState,
    fs: &InMemoryFs,
    pending_writes: &mut VecDeque<Vec<u8>>,
    close_pending: &mut bool,
) {
    loop {
        match &mut sync_state.state {
            SyncState::Idle => {
                if sync_state.decoder_buf.len() < 8 {
                    break;
                }
                let mut req_bytes = [0u8; 8];
                for (i, b) in sync_state.decoder_buf.iter().take(8).enumerate() {
                    req_bytes[i] = *b;
                }
                let id =
                    u32::from_le_bytes([req_bytes[0], req_bytes[1], req_bytes[2], req_bytes[3]]);
                let path_len =
                    u32::from_le_bytes([req_bytes[4], req_bytes[5], req_bytes[6], req_bytes[7]])
                        as usize;

                if path_len > SYNC_PATH_MAX {
                    pending_writes.push_back(encode_sync_fail("path too long"));
                    *close_pending = true;
                    break;
                }

                if sync_state.decoder_buf.len() < 8 + path_len {
                    break;
                }

                if id == ID_RECV_V2 && sync_state.decoder_buf.len() < 8 + path_len + 8 {
                    break;
                }

                if id == ID_SEND_V2 && sync_state.decoder_buf.len() < 8 + path_len + 12 {
                    break;
                }

                sync_state.decoder_buf.drain(..8);
                let path_bytes: Vec<u8> = sync_state.decoder_buf.drain(..path_len).collect();
                let path = String::from_utf8_lossy(&path_bytes).to_string();

                mock_log(&format!(
                    "Sync request: id={:08x} ({:?}), path='{}'",
                    id,
                    String::from_utf8_lossy(&id.to_le_bytes()),
                    path
                ));

                match id {
                    ID_LSTAT_V1 => {
                        let node = fs.get(&path);
                        let mut resp = Vec::with_capacity(16);
                        resp.extend_from_slice(&ID_LSTAT_V1.to_le_bytes());
                        if let Some(n) = node {
                            resp.extend_from_slice(&n.mode().to_le_bytes());
                            resp.extend_from_slice(&(n.size() as u32).to_le_bytes());
                            resp.extend_from_slice(&n.mtime().to_le_bytes());
                        } else {
                            resp.extend_from_slice(&0u32.to_le_bytes());
                            resp.extend_from_slice(&0u32.to_le_bytes());
                            resp.extend_from_slice(&0u32.to_le_bytes());
                        }
                        pending_writes.push_back(resp);
                    }
                    ID_LSTAT_V2 | ID_STAT_V2 => {
                        let node = fs.get(&path);
                        let mut resp = Vec::with_capacity(72);
                        resp.extend_from_slice(&id.to_le_bytes());
                        if let Some(n) = node {
                            resp.extend_from_slice(&0u32.to_le_bytes()); // error = 0
                            resp.extend_from_slice(&0u64.to_le_bytes()); // dev
                            resp.extend_from_slice(&0u64.to_le_bytes()); // ino
                            resp.extend_from_slice(&n.mode().to_le_bytes());
                            resp.extend_from_slice(&1u32.to_le_bytes()); // nlink
                            resp.extend_from_slice(&0u32.to_le_bytes()); // uid
                            resp.extend_from_slice(&0u32.to_le_bytes()); // gid
                            resp.extend_from_slice(&n.size().to_le_bytes());
                            resp.extend_from_slice(&(n.mtime() as i64).to_le_bytes());
                            resp.extend_from_slice(&(n.mtime() as i64).to_le_bytes());
                            resp.extend_from_slice(&(n.mtime() as i64).to_le_bytes());
                        } else {
                            resp.extend_from_slice(&2u32.to_le_bytes()); // ENOENT (2)
                            resp.extend_from_slice(&[0u8; 64]);
                        }
                        pending_writes.push_back(resp);
                    }
                    ID_LIST_V1 => {
                        let mut resp = Vec::new();
                        if let Some(entries) = fs.list(&path) {
                            mock_log(&format!("List v1 entries count: {}", entries.len()));
                            for (name, node) in entries {
                                let name_b = name.as_bytes();
                                resp.extend_from_slice(&ID_DENT_V1.to_le_bytes());
                                resp.extend_from_slice(&node.mode().to_le_bytes());
                                resp.extend_from_slice(&(node.size() as u32).to_le_bytes());
                                resp.extend_from_slice(&node.mtime().to_le_bytes());
                                resp.extend_from_slice(&(name_b.len() as u32).to_le_bytes());
                                resp.extend_from_slice(name_b);
                            }
                        } else {
                            mock_log(&format!("List v1 failed: '{}' is not a directory", path));
                        }
                        resp.extend_from_slice(&ID_DONE.to_le_bytes());
                        resp.extend_from_slice(&[0u8; 16]);
                        pending_writes.push_back(resp);
                    }
                    ID_LIST_V2 => {
                        let mut resp = Vec::new();
                        if let Some(entries) = fs.list(&path) {
                            mock_log(&format!("List v2 entries count: {}", entries.len()));
                            for (name, node) in entries {
                                let name_b = name.as_bytes();
                                resp.extend_from_slice(&ID_DENT_V2.to_le_bytes());
                                resp.extend_from_slice(&0u32.to_le_bytes());
                                resp.extend_from_slice(&0u64.to_le_bytes());
                                resp.extend_from_slice(&0u64.to_le_bytes());
                                resp.extend_from_slice(&node.mode().to_le_bytes());
                                resp.extend_from_slice(&1u32.to_le_bytes());
                                resp.extend_from_slice(&0u32.to_le_bytes());
                                resp.extend_from_slice(&0u32.to_le_bytes());
                                resp.extend_from_slice(&node.size().to_le_bytes());
                                resp.extend_from_slice(&(node.mtime() as i64).to_le_bytes());
                                resp.extend_from_slice(&(node.mtime() as i64).to_le_bytes());
                                resp.extend_from_slice(&(node.mtime() as i64).to_le_bytes());
                                resp.extend_from_slice(&(name_b.len() as u32).to_le_bytes());
                                resp.extend_from_slice(name_b);
                            }
                        } else {
                            mock_log(&format!("List v2 failed: '{}' is not a directory", path));
                        }
                        resp.extend_from_slice(&ID_DONE.to_le_bytes());
                        resp.extend_from_slice(&[0u8; 72]);
                        pending_writes.push_back(resp);
                    }
                    ID_RECV_V1 | ID_RECV_V2 => {
                        if id == ID_RECV_V2 {
                            sync_state.decoder_buf.drain(..8);
                        }
                        let node = fs.get(&path);
                        match node {
                            Some(fs::Node::File(f)) => {
                                let mut resp = Vec::new();
                                for chunk in f.data.chunks(SYNC_DATA_MAX) {
                                    resp.extend_from_slice(&ID_DATA.to_le_bytes());
                                    resp.extend_from_slice(&(chunk.len() as u32).to_le_bytes());
                                    resp.extend_from_slice(chunk);
                                }
                                resp.extend_from_slice(&ID_DONE.to_le_bytes());
                                resp.extend_from_slice(&0u32.to_le_bytes());
                                pending_writes.push_back(resp);
                            }
                            Some(fs::Node::Directory { .. }) => {
                                let fail_msg = format!("'{}' is a directory", path);
                                pending_writes.push_back(encode_sync_fail(&fail_msg));
                            }
                            _ => {
                                let fail_msg = format!("remote object '{}' does not exist", path);
                                pending_writes.push_back(encode_sync_fail(&fail_msg));
                            }
                        }
                    }
                    ID_SEND_V1 => {
                        let (file_path, mode) = if let Some(idx) = path.rfind(',') {
                            let p = &path[..idx];
                            let m_str = &path[idx + 1..];
                            let mode = parse_mode(m_str).unwrap_or(0o100644);
                            (p.to_string(), mode)
                        } else {
                            (path, 0o100644)
                        };
                        sync_state.state = SyncState::ReceivingSend {
                            path: file_path,
                            mode,
                            data: Vec::new(),
                        };
                    }
                    ID_SEND_V2 => {
                        let mut setup_b = [0u8; 12];
                        for (i, b) in sync_state.decoder_buf.drain(..12).enumerate() {
                            setup_b[i] = b;
                        }
                        let mode =
                            u32::from_le_bytes([setup_b[4], setup_b[5], setup_b[6], setup_b[7]]);
                        sync_state.state = SyncState::ReceivingSend {
                            path,
                            mode,
                            data: Vec::new(),
                        };
                    }
                    ID_QUIT => {
                        *close_pending = true;
                        break;
                    }
                    _ => {
                        let fail_msg = format!("unknown command {:08x}", id);
                        pending_writes.push_back(encode_sync_fail(&fail_msg));
                        *close_pending = true;
                        break;
                    }
                }
            }
            SyncState::ReceivingSend { path, mode, data } => {
                if sync_state.decoder_buf.len() < 8 {
                    break;
                }
                let mut hdr_bytes = [0u8; 8];
                for (i, b) in sync_state.decoder_buf.iter().take(8).enumerate() {
                    hdr_bytes[i] = *b;
                }
                let id =
                    u32::from_le_bytes([hdr_bytes[0], hdr_bytes[1], hdr_bytes[2], hdr_bytes[3]]);
                let len_or_mtime =
                    u32::from_le_bytes([hdr_bytes[4], hdr_bytes[5], hdr_bytes[6], hdr_bytes[7]]);

                if id == ID_DATA {
                    let chunk_len = len_or_mtime as usize;
                    if sync_state.decoder_buf.len() < 8 + chunk_len {
                        break;
                    }
                    sync_state.decoder_buf.drain(..8);
                    let chunk: Vec<u8> = sync_state.decoder_buf.drain(..chunk_len).collect();
                    data.extend_from_slice(&chunk);
                } else if id == ID_DONE {
                    sync_state.decoder_buf.drain(..8);
                    let mtime = len_or_mtime;
                    let target_path = path.clone();
                    let file_mode = *mode;
                    let file_data = std::mem::take(data);

                    if fs.parent_exists(&target_path) {
                        fs.add_file(&target_path, file_data, file_mode, mtime);
                        let mut resp = Vec::with_capacity(8);
                        resp.extend_from_slice(&ID_OKAY.to_le_bytes());
                        resp.extend_from_slice(&0u32.to_le_bytes());
                        pending_writes.push_back(resp);
                    } else {
                        let fail_msg = format!(
                            "failed to create file '{}': No such file or directory",
                            target_path
                        );
                        pending_writes.push_back(encode_sync_fail(&fail_msg));
                    }
                    sync_state.state = SyncState::Idle;
                } else {
                    let fail_msg = format!("expected DATA or DONE, got {:08x}", id);
                    pending_writes.push_back(encode_sync_fail(&fail_msg));
                    *close_pending = true;
                    break;
                }
            }
        }
    }
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
    if options.enable_stat_v2 {
        features.push(Feature::Stat2);
    }
    if options.enable_ls_v2 {
        features.push(Feature::Ls2);
    }
    if options.enable_sendrecv_v2 {
        features.push(Feature::SendRecv2);
    }
    if options.fixed_push_mkdir {
        features.push(Feature::FixedPushMkdir);
    }

    let banner = Banner {
        system_type: "device".to_string(),
        serial: "".to_string(),
        entries: vec![
            BannerEntry::Property("ro.product.name".to_string(), "mock_device".to_string()),
            BannerEntry::Property("ro.product.model".to_string(), "mock_model".to_string()),
            BannerEntry::Property("ro.product.device".to_string(), "mock_device".to_string()),
            BannerEntry::Features(features),
        ],
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

                if service == "sync:" || service == "sync" {
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
                        mock_log("Failed to write OKAY for sync: OPEN");
                        break;
                    }

                    streams.insert(
                        mock_local_id,
                        StreamState {
                            mock_local_id,
                            host_local_id,
                            kind: StreamKind::Sync(SyncStreamState {
                                decoder_buf: VecDeque::new(),
                                state: SyncState::Idle,
                            }),
                            write_in_flight: false,
                            pending_writes: VecDeque::new(),
                            close_pending: false,
                        },
                    );
                } else if let Some(rest) = service.strip_prefix("shell,") {
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
                                    Some(0),
                                )
                            } else if let Some(stderr_text) = parse_text_command(cmd_str, "stderr")
                            {
                                (
                                    Some(ShellPacket::new(
                                        ShellPacketId::Stderr,
                                        stderr_text.into_bytes(),
                                    )),
                                    Some(0),
                                )
                            } else if let Some(code) = parse_exit_command(cmd_str) {
                                (None, Some(code))
                            } else if cmd_str == "env-term" {
                                let term_text = format!("{}\n", term_val.unwrap_or("unset"));
                                (
                                    Some(ShellPacket::new(
                                        ShellPacketId::Stdout,
                                        term_text.into_bytes(),
                                    )),
                                    Some(0),
                                )
                            } else if cmd_str == "no_exit" {
                                (
                                    Some(ShellPacket::new(
                                        ShellPacketId::Stdout,
                                        b"no exit packet stdout\n".to_vec(),
                                    )),
                                    None,
                                )
                            } else {
                                (
                                    Some(ShellPacket::new(
                                        ShellPacketId::Stderr,
                                        b"mock-adbd: unsupported command\n".to_vec(),
                                    )),
                                    Some(127),
                                )
                            };

                            let mut pending_writes = VecDeque::new();
                            if let Some(pkt) = output_pkt {
                                pending_writes
                                    .push_back(pkt.encode().expect("shell encode failed"));
                            }
                            if let Some(code) = exit_code {
                                pending_writes.push_back(
                                    ShellPacket::exit_code(code)
                                        .encode()
                                        .expect("shell encode failed"),
                                );
                            }

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
                } else if let Some(cmd) = service
                    .strip_prefix("shell:")
                    .or_else(|| service.strip_prefix("exec:"))
                {
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
                        mock_log("Failed to write OKAY for shell/exec: OPEN");
                        break;
                    }

                    if cmd == "cat" {
                        streams.insert(
                            mock_local_id,
                            StreamState {
                                mock_local_id,
                                host_local_id,
                                kind: StreamKind::ShellV1Cat,
                                write_in_flight: false,
                                pending_writes: VecDeque::new(),
                                close_pending: false,
                            },
                        );
                    } else {
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
                    }
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
                        StreamKind::Sync(sync_state) => {
                            sync_state.decoder_buf.extend(&payload);
                            process_sync_buffer(
                                sync_state,
                                &options.fs,
                                &mut stream_state.pending_writes,
                                &mut stream_state.close_pending,
                            );
                        }
                        StreamKind::ShellV1 => {}
                        StreamKind::ShellV1Cat => {
                            stream_state.pending_writes.push_back(payload.clone());
                        }
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
                                                .encode()
                                                .expect("shell encode failed");
                                        stream_state.pending_writes.push_back(stdout_bytes);
                                    }
                                    ShellPacketId::CloseStdin => {
                                        let exit_bytes = ShellPacket::exit_code(0)
                                            .encode()
                                            .expect("shell encode failed");
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
                let host_local_id = header.arg0;
                if let Some(stream_state) = streams.get_mut(&mock_local_id) {
                    if stream_state.host_local_id == host_local_id {
                        stream_state.write_in_flight = false;
                    } else {
                        let expected = stream_state.host_local_id;
                        mock_log(&format!(
                            "OKAY host_local_id mismatch for mock_id {mock_local_id}: expected {expected}, got {host_local_id}"
                        ));
                    }
                } else {
                    mock_log(&format!(
                        "OKAY received for unknown stream id {mock_local_id}"
                    ));
                }
            }
            Command::Clse => {
                let mock_local_id = header.arg1;
                let host_local_id = header.arg0;

                if mock_local_id != 0 {
                    if let Some(stream_state) = streams.get(&mock_local_id) {
                        if stream_state.host_local_id == host_local_id {
                            streams.remove(&mock_local_id);
                        } else {
                            let expected = stream_state.host_local_id;
                            mock_log(&format!(
                                "CLSE host_local_id mismatch for mock_id {mock_local_id}: expected {expected}, got {host_local_id}"
                            ));
                        }
                    }
                } else {
                    let matching_id = streams.iter().find_map(|(&id, s)| {
                        if s.host_local_id == host_local_id {
                            Some(id)
                        } else {
                            None
                        }
                    });
                    if let Some(id) = matching_id {
                        streams.remove(&id);
                    }
                }
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
