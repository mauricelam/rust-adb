use adb_proto::{
    negotiate_max_payload, negotiate_version, Banner, Command, Feature, Features, Header,
    ShellDecoder, ShellPacket, ShellPacketId, A_VERSION, A_VERSION_SKIP_CHECKSUM, ID_DATA,
    ID_DENT_V1, ID_DENT_V2, ID_DONE, ID_FAIL, ID_LIST_V1, ID_LIST_V2, ID_LSTAT_V1, ID_LSTAT_V2,
    ID_OKAY, ID_QUIT, ID_RECV_V1, ID_RECV_V2, ID_SEND_V1, ID_SEND_V2, ID_STAT_V2, MAX_PAYLOAD,
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

fn hex_prefix(data: &[u8]) -> String {
    let slice = &data[..32.min(data.len())];
    let mut s = String::with_capacity(slice.len() * 2);
    for &b in slice {
        use std::fmt::Write;
        let _ = write!(s, "{:02x}", b);
    }
    s
}

pub fn sync_id_name(id: u32) -> &'static str {
    if id == ID_LSTAT_V1 {
        "STAT"
    } else if id == ID_STAT_V2 {
        "STT2"
    } else if id == ID_LSTAT_V2 {
        "LST2"
    } else if id == ID_LIST_V1 {
        "LIST"
    } else if id == ID_LIST_V2 {
        "LIS2"
    } else if id == ID_DENT_V1 {
        "DNT1"
    } else if id == ID_DENT_V2 {
        "DNT2"
    } else if id == ID_RECV_V1 {
        "RCV1"
    } else if id == ID_RECV_V2 {
        "RCV2"
    } else if id == ID_SEND_V1 {
        "SND1"
    } else if id == ID_SEND_V2 {
        "SND2"
    } else if id == ID_OKAY {
        "OKAY"
    } else if id == ID_FAIL {
        "FAIL"
    } else if id == ID_DONE {
        "DONE"
    } else if id == ID_QUIT {
        "QUIT"
    } else if id == ID_DATA {
        "DATA"
    } else {
        "UNKN"
    }
}

#[derive(Debug, Clone)]
pub enum EventKind {
    Packet {
        direction: &'static str,
        command: Command,
        arg0: u32,
        arg1: u32,
        data_length: u32,
        payload_hex_prefix: String,
    },
    SyncRequest {
        stream_id: u32,
        id_hex: String,
        id_name: String,
        path: String,
    },
    SyncQueuedReply {
        stream_id: u32,
        id_hex: String,
        id_name: String,
        length: usize,
    },
}

#[derive(Debug, Clone)]
pub struct EventRecord {
    pub timestamp_ms: u128,
    pub kind: EventKind,
}

impl std::fmt::Display for EventRecord {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let sec = self.timestamp_ms / 1000;
        let ms = self.timestamp_ms % 1000;
        match &self.kind {
            EventKind::Packet {
                direction,
                command,
                arg0,
                arg1,
                data_length,
                payload_hex_prefix,
            } => {
                write!(
                    f,
                    "[+{sec}.{ms:03}s] [{direction}] {command:?} arg0={arg0} arg1={arg1} len={data_length} hex={payload_hex_prefix}"
                )
            }
            EventKind::SyncRequest {
                stream_id,
                id_hex,
                id_name,
                path,
            } => {
                write!(
                    f,
                    "[+{sec}.{ms:03}s] [SyncReq stream={stream_id}] {id_name} ({id_hex}) path=\"{path}\""
                )
            }
            EventKind::SyncQueuedReply {
                stream_id,
                id_hex,
                id_name,
                length,
            } => {
                write!(
                    f,
                    "[+{sec}.{ms:03}s] [SyncReply stream={stream_id}] {id_name} ({id_hex}) len={length}"
                )
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct StreamSnapshot {
    pub mock_local_id: u32,
    pub host_local_id: u32,
    pub kind_name: String,
    pub write_in_flight: bool,
    pub pending_writes_lengths: Vec<usize>,
    pub close_pending: bool,
    pub sync_buffer_len: usize,
}

#[derive(Debug)]
pub struct ServerDiagnostics {
    pub start_time: std::time::Instant,
    pub events: VecDeque<EventRecord>,
    pub streams: HashMap<u32, StreamSnapshot>,
    pub last_packet_time: std::time::Instant,
}

impl ServerDiagnostics {
    pub fn new() -> Self {
        let now = std::time::Instant::now();
        Self {
            start_time: now,
            events: VecDeque::with_capacity(300),
            streams: HashMap::new(),
            last_packet_time: now,
        }
    }

    pub fn record_event(&mut self, kind: EventKind) {
        let elapsed_ms = self.start_time.elapsed().as_millis();
        let record = EventRecord {
            timestamp_ms: elapsed_ms,
            kind,
        };
        if self.events.len() >= 300 {
            self.events.pop_front();
        }
        if std::env::var_os("MOCK_ADBD_LOG").is_some() {
            eprintln!("[mock-adbd] {record}");
        }
        self.events.push_back(record);
    }

    pub fn format_debug_dump(&self) -> String {
        let mut out = String::new();
        out.push_str("=== MOCK ADBD DEBUG DUMP ===\n");
        out.push_str(&format!("--- LAST {} EVENTS ---\n", self.events.len()));
        for ev in &self.events {
            out.push_str(&format!("{ev}\n"));
        }
        out.push_str("--- STREAMS SNAPSHOT ---\n");
        if self.streams.is_empty() {
            out.push_str("(no active streams)\n");
        } else {
            let mut sorted_keys: Vec<_> = self.streams.keys().cloned().collect();
            sorted_keys.sort();
            for id in sorted_keys {
                if let Some(s) = self.streams.get(&id) {
                    out.push_str(&format!(
                        "Stream mock_id={} host_id={} kind={} write_in_flight={} pending_writes_lengths={:?} close_pending={} sync_buffer_len={}\n",
                        s.mock_local_id,
                        s.host_local_id,
                        s.kind_name,
                        s.write_in_flight,
                        s.pending_writes_lengths,
                        s.close_pending,
                        s.sync_buffer_len
                    ));
                }
            }
        }
        out.push_str("=== END DEBUG DUMP ===\n");
        out
    }
}

impl Default for ServerDiagnostics {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InMemoryNodeKind {
    File { contents: Vec<u8> },
    Directory,
    Symlink { target: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InMemoryNode {
    pub mode: u32,
    pub mtime: u64,
    pub atime: u64,
    pub ctime: u64,
    pub dev: u64,
    pub ino: u64,
    pub nlink: u64,
    pub uid: u32,
    pub gid: u32,
    pub kind: InMemoryNodeKind,
}

impl InMemoryNode {
    pub fn size(&self) -> u64 {
        match &self.kind {
            InMemoryNodeKind::File { contents } => contents.len() as u64,
            InMemoryNodeKind::Symlink { target } => target.len() as u64,
            InMemoryNodeKind::Directory => 0,
        }
    }
}

pub fn normalize_path(path: &str) -> String {
    let path = path.replace('\\', "/");
    let mut components = Vec::new();
    for part in path.split('/') {
        if part.is_empty() || part == "." {
            continue;
        } else if part == ".." {
            components.pop();
        } else {
            components.push(part);
        }
    }
    if components.is_empty() {
        "/".to_string()
    } else {
        format!("/{}", components.join("/"))
    }
}

pub fn parent_and_name(path: &str) -> Option<(&str, &str)> {
    if path == "/" {
        return None;
    }
    let idx = path.rfind('/')?;
    let parent = if idx == 0 { "/" } else { &path[..idx] };
    let name = &path[idx + 1..];
    Some((parent, name))
}

#[derive(Debug, Clone)]
pub struct InMemoryFs {
    inner: Arc<Mutex<InMemoryFsInner>>,
}

#[derive(Debug)]
struct InMemoryFsInner {
    nodes: HashMap<String, InMemoryNode>,
    next_ino: u64,
}

impl InMemoryFs {
    pub fn new() -> Self {
        let mut nodes = HashMap::new();
        nodes.insert(
            "/".to_string(),
            InMemoryNode {
                mode: 0o040755,
                mtime: 0,
                atime: 0,
                ctime: 0,
                dev: 1,
                ino: 1,
                nlink: 2,
                uid: 0,
                gid: 0,
                kind: InMemoryNodeKind::Directory,
            },
        );
        Self {
            inner: Arc::new(Mutex::new(InMemoryFsInner { nodes, next_ino: 2 })),
        }
    }

    fn ensure_parents(&self, inner: &mut InMemoryFsInner, norm_path: &str) {
        if let Some((parent, _)) = parent_and_name(norm_path) {
            if !inner.nodes.contains_key(parent) {
                let parent_str = parent.to_string();
                let ino = inner.next_ino;
                inner.next_ino += 1;
                inner.nodes.insert(
                    parent_str.clone(),
                    InMemoryNode {
                        mode: 0o040755,
                        mtime: 0,
                        atime: 0,
                        ctime: 0,
                        dev: 1,
                        ino,
                        nlink: 2,
                        uid: 0,
                        gid: 0,
                        kind: InMemoryNodeKind::Directory,
                    },
                );
                self.ensure_parents(inner, &parent_str);
            }
        }
    }

    pub fn add_file(&self, path: &str, contents: Vec<u8>, mode: u32, mtime: u64) {
        let norm = normalize_path(path);
        let mode = if mode & 0o170000 == 0 {
            mode | 0o100000
        } else {
            mode
        };
        let mut inner = self.inner.lock().unwrap();
        self.ensure_parents(&mut inner, &norm);
        let ino = inner.next_ino;
        inner.next_ino += 1;
        inner.nodes.insert(
            norm,
            InMemoryNode {
                mode,
                mtime,
                atime: mtime,
                ctime: mtime,
                dev: 1,
                ino,
                nlink: 1,
                uid: 0,
                gid: 0,
                kind: InMemoryNodeKind::File { contents },
            },
        );
    }

    pub fn add_dir(&self, path: &str, mode: u32, mtime: u64) {
        let norm = normalize_path(path);
        let mode = if mode & 0o170000 == 0 {
            mode | 0o040000
        } else {
            mode
        };
        let mut inner = self.inner.lock().unwrap();
        self.ensure_parents(&mut inner, &norm);
        let ino = inner.next_ino;
        inner.next_ino += 1;
        inner.nodes.insert(
            norm,
            InMemoryNode {
                mode,
                mtime,
                atime: mtime,
                ctime: mtime,
                dev: 1,
                ino,
                nlink: 2,
                uid: 0,
                gid: 0,
                kind: InMemoryNodeKind::Directory,
            },
        );
    }

    pub fn add_symlink(&self, path: &str, target: &str, mode: u32, mtime: u64) {
        let norm = normalize_path(path);
        let mode = if mode & 0o170000 == 0 {
            mode | 0o120000
        } else {
            mode
        };
        let mut inner = self.inner.lock().unwrap();
        self.ensure_parents(&mut inner, &norm);
        let ino = inner.next_ino;
        inner.next_ino += 1;
        inner.nodes.insert(
            norm,
            InMemoryNode {
                mode,
                mtime,
                atime: mtime,
                ctime: mtime,
                dev: 1,
                ino,
                nlink: 1,
                uid: 0,
                gid: 0,
                kind: InMemoryNodeKind::Symlink {
                    target: target.to_string(),
                },
            },
        );
    }

    pub fn get_file(&self, path: &str) -> Option<Vec<u8>> {
        let norm = normalize_path(path);
        let inner = self.inner.lock().unwrap();
        let node = inner.nodes.get(&norm)?;
        match &node.kind {
            InMemoryNodeKind::File { contents } => Some(contents.clone()),
            _ => None,
        }
    }

    pub fn get_node(&self, path: &str) -> Option<InMemoryNode> {
        let norm = normalize_path(path);
        let inner = self.inner.lock().unwrap();
        inner.nodes.get(&norm).cloned()
    }

    pub fn list_dir(&self, path: &str) -> Vec<(String, InMemoryNode)> {
        let norm = normalize_path(path);
        let inner = self.inner.lock().unwrap();
        let mut result = Vec::new();
        for (key, node) in inner.nodes.iter() {
            if let Some((parent, name)) = parent_and_name(key) {
                if parent == norm {
                    result.push((name.to_string(), node.clone()));
                }
            }
        }
        result.sort_by(|a, b| a.0.cmp(&b.0));
        result
    }

    pub fn remove(&self, path: &str) -> bool {
        let norm = normalize_path(path);
        let mut inner = self.inner.lock().unwrap();
        inner.nodes.remove(&norm).is_some()
    }
}

impl Default for InMemoryFs {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone)]
pub struct MockAdbdOptions {
    pub enable_shell_v2: bool,
    pub enable_stat_v2: bool,
    pub enable_ls_v2: bool,
    pub enable_sendrecv_v2: bool,
    pub enable_fixed_push_mkdir: bool,
    pub fs: InMemoryFs,
}

impl Default for MockAdbdOptions {
    fn default() -> Self {
        Self {
            enable_shell_v2: true,
            enable_stat_v2: true,
            enable_ls_v2: true,
            enable_sendrecv_v2: true,
            enable_fixed_push_mkdir: false,
            fs: InMemoryFs::new(),
        }
    }
}

pub struct MockAdbdServer {
    local_addr: SocketAddr,
    options: MockAdbdOptions,
    diagnostics: Arc<Mutex<ServerDiagnostics>>,
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
        let diagnostics = Arc::new(Mutex::new(ServerDiagnostics::new()));

        let tasks_clone = tasks.clone();
        let options_clone = options.clone();
        let diagnostics_clone = diagnostics.clone();
        let accept_handle = tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let options = options_clone.clone();
                let diagnostics = diagnostics_clone.clone();
                let handle = tokio::spawn(async move {
                    if let Err(e) =
                        handle_connection_with_diagnostics(stream, options, diagnostics).await
                    {
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
            options,
            diagnostics,
            accept_handle,
            tasks,
        })
    }

    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    pub fn fs(&self) -> InMemoryFs {
        self.options.fs.clone()
    }

    pub fn debug_dump(&self) -> String {
        self.diagnostics.lock().unwrap().format_debug_dump()
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

async fn write_packet_recorded<W: AsyncWrite + Unpin>(
    writer: &mut W,
    command: Command,
    arg0: u32,
    arg1: u32,
    payload: &[u8],
    version: u32,
    diagnostics: &Arc<Mutex<ServerDiagnostics>>,
) -> std::io::Result<()> {
    diagnostics.lock().unwrap().record_event(EventKind::Packet {
        direction: "Send",
        command,
        arg0,
        arg1,
        data_length: payload.len() as u32,
        payload_hex_prefix: hex_prefix(payload),
    });
    write_packet_versioned(writer, command, arg0, arg1, payload, version).await
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
    Sync { buffer: Vec<u8> },
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

fn sync_stream_snapshots(diag: &mut ServerDiagnostics, streams: &HashMap<u32, StreamState>) {
    diag.streams.clear();
    for (&id, s) in streams {
        diag.streams.insert(
            id,
            StreamSnapshot {
                mock_local_id: s.mock_local_id,
                host_local_id: s.host_local_id,
                kind_name: match &s.kind {
                    StreamKind::ShellV1 => "ShellV1".to_string(),
                    StreamKind::ShellV2NonCat { .. } => "ShellV2NonCat".to_string(),
                    StreamKind::ShellV2Cat { .. } => "ShellV2Cat".to_string(),
                    StreamKind::Sync { .. } => "Sync".to_string(),
                },
                write_in_flight: s.write_in_flight,
                pending_writes_lengths: s.pending_writes.iter().map(|v| v.len()).collect(),
                close_pending: s.close_pending,
                sync_buffer_len: match &s.kind {
                    StreamKind::Sync { buffer } => buffer.len(),
                    _ => 0,
                },
            },
        );
    }
}

pub async fn handle_connection_with_options(
    stream: TcpStream,
    options: MockAdbdOptions,
) -> std::io::Result<()> {
    let diagnostics = Arc::new(Mutex::new(ServerDiagnostics::new()));
    handle_connection_with_diagnostics(stream, options, diagnostics).await
}

pub async fn handle_connection_with_diagnostics(
    stream: TcpStream,
    options: MockAdbdOptions,
    diagnostics: Arc<Mutex<ServerDiagnostics>>,
) -> std::io::Result<()> {
    let diag_for_watchdog = diagnostics.clone();
    let watchdog_task = tokio::spawn(async move {
        let mut stall_reported = false;
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            let diag = diag_for_watchdog.lock().unwrap();

            let has_pending_work = diag
                .streams
                .values()
                .any(|s| s.write_in_flight || !s.pending_writes_lengths.is_empty());

            let elapsed = diag.last_packet_time.elapsed();

            if has_pending_work && elapsed >= std::time::Duration::from_secs(10) {
                if !stall_reported {
                    stall_reported = true;
                    let dump = diag.format_debug_dump();
                    eprintln!("STALL\n{dump}");
                }
            } else if elapsed < std::time::Duration::from_secs(10) {
                stall_reported = false;
            }
        }
    });

    let res = handle_connection_inner(stream, options, &diagnostics).await;
    watchdog_task.abort();
    res
}

async fn handle_connection_inner(
    stream: TcpStream,
    options: MockAdbdOptions,
    diagnostics: &Arc<Mutex<ServerDiagnostics>>,
) -> std::io::Result<()> {
    let (mut reader, mut writer) = stream.into_split();

    // 1. Handshake: Read host CNXN packet
    let host_cnxn = loop {
        match read_packet(&mut reader).await {
            Ok(Some((header, payload))) => {
                {
                    let mut diag = diagnostics.lock().unwrap();
                    diag.last_packet_time = std::time::Instant::now();
                    diag.record_event(EventKind::Packet {
                        direction: "Recv",
                        command: header.command,
                        arg0: header.arg0,
                        arg1: header.arg1,
                        data_length: header.data_length,
                        payload_hex_prefix: hex_prefix(&payload),
                    });
                }
                if header.command == Command::Cnxn {
                    break (header, payload);
                } else {
                    let _ = write_packet_recorded(
                        &mut writer,
                        Command::Clse,
                        0,
                        header.arg0,
                        &[],
                        A_VERSION,
                        diagnostics,
                    )
                    .await;
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
    if options.enable_fixed_push_mkdir {
        features.push(Feature::FixedPushMkdir);
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

    if write_packet_recorded(
        &mut writer,
        Command::Cnxn,
        negotiated_version,
        negotiated_max_payload as u32,
        &banner_bytes,
        negotiated_version,
        diagnostics,
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
            Ok(Some((hdr, payload))) => {
                {
                    let mut diag = diagnostics.lock().unwrap();
                    diag.last_packet_time = std::time::Instant::now();
                    diag.record_event(EventKind::Packet {
                        direction: "Recv",
                        command: hdr.command,
                        arg0: hdr.arg0,
                        arg1: hdr.arg1,
                        data_length: hdr.data_length,
                        payload_hex_prefix: hex_prefix(&payload),
                    });
                }
                (hdr, payload)
            }
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

                if service == "sync" || service.starts_with("sync:") {
                    let mock_local_id = next_stream_id.fetch_add(1, Ordering::SeqCst);
                    if write_packet_recorded(
                        &mut writer,
                        Command::Okay,
                        mock_local_id,
                        host_local_id,
                        &[],
                        negotiated_version,
                        diagnostics,
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
                            kind: StreamKind::Sync { buffer: Vec::new() },
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

                        if write_packet_recorded(
                            &mut writer,
                            Command::Okay,
                            mock_local_id,
                            host_local_id,
                            &[],
                            negotiated_version,
                            diagnostics,
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
                            let (output_pkt, exit_code) = if cmd_str.starts_with("mkdir") {
                                for arg in cmd_str.split_whitespace().skip(1) {
                                    if !arg.starts_with('-') {
                                        let clean_path = arg.trim_matches('\'').trim_matches('"');
                                        options.fs.add_dir(clean_path, 0o040755, 0);
                                    }
                                }
                                (None, 0)
                            } else if let Some(echo_text) = parse_text_command(cmd_str, "echo") {
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
                        let _ = write_packet_recorded(
                            &mut writer,
                            Command::Clse,
                            0,
                            host_local_id,
                            &[],
                            negotiated_version,
                            diagnostics,
                        )
                        .await;
                    }
                } else if let Some(cmd) = service.strip_prefix("shell:") {
                    let mock_local_id = next_stream_id.fetch_add(1, Ordering::SeqCst);

                    if write_packet_recorded(
                        &mut writer,
                        Command::Okay,
                        mock_local_id,
                        host_local_id,
                        &[],
                        negotiated_version,
                        diagnostics,
                    )
                    .await
                    .is_err()
                    {
                        mock_log("Failed to write OKAY for shell: OPEN");
                        break;
                    }

                    let response_payload = if cmd.starts_with("mkdir") {
                        for arg in cmd.split_whitespace().skip(1) {
                            if !arg.starts_with('-') {
                                let clean_path = arg.trim_matches('\'').trim_matches('"');
                                options.fs.add_dir(clean_path, 0o040755, 0);
                            }
                        }
                        Vec::new()
                    } else if let Some(text) = parse_text_command(cmd, "echo") {
                        text.into_bytes()
                    } else {
                        b"mock-adbd: unsupported command\n".to_vec()
                    };

                    let mut pending_writes = VecDeque::new();
                    if !response_payload.is_empty() {
                        pending_writes.push_back(response_payload);
                    }

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
                    let _ = write_packet_recorded(
                        &mut writer,
                        Command::Clse,
                        0,
                        host_local_id,
                        &[],
                        negotiated_version,
                        diagnostics,
                    )
                    .await;
                }
            }
            Command::Wrte => {
                let mock_local_id = header.arg1;
                let host_local_id = header.arg0;

                if let Some(stream_state) = streams.get_mut(&mock_local_id) {
                    if write_packet_recorded(
                        &mut writer,
                        Command::Okay,
                        mock_local_id,
                        host_local_id,
                        &[],
                        negotiated_version,
                        diagnostics,
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
                        StreamKind::Sync { buffer } => {
                            buffer.extend_from_slice(&payload);
                            let mut buf = std::mem::take(buffer);
                            process_sync_requests(&mut buf, stream_state, &options.fs, diagnostics);
                            if let StreamKind::Sync { buffer: b } = &mut stream_state.kind {
                                *b = buf;
                            }
                        }
                    }
                } else {
                    mock_log(&format!(
                        "WRTE received for unknown stream id {mock_local_id}"
                    ));
                    let _ = write_packet_recorded(
                        &mut writer,
                        Command::Clse,
                        0,
                        host_local_id,
                        &[],
                        negotiated_version,
                        diagnostics,
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
                    let _ = write_packet_recorded(
                        &mut writer,
                        Command::Clse,
                        mock_local_id,
                        host_local_id,
                        &[],
                        negotiated_version,
                        diagnostics,
                    )
                    .await;
                } else {
                    let _ = write_packet_recorded(
                        &mut writer,
                        Command::Clse,
                        0,
                        host_local_id,
                        &[],
                        negotiated_version,
                        diagnostics,
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

                    if write_packet_recorded(
                        &mut writer,
                        Command::Wrte,
                        stream_state.mock_local_id,
                        stream_state.host_local_id,
                        &to_send,
                        negotiated_version,
                        diagnostics,
                    )
                    .await
                    .is_err()
                    {
                        mock_log("Failed to send WRTE packet");
                        return Ok(());
                    }

                    stream_state.write_in_flight = true;
                } else if stream_state.close_pending {
                    if write_packet_recorded(
                        &mut writer,
                        Command::Clse,
                        stream_state.mock_local_id,
                        stream_state.host_local_id,
                        &[],
                        negotiated_version,
                        diagnostics,
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

        {
            let mut diag = diagnostics.lock().unwrap();
            sync_stream_snapshots(&mut diag, &streams);
        }
    }

    Ok(())
}

fn encode_sync_okay() -> Vec<u8> {
    let mut buf = Vec::with_capacity(8);
    buf.extend_from_slice(&ID_OKAY.to_le_bytes());
    buf.extend_from_slice(&0u32.to_le_bytes());
    buf
}

fn encode_sync_fail(msg: &str) -> Vec<u8> {
    let msg_bytes = msg.as_bytes();
    let mut buf = Vec::with_capacity(8 + msg_bytes.len());
    buf.extend_from_slice(&ID_FAIL.to_le_bytes());
    buf.extend_from_slice(&(msg_bytes.len() as u32).to_le_bytes());
    buf.extend_from_slice(msg_bytes);
    buf
}

fn queue_sync_reply(
    stream_state: &mut StreamState,
    diagnostics: &Arc<Mutex<ServerDiagnostics>>,
    resp: Vec<u8>,
) {
    if resp.len() >= 4 {
        let reply_id = u32::from_le_bytes(resp[0..4].try_into().unwrap());
        let reply_hex = format!("0x{reply_id:08x}");
        let reply_name = sync_id_name(reply_id);
        diagnostics
            .lock()
            .unwrap()
            .record_event(EventKind::SyncQueuedReply {
                stream_id: stream_state.mock_local_id,
                id_hex: reply_hex,
                id_name: reply_name.to_string(),
                length: resp.len(),
            });
    }
    stream_state.pending_writes.push_back(resp);
}

fn parse_send_v1_spec(spec: &str) -> Option<(String, u32)> {
    let (path, mode_str) = spec.rsplit_once(',')?;
    let mode = if let Some(hex) = mode_str
        .strip_prefix("0x")
        .or_else(|| mode_str.strip_prefix("0X"))
    {
        u32::from_str_radix(hex, 16).ok()?
    } else if mode_str.len() > 1 && mode_str.starts_with('0') {
        u32::from_str_radix(&mode_str[1..], 8).ok()?
    } else {
        mode_str.parse::<u32>().ok()?
    };
    Some((path.to_string(), mode))
}

fn process_sync_requests(
    buffer: &mut Vec<u8>,
    stream_state: &mut StreamState,
    fs: &InMemoryFs,
    diagnostics: &Arc<Mutex<ServerDiagnostics>>,
) {
    loop {
        if buffer.len() < 8 {
            break;
        }

        let id = u32::from_le_bytes(buffer[0..4].try_into().unwrap());
        let path_length = u32::from_le_bytes(buffer[4..8].try_into().unwrap()) as usize;

        if path_length > 1024 {
            stream_state
                .pending_writes
                .push_back(encode_sync_fail("path too long"));
            stream_state.close_pending = true;
            buffer.clear();
            break;
        }

        if buffer.len() < 8 + path_length {
            break;
        }

        let path_bytes = &buffer[8..8 + path_length];
        let path = match String::from_utf8(path_bytes.to_vec()) {
            Ok(s) => s,
            Err(_) => {
                stream_state
                    .pending_writes
                    .push_back(encode_sync_fail("invalid utf-8 path"));
                stream_state.close_pending = true;
                buffer.clear();
                break;
            }
        };

        diagnostics
            .lock()
            .unwrap()
            .record_event(EventKind::SyncRequest {
                stream_id: stream_state.mock_local_id,
                id_hex: format!("0x{id:08x}"),
                id_name: sync_id_name(id).to_string(),
                path: path.clone(),
            });

        match id {
            ID_LSTAT_V1 => {
                buffer.drain(..8 + path_length);
                let node_opt = fs.get_node(&path);
                let mut resp = Vec::with_capacity(16);
                resp.extend_from_slice(&ID_LSTAT_V1.to_le_bytes());
                if let Some(node) = node_opt {
                    resp.extend_from_slice(&node.mode.to_le_bytes());
                    resp.extend_from_slice(&(node.size() as u32).to_le_bytes());
                    resp.extend_from_slice(&(node.mtime as u32).to_le_bytes());
                } else {
                    resp.extend_from_slice(&0u32.to_le_bytes());
                    resp.extend_from_slice(&0u32.to_le_bytes());
                    resp.extend_from_slice(&0u32.to_le_bytes());
                }
                stream_state.pending_writes.push_back(resp);
            }
            id if id == ID_STAT_V2 || id == ID_LSTAT_V2 => {
                buffer.drain(..8 + path_length);
                let node_opt = fs.get_node(&path);
                let mut resp = Vec::with_capacity(72);
                resp.extend_from_slice(&id.to_le_bytes());
                if let Some(node) = node_opt {
                    resp.extend_from_slice(&0u32.to_le_bytes()); // error = 0
                    resp.extend_from_slice(&node.dev.to_le_bytes());
                    resp.extend_from_slice(&node.ino.to_le_bytes());
                    resp.extend_from_slice(&node.mode.to_le_bytes());
                    resp.extend_from_slice(&(node.nlink as u32).to_le_bytes());
                    resp.extend_from_slice(&node.uid.to_le_bytes());
                    resp.extend_from_slice(&node.gid.to_le_bytes());
                    resp.extend_from_slice(&node.size().to_le_bytes());
                    resp.extend_from_slice(&(node.atime as i64).to_le_bytes());
                    resp.extend_from_slice(&(node.mtime as i64).to_le_bytes());
                    resp.extend_from_slice(&(node.ctime as i64).to_le_bytes());
                } else {
                    resp.extend_from_slice(&2u32.to_le_bytes()); // wire ENOENT = 2
                    resp.extend_from_slice(&[0u8; 64]);
                }
                stream_state.pending_writes.push_back(resp);
            }
            ID_LIST_V1 => {
                buffer.drain(..8 + path_length);
                let entries = fs.list_dir(&path);
                for (name, node) in entries {
                    let name_bytes = name.as_bytes();
                    let mut resp = Vec::with_capacity(20 + name_bytes.len());
                    resp.extend_from_slice(&ID_DENT_V1.to_le_bytes());
                    resp.extend_from_slice(&node.mode.to_le_bytes());
                    resp.extend_from_slice(&(node.size() as u32).to_le_bytes());
                    resp.extend_from_slice(&(node.mtime as u32).to_le_bytes());
                    resp.extend_from_slice(&(name_bytes.len() as u32).to_le_bytes());
                    resp.extend_from_slice(name_bytes);
                    stream_state.pending_writes.push_back(resp);
                }
                let mut done_resp = Vec::with_capacity(20);
                done_resp.extend_from_slice(&ID_DONE.to_le_bytes());
                done_resp.extend_from_slice(&[0u8; 16]);
                queue_sync_reply(stream_state, diagnostics, done_resp);
            }
            ID_LIST_V2 => {
                buffer.drain(..8 + path_length);
                let entries = fs.list_dir(&path);
                for (name, node) in entries {
                    let name_bytes = name.as_bytes();
                    let mut resp = Vec::with_capacity(72 + name_bytes.len());
                    resp.extend_from_slice(&ID_DENT_V2.to_le_bytes());
                    resp.extend_from_slice(&0u32.to_le_bytes()); // error = 0
                    resp.extend_from_slice(&node.dev.to_le_bytes());
                    resp.extend_from_slice(&node.ino.to_le_bytes());
                    resp.extend_from_slice(&node.mode.to_le_bytes());
                    resp.extend_from_slice(&(node.nlink as u32).to_le_bytes());
                    resp.extend_from_slice(&node.uid.to_le_bytes());
                    resp.extend_from_slice(&node.gid.to_le_bytes());
                    resp.extend_from_slice(&node.size().to_le_bytes());
                    resp.extend_from_slice(&(node.atime as i64).to_le_bytes());
                    resp.extend_from_slice(&(node.mtime as i64).to_le_bytes());
                    resp.extend_from_slice(&(node.ctime as i64).to_le_bytes());
                    resp.extend_from_slice(&(name_bytes.len() as u32).to_le_bytes());
                    resp.extend_from_slice(name_bytes);
                    stream_state.pending_writes.push_back(resp);
                }
                let mut done_resp = Vec::with_capacity(72);
                done_resp.extend_from_slice(&ID_DONE.to_le_bytes());
                done_resp.extend_from_slice(&[0u8; 68]);
                queue_sync_reply(stream_state, diagnostics, done_resp);
            }
            ID_RECV_V1 => {
                buffer.drain(..8 + path_length);
                match fs.get_file(&path) {
                    Some(contents) => {
                        let chunk_size = 64 * 1024;
                        for chunk in contents.chunks(chunk_size) {
                            let mut resp = Vec::with_capacity(8 + chunk.len());
                            resp.extend_from_slice(&ID_DATA.to_le_bytes());
                            resp.extend_from_slice(&(chunk.len() as u32).to_le_bytes());
                            resp.extend_from_slice(chunk);
                            stream_state.pending_writes.push_back(resp);
                        }
                        let mut done_resp = Vec::with_capacity(8);
                        done_resp.extend_from_slice(&ID_DONE.to_le_bytes());
                        done_resp.extend_from_slice(&0u32.to_le_bytes());
                        queue_sync_reply(stream_state, diagnostics, done_resp);
                    }
                    None => {
                        stream_state
                            .pending_writes
                            .push_back(encode_sync_fail("no such file or directory"));
                    }
                }
            }
            ID_RECV_V2 => {
                if buffer.len() < 8 + path_length + 8 {
                    break;
                }
                let setup_bytes = &buffer[8 + path_length..8 + path_length + 8];
                let setup_id = u32::from_le_bytes(setup_bytes[0..4].try_into().unwrap());
                let flags = u32::from_le_bytes(setup_bytes[4..8].try_into().unwrap());
                buffer.drain(..8 + path_length + 8);

                if setup_id != ID_RECV_V2 {
                    stream_state
                        .pending_writes
                        .push_back(encode_sync_fail("mismatched recv_v2 setup id"));
                } else if flags != 0 {
                    stream_state
                        .pending_writes
                        .push_back(encode_sync_fail(&format!("unknown flags: {flags}")));
                } else {
                    match fs.get_file(&path) {
                        Some(contents) => {
                            let chunk_size = 64 * 1024;
                            for chunk in contents.chunks(chunk_size) {
                                let mut resp = Vec::with_capacity(8 + chunk.len());
                                resp.extend_from_slice(&ID_DATA.to_le_bytes());
                                resp.extend_from_slice(&(chunk.len() as u32).to_le_bytes());
                                resp.extend_from_slice(chunk);
                                stream_state.pending_writes.push_back(resp);
                            }
                            let mut done_resp = Vec::with_capacity(8);
                            done_resp.extend_from_slice(&ID_DONE.to_le_bytes());
                            done_resp.extend_from_slice(&0u32.to_le_bytes());
                            queue_sync_reply(stream_state, diagnostics, done_resp);
                        }
                        None => {
                            stream_state
                                .pending_writes
                                .push_back(encode_sync_fail("no such file or directory"));
                        }
                    }
                }
            }
            ID_SEND_V1 => {
                let (target_path, mode) = match parse_send_v1_spec(&path) {
                    Some(res) => res,
                    None => {
                        buffer.drain(..8 + path_length);
                        stream_state
                            .pending_writes
                            .push_back(encode_sync_fail("invalid SEND_V1 spec"));
                        continue;
                    }
                };

                let mut offset = 8 + path_length;
                let mut file_data = Vec::new();
                let mut mtime = 0u32;
                let mut send_complete = false;

                while offset + 8 <= buffer.len() {
                    let chunk_id =
                        u32::from_le_bytes(buffer[offset..offset + 4].try_into().unwrap());
                    let chunk_size =
                        u32::from_le_bytes(buffer[offset + 4..offset + 8].try_into().unwrap())
                            as usize;

                    if chunk_id == ID_DATA {
                        if chunk_size > 64 * 1024 {
                            stream_state
                                .pending_writes
                                .push_back(encode_sync_fail("data chunk too large"));
                            send_complete = false;
                            break;
                        }
                        if offset + 8 + chunk_size > buffer.len() {
                            break;
                        }
                        file_data.extend_from_slice(&buffer[offset + 8..offset + 8 + chunk_size]);
                        offset += 8 + chunk_size;
                    } else if chunk_id == ID_DONE {
                        mtime = chunk_size as u32;
                        offset += 8;
                        send_complete = true;
                        break;
                    } else {
                        stream_state
                            .pending_writes
                            .push_back(encode_sync_fail("invalid chunk id in SEND"));
                        buffer.drain(..offset);
                        send_complete = false;
                        break;
                    }
                }

                if !send_complete {
                    break;
                }

                buffer.drain(..offset);

                let parent_dir = parent_and_name(&target_path).map(|(p, _)| p);
                if let Some(p) = parent_dir {
                    if fs.get_node(p).is_none() {
                        stream_state
                            .pending_writes
                            .push_back(encode_sync_fail(&format!("directory {p} does not exist")));
                        continue;
                    }
                }

                fs.add_file(&target_path, file_data, mode, mtime as u64);
                queue_sync_reply(stream_state, diagnostics, encode_sync_okay());
            }
            ID_SEND_V2 => {
                if buffer.len() < 8 + path_length + 12 {
                    break;
                }
                let setup_bytes = &buffer[8 + path_length..8 + path_length + 12];
                let setup_id = u32::from_le_bytes(setup_bytes[0..4].try_into().unwrap());
                let mode = u32::from_le_bytes(setup_bytes[4..8].try_into().unwrap());
                let flags = u32::from_le_bytes(setup_bytes[8..12].try_into().unwrap());

                if setup_id != ID_SEND_V2 {
                    buffer.drain(..8 + path_length + 12);
                    stream_state
                        .pending_writes
                        .push_back(encode_sync_fail("mismatched send_v2 setup id"));
                    continue;
                }
                if flags != 0 {
                    buffer.drain(..8 + path_length + 12);
                    stream_state
                        .pending_writes
                        .push_back(encode_sync_fail(&format!("unknown flags: {flags}")));
                    continue;
                }

                let mut offset = 8 + path_length + 12;
                let mut file_data = Vec::new();
                let mut mtime = 0u32;
                let mut send_complete = false;

                while offset + 8 <= buffer.len() {
                    let chunk_id =
                        u32::from_le_bytes(buffer[offset..offset + 4].try_into().unwrap());
                    let chunk_size =
                        u32::from_le_bytes(buffer[offset + 4..offset + 8].try_into().unwrap())
                            as usize;

                    if chunk_id == ID_DATA {
                        if chunk_size > 64 * 1024 {
                            stream_state
                                .pending_writes
                                .push_back(encode_sync_fail("data chunk too large"));
                            send_complete = false;
                            break;
                        }
                        if offset + 8 + chunk_size > buffer.len() {
                            break;
                        }
                        file_data.extend_from_slice(&buffer[offset + 8..offset + 8 + chunk_size]);
                        offset += 8 + chunk_size;
                    } else if chunk_id == ID_DONE {
                        mtime = chunk_size as u32;
                        offset += 8;
                        send_complete = true;
                        break;
                    } else {
                        stream_state
                            .pending_writes
                            .push_back(encode_sync_fail("invalid chunk id in SEND_V2"));
                        buffer.drain(..offset);
                        send_complete = false;
                        break;
                    }
                }

                if !send_complete {
                    break;
                }

                buffer.drain(..offset);

                let parent_dir = parent_and_name(&path).map(|(p, _)| p);
                if let Some(p) = parent_dir {
                    if fs.get_node(p).is_none() {
                        stream_state
                            .pending_writes
                            .push_back(encode_sync_fail(&format!("directory {p} does not exist")));
                        continue;
                    }
                }

                fs.add_file(&path, file_data, mode, mtime as u64);
                queue_sync_reply(stream_state, diagnostics, encode_sync_okay());
            }
            ID_QUIT => {
                buffer.drain(..8 + path_length);
                stream_state.close_pending = true;
                break;
            }
            unknown => {
                buffer.drain(..8 + path_length);
                stream_state
                    .pending_writes
                    .push_back(encode_sync_fail(&format!("unknown command {unknown:08x}")));
                break;
            }
        }
    }
}
