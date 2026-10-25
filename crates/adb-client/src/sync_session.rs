use crate::client::AdbClient;
use crate::session::{Session, SessionError};
use crate::transport::TransportSelector;
use adb_proto::sync::{
    ID_DATA, ID_DENT_V1, ID_DENT_V2, ID_DONE, ID_FAIL, ID_LIST_V1, ID_LIST_V2, ID_LSTAT_V1,
    ID_LSTAT_V2, ID_OKAY, ID_QUIT, ID_RECV_V1, ID_RECV_V2, ID_SEND_V1, ID_SEND_V2, ID_STAT_V2,
    SYNC_DATA_MAX,
};
use std::collections::VecDeque;
use std::fmt;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Converts wire errno values (Linux asm-generic) to host OS errno.
pub fn errno_from_wire(wire: u32) -> i32 {
    match wire {
        0 => 0,
        1 => 1,   // EPERM
        2 => 2,   // ENOENT
        4 => 4,   // EINTR
        5 => 5,   // EIO
        12 => 12, // ENOMEM
        13 => 13, // EACCES
        14 => 14, // EFAULT
        17 => 17, // EEXIST
        20 => 20, // ENOTDIR
        21 => 21, // EISDIR
        22 => 22, // EINVAL
        23 => 23, // ENFILE
        24 => 24, // EMFILE
        26 => 26, // ETXTBSY
        27 => 27, // EFBIG
        28 => 28, // ENOSPC
        30 => 30, // EROFS
        36 => 36, // ENAMETOOLONG
        40 => 40, // ELOOP
        75 => 75, // EOVERFLOW
        _ => 5,   // EIO fallback
    }
}

/// Errors that can occur during a Sync session operation.
#[derive(Debug)]
pub enum SyncError {
    ConnectFailed(String),
    PathTooLong {
        message: String,
        errno: i32,
    },
    NotSupported(String),
    ProtocolFault(String),
    HostilePath(String),
    Errno {
        wire: u32,
        errno: i32,
        message: String,
    },
    Io(std::io::Error),
    Session(SessionError),
}

impl SyncError {
    pub fn raw_os_error(&self) -> Option<i32> {
        match self {
            SyncError::PathTooLong { errno, .. } => Some(*errno),
            SyncError::Errno { errno, .. } => Some(*errno),
            SyncError::Io(err) => err.raw_os_error(),
            _ => None,
        }
    }
}

impl fmt::Display for SyncError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SyncError::ConnectFailed(msg) => write!(f, "adb: error: {msg}"),
            SyncError::PathTooLong { message, .. } => write!(f, "{message}"),
            SyncError::NotSupported(msg) => write!(f, "{msg}"),
            SyncError::ProtocolFault(msg) => write!(f, "protocol fault: {msg}"),
            SyncError::HostilePath(msg) => write!(f, "{msg}"),
            SyncError::Errno { message, .. } => write!(f, "{message}"),
            SyncError::Io(err) => write!(f, "IO error: {err}"),
            SyncError::Session(err) => write!(f, "{err}"),
        }
    }
}

impl std::error::Error for SyncError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            SyncError::Io(err) => Some(err),
            SyncError::Session(err) => Some(err),
            _ => None,
        }
    }
}

impl From<std::io::Error> for SyncError {
    fn from(err: std::io::Error) -> Self {
        SyncError::Io(err)
    }
}

impl From<SessionError> for SyncError {
    fn from(err: SessionError) -> Self {
        SyncError::Session(err)
    }
}

/// Compression algorithm option for sync operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CompressionType {
    #[default]
    None,
    Any,
    Brotli,
    LZ4,
    Zstd,
}

/// Options controlling `pull` behavior.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PullOptions {
    pub copy_attrs: bool,
    pub compression: CompressionType,
}

/// Outcome of pulling a single source file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PullSourceOutcome {
    pub source: String,
    pub destination: String,
    pub bytes_transferred: u64,
    pub success: bool,
    pub error: Option<String>,
}

/// Summary result of a `pull` operation over one or more sources.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PullResult {
    pub bytes_transferred: u64,
    pub outcomes: Vec<PullSourceOutcome>,
    pub warnings: Vec<String>,
    pub errors: Vec<String>,
    pub success: bool,
}

/// Options controlling `push` behavior.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PushOptions {
    pub sync: bool,
    pub compression: CompressionType,
    pub dry_run: bool,
}

/// Outcome of pushing a single source file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PushSourceOutcome {
    pub source: String,
    pub destination: String,
    pub bytes_transferred: u64,
    pub success: bool,
    pub error: Option<String>,
}

/// Summary result of a `push` operation over one or more sources.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PushResult {
    pub bytes_transferred: u64,
    pub outcomes: Vec<PushSourceOutcome>,
    pub warnings: Vec<String>,
    pub errors: Vec<String>,
    pub success: bool,
}

/// Represents a directory entry returned by sync `ls` request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirEntry {
    pub mode: u32,
    pub size: u64,
    pub mtime: u64,
    pub name: String,
}

impl DirEntry {
    /// Formats the directory entry into standard `adb ls` line output format (`%08x %08x %08x %s\n`).
    pub fn format_line(&self) -> String {
        format!(
            "{:08x} {:08x} {:08x} {}\n",
            self.mode, self.size, self.mtime, self.name
        )
    }
}

/// Represents file status information returned by sync `stat` or `lstat` requests.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Stat {
    pub id: u32,
    pub error: u32,
    pub dev: u64,
    pub ino: u64,
    pub mode: u32,
    pub nlink: u32,
    pub uid: u32,
    pub gid: u32,
    pub size: u64,
    pub atime: i64,
    pub mtime: i64,
    pub ctime: i64,
}

impl Stat {
    pub fn is_dir(&self) -> bool {
        (self.mode & 0o170000) == 0o040000
    }

    pub fn is_file(&self) -> bool {
        (self.mode & 0o170000) == 0o100000
    }

    pub fn is_symlink(&self) -> bool {
        (self.mode & 0o170000) == 0o120000
    }
}

/// An active ADB sync connection session supporting `ls`, `stat`, `lstat`, `stat_with_fallback`, and `pull`.
pub struct SyncSession {
    session: Session,
    features: adb_proto::Features,
    have_stat_v2: bool,
    have_ls_v2: bool,
    have_sendrecv_v2: bool,
    have_sendrecv_v2_brotli: bool,
    have_sendrecv_v2_lz4: bool,
    have_sendrecv_v2_zstd: bool,
    have_sendrecv_v2_dry_run_send: bool,
    have_fixed_push_mkdir: bool,
}

impl SyncSession {
    /// Connects a sync session using default transport selector (`Any`).
    pub async fn connect(client: &AdbClient) -> Result<Self, SyncError> {
        Self::connect_with_selector(client, &TransportSelector::Any).await
    }

    /// Connects a sync session targeting device specified by `selector`.
    pub async fn connect_with_selector(
        client: &AdbClient,
        selector: &TransportSelector,
    ) -> Result<Self, SyncError> {
        let features_res = client.features(selector).await;
        let features_str = match features_res.result {
            Ok(f) => f,
            Err(e) => {
                let err_msg = format!("failed to get feature set: {e}");
                return Err(SyncError::ConnectFailed(err_msg));
            }
        };

        let features = adb_proto::Features::parse(&features_str.replace('\n', ","));

        let have_stat_v2 = features.contains(&adb_proto::Feature::Stat2);
        let have_ls_v2 = features.contains(&adb_proto::Feature::Ls2);
        let have_sendrecv_v2 = features.contains(&adb_proto::Feature::SendRecv2);
        let have_sendrecv_v2_brotli = features.contains(&adb_proto::Feature::SendRecv2Brotli);
        let have_sendrecv_v2_lz4 = features.contains(&adb_proto::Feature::SendRecv2LZ4);
        let have_sendrecv_v2_zstd = features.contains(&adb_proto::Feature::SendRecv2Zstd);
        let have_sendrecv_v2_dry_run_send =
            features.contains(&adb_proto::Feature::SendRecv2DryRunSend);
        let have_fixed_push_mkdir = features.contains(&adb_proto::Feature::FixedPushMkdir);

        let session_res = client.session_factory().connect(client.addr()).await;
        let mut session = match session_res {
            Ok(s) => s,
            Err(e) => {
                let err_msg = format!("connect failed: {e}");
                return Err(SyncError::ConnectFailed(err_msg));
            }
        };

        if let Err(e) = session.select_transport(selector).await {
            let err_msg = format!("connect failed: {e}");
            return Err(SyncError::ConnectFailed(err_msg));
        }

        if let Err(e) = session.send_service("sync:").await {
            let err_msg = format!("connect failed: {e}");
            return Err(SyncError::ConnectFailed(err_msg));
        }

        if let Err(e) = session.read_status().await {
            let err_msg = format!("connect failed: {e}");
            return Err(SyncError::ConnectFailed(err_msg));
        }

        Ok(Self {
            session,
            features,
            have_stat_v2,
            have_ls_v2,
            have_sendrecv_v2,
            have_sendrecv_v2_brotli,
            have_sendrecv_v2_lz4,
            have_sendrecv_v2_zstd,
            have_sendrecv_v2_dry_run_send,
            have_fixed_push_mkdir,
        })
    }

    pub fn features(&self) -> &adb_proto::Features {
        &self.features
    }

    pub fn have_stat_v2(&self) -> bool {
        self.have_stat_v2
    }

    pub fn have_ls_v2(&self) -> bool {
        self.have_ls_v2
    }

    pub fn have_sendrecv_v2(&self) -> bool {
        self.have_sendrecv_v2
    }

    pub fn have_sendrecv_v2_brotli(&self) -> bool {
        self.have_sendrecv_v2_brotli
    }

    pub fn have_sendrecv_v2_lz4(&self) -> bool {
        self.have_sendrecv_v2_lz4
    }

    pub fn have_sendrecv_v2_zstd(&self) -> bool {
        self.have_sendrecv_v2_zstd
    }

    pub fn have_sendrecv_v2_dry_run_send(&self) -> bool {
        self.have_sendrecv_v2_dry_run_send
    }

    pub fn have_fixed_push_mkdir(&self) -> bool {
        self.have_fixed_push_mkdir
    }

    async fn send_request(&mut self, id: u32, path: &str) -> Result<(), SyncError> {
        if path.len() > 1024 {
            let message = format!("SendRequest failed: path too long: {}", path.len());
            return Err(SyncError::PathTooLong { message, errno: 36 });
        }

        let path_bytes = path.as_bytes();
        let mut buf = Vec::with_capacity(8 + path_bytes.len());
        buf.extend_from_slice(&id.to_le_bytes());
        buf.extend_from_slice(&(path_bytes.len() as u32).to_le_bytes());
        buf.extend_from_slice(path_bytes);

        let stream = self.session.stream_mut();
        stream.write_all(&buf).await?;
        stream.flush().await?;
        Ok(())
    }

    /// Lists entries in directory `path` (`adb ls`).
    pub async fn ls(&mut self, path: &str) -> Result<Vec<DirEntry>, SyncError> {
        let req_id = if self.have_ls_v2 {
            ID_LIST_V2
        } else {
            ID_LIST_V1
        };
        self.send_request(req_id, path).await?;

        let mut entries = Vec::new();
        let stream = self.session.stream_mut();

        if self.have_ls_v2 {
            loop {
                let mut dent_buf = [0u8; 76];
                stream.read_exact(&mut dent_buf).await?;

                let id = u32::from_le_bytes(dent_buf[0..4].try_into().unwrap());
                if id == ID_DONE {
                    break;
                }
                if id != ID_DENT_V2 {
                    return Err(SyncError::ProtocolFault(format!(
                        "unexpected response ID 0x{id:08x}"
                    )));
                }

                let mode = u32::from_le_bytes(dent_buf[24..28].try_into().unwrap());
                let size = u64::from_le_bytes(dent_buf[40..48].try_into().unwrap());
                let mtime = i64::from_le_bytes(dent_buf[56..64].try_into().unwrap()) as u64;
                let namelen = u32::from_le_bytes(dent_buf[72..76].try_into().unwrap()) as usize;

                if namelen > 255 {
                    return Err(SyncError::ProtocolFault("filename too long".to_string()));
                }

                let mut name_buf = vec![0u8; namelen];
                stream.read_exact(&mut name_buf).await?;

                if name_buf.contains(&b'/') || name_buf.contains(&b'\\') {
                    return Err(SyncError::HostilePath(
                        "entry contains path separator".to_string(),
                    ));
                }

                let name = String::from_utf8(name_buf)
                    .map_err(|e| SyncError::ProtocolFault(e.to_string()))?;

                entries.push(DirEntry {
                    mode,
                    size,
                    mtime,
                    name,
                });
            }
        } else {
            loop {
                let mut dent_buf = [0u8; 20];
                stream.read_exact(&mut dent_buf).await?;

                let id = u32::from_le_bytes(dent_buf[0..4].try_into().unwrap());
                if id == ID_DONE {
                    break;
                }
                if id != ID_DENT_V1 {
                    return Err(SyncError::ProtocolFault(format!(
                        "unexpected response ID 0x{id:08x}"
                    )));
                }

                let mode = u32::from_le_bytes(dent_buf[4..8].try_into().unwrap());
                let size = u32::from_le_bytes(dent_buf[8..12].try_into().unwrap()) as u64;
                let mtime = u32::from_le_bytes(dent_buf[12..16].try_into().unwrap()) as u64;
                let namelen = u32::from_le_bytes(dent_buf[16..20].try_into().unwrap()) as usize;

                if namelen > 255 {
                    return Err(SyncError::ProtocolFault("filename too long".to_string()));
                }

                let mut name_buf = vec![0u8; namelen];
                stream.read_exact(&mut name_buf).await?;

                if name_buf.contains(&b'/') || name_buf.contains(&b'\\') {
                    return Err(SyncError::HostilePath(
                        "entry contains path separator".to_string(),
                    ));
                }

                let name = String::from_utf8(name_buf)
                    .map_err(|e| SyncError::ProtocolFault(e.to_string()))?;

                entries.push(DirEntry {
                    mode,
                    size,
                    mtime,
                    name,
                });
            }
        }

        Ok(entries)
    }

    /// Queries file stat using `stat_v2` request.
    pub async fn stat(&mut self, path: &str) -> Result<Stat, SyncError> {
        if !self.have_stat_v2 {
            return Err(SyncError::NotSupported("stat_v2 not supported".to_string()));
        }
        self.send_request(ID_STAT_V2, path).await?;
        self.finish_stat().await
    }

    /// Queries file lstat using `lstat_v2` (or `lstat_v1` when `stat_v2` is absent).
    pub async fn lstat(&mut self, path: &str) -> Result<Stat, SyncError> {
        if self.have_stat_v2 {
            self.send_request(ID_LSTAT_V2, path).await?;
        } else {
            self.send_request(ID_LSTAT_V1, path).await?;
        }
        self.finish_stat().await
    }

    /// Queries file stat, falling back to `lstat` and symlink directory probing when `stat_v2` is not supported.
    pub async fn stat_with_fallback(&mut self, path: &str) -> Result<Stat, SyncError> {
        match self.stat(path).await {
            Ok(st) => Ok(st),
            Err(SyncError::NotSupported(_)) => {
                let mut st = self.lstat(path).await?;
                if st.is_symlink() {
                    st.size = 0;
                    let dir_path = format!("{path}/");
                    st.mode &= !0o170000;
                    if let Ok(_tmp_st) = self.lstat(&dir_path).await {
                        st.mode |= 0o040000;
                    } else {
                        st.mode |= 0o100000;
                    }
                }
                Ok(st)
            }
            Err(err) => Err(err),
        }
    }

    async fn finish_stat(&mut self) -> Result<Stat, SyncError> {
        let stream = self.session.stream_mut();

        if self.have_stat_v2 {
            let mut buf = [0u8; 72];
            stream.read_exact(&mut buf).await?;

            let id = u32::from_le_bytes(buf[0..4].try_into().unwrap());
            if id != ID_LSTAT_V2 && id != ID_STAT_V2 {
                return Err(SyncError::ProtocolFault(format!(
                    "stat response has wrong message id: 0x{id:08x}"
                )));
            }

            let error = u32::from_le_bytes(buf[4..8].try_into().unwrap());
            if error != 0 {
                let host_errno = errno_from_wire(error);
                return Err(SyncError::Errno {
                    wire: error,
                    errno: host_errno,
                    message: match host_errno {
                        2 => "No such file or directory".to_string(),
                        _ => format!("wire error {error}"),
                    },
                });
            }

            let dev = u64::from_le_bytes(buf[8..16].try_into().unwrap());
            let ino = u64::from_le_bytes(buf[16..24].try_into().unwrap());
            let mode = u32::from_le_bytes(buf[24..28].try_into().unwrap());
            let nlink = u32::from_le_bytes(buf[28..32].try_into().unwrap());
            let uid = u32::from_le_bytes(buf[32..36].try_into().unwrap());
            let gid = u32::from_le_bytes(buf[36..40].try_into().unwrap());
            let size = u64::from_le_bytes(buf[40..48].try_into().unwrap());
            let atime = i64::from_le_bytes(buf[48..56].try_into().unwrap());
            let mtime = i64::from_le_bytes(buf[56..64].try_into().unwrap());
            let ctime = i64::from_le_bytes(buf[64..72].try_into().unwrap());

            Ok(Stat {
                id,
                error: 0,
                dev,
                ino,
                mode,
                nlink,
                uid,
                gid,
                size,
                atime,
                mtime,
                ctime,
            })
        } else {
            let mut buf = [0u8; 16];
            stream.read_exact(&mut buf).await?;

            let id = u32::from_le_bytes(buf[0..4].try_into().unwrap());
            if id != ID_LSTAT_V1 {
                return Err(SyncError::ProtocolFault(format!(
                    "stat response has wrong message id: 0x{id:08x}"
                )));
            }

            let mode = u32::from_le_bytes(buf[4..8].try_into().unwrap());
            let size = u32::from_le_bytes(buf[8..12].try_into().unwrap());
            let mtime = u32::from_le_bytes(buf[12..16].try_into().unwrap());

            if mode == 0 && size == 0 && mtime == 0 {
                return Err(SyncError::Errno {
                    wire: 2,
                    errno: 2,
                    message: "No such file or directory".to_string(),
                });
            }

            Ok(Stat {
                id,
                error: 0,
                dev: 0,
                ino: 0,
                mode,
                nlink: 1,
                uid: 0,
                gid: 0,
                size: size as u64,
                atime: mtime as i64,
                mtime: mtime as i64,
                ctime: mtime as i64,
            })
        }
    }

    /// Pulls one or more remote files to a local destination path.
    pub async fn pull<P: AsRef<std::path::Path>>(
        &mut self,
        sources: &[&str],
        destination: P,
        options: &PullOptions,
    ) -> Result<PullResult, SyncError> {
        if options.compression != CompressionType::None {
            return Err(SyncError::NotSupported(
                "compression option is not supported".to_string(),
            ));
        }

        let dst = destination.as_ref();
        let dst_str = dst.to_string_lossy().to_string();
        let dst_bytes = dst_str.as_bytes();
        let ends_with_sep = !dst_bytes.is_empty()
            && (dst_bytes.last() == Some(&b'/') || dst_bytes.last() == Some(&b'\\'));

        let stripped_str = dst_str.trim_end_matches(['/', '\\']);
        let stripped_path = if stripped_str.is_empty() {
            dst
        } else {
            std::path::Path::new(stripped_str)
        };

        let mut dst_exists = true;
        let mut dst_isdir = false;

        match std::fs::metadata(stripped_path) {
            Ok(st) => {
                dst_isdir = st.is_dir();
            }
            Err(err) => {
                dst_exists = false;
                if sources.len() == 1 && err.kind() == std::io::ErrorKind::NotFound {
                    let parent = dst.parent().unwrap_or_else(|| std::path::Path::new(""));
                    let parent = if parent.as_os_str().is_empty() {
                        std::path::Path::new(".")
                    } else {
                        parent
                    };
                    if let Err(parent_err) = std::fs::metadata(parent) {
                        let mut res = PullResult::default();
                        res.errors.push(format!(
                            "cannot create file/directory '{dst_str}': {parent_err}"
                        ));
                        return Ok(res);
                    }
                } else {
                    let mut res = PullResult::default();
                    res.errors
                        .push(format!("failed to access '{dst_str}': {err}"));
                    return Ok(res);
                }
            }
        }

        if !dst_isdir {
            if sources.len() > 1 {
                let mut res = PullResult::default();
                res.errors
                    .push(format!("target '{dst_str}' is not a directory"));
                return Ok(res);
            } else if ends_with_sep && dst_exists {
                let mut res = PullResult::default();
                res.errors
                    .push(format!("failed to access '{dst_str}': Not a directory"));
                return Ok(res);
            }
        }

        let mut res = PullResult {
            success: true,
            ..Default::default()
        };

        for src in sources {
            let src_path = *src;

            let src_st = match self.stat_with_fallback(src_path).await {
                Ok(st) => st,
                Err(SyncError::Errno { errno: 2, .. }) | Err(SyncError::Errno { wire: 2, .. }) => {
                    let msg = format!("remote object '{src_path}' does not exist");
                    res.errors.push(msg.clone());
                    res.success = false;
                    res.outcomes.push(PullSourceOutcome {
                        source: src_path.to_string(),
                        destination: dst_str.clone(),
                        bytes_transferred: 0,
                        success: false,
                        error: Some(msg),
                    });
                    continue;
                }
                Err(err) => {
                    let msg = format!("failed to stat remote object '{src_path}': {err}");
                    res.errors.push(msg.clone());
                    res.success = false;
                    res.outcomes.push(PullSourceOutcome {
                        source: src_path.to_string(),
                        destination: dst_str.clone(),
                        bytes_transferred: 0,
                        success: false,
                        error: Some(msg),
                    });
                    continue;
                }
            };

            let mode = src_st.mode;
            let file_type = mode & 0o170000;
            let is_pullable =
                file_type == 0o100000 || file_type == 0o060000 || file_type == 0o020000;
            if !is_pullable {
                res.warnings.push(format!(
                    "skipping special file '{src_path}' (mode = 0o{mode:o})"
                ));
                continue;
            }

            let target_dst_path: std::path::PathBuf = if dst_isdir {
                let basename = std::path::Path::new(src_path)
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or(src_path);
                dst.join(basename)
            } else {
                dst.to_path_buf()
            };

            let target_dst_str = target_dst_path.to_string_lossy().to_string();

            match self.sync_recv(src_path, &target_dst_path).await {
                Ok(bytes) => {
                    if options.copy_attrs {
                        if let Err(e) =
                            set_time_and_mode(&target_dst_path, src_st.mtime as u64, src_st.mode)
                        {
                            let msg =
                                format!("failed to set time and mode on '{target_dst_str}': {e}");
                            res.errors.push(msg.clone());
                            res.success = false;
                            res.outcomes.push(PullSourceOutcome {
                                source: src_path.to_string(),
                                destination: target_dst_str,
                                bytes_transferred: bytes,
                                success: false,
                                error: Some(msg),
                            });
                            continue;
                        }
                    }
                    res.bytes_transferred += bytes;
                    res.outcomes.push(PullSourceOutcome {
                        source: src_path.to_string(),
                        destination: target_dst_str,
                        bytes_transferred: bytes,
                        success: true,
                        error: None,
                    });
                }
                Err(err_msg) => {
                    res.errors.push(err_msg.clone());
                    res.success = false;
                    res.outcomes.push(PullSourceOutcome {
                        source: src_path.to_string(),
                        destination: target_dst_str,
                        bytes_transferred: 0,
                        success: false,
                        error: Some(err_msg),
                    });
                }
            }
        }

        Ok(res)
    }

    async fn sync_recv(&mut self, rpath: &str, lpath: &std::path::Path) -> Result<u64, String> {
        if self.have_sendrecv_v2 {
            self.send_recv_v2_req(rpath).await?;
        } else {
            self.send_request(ID_RECV_V1, rpath)
                .await
                .map_err(|e| e.to_string())?;
        }

        let _ = std::fs::remove_file(lpath);

        let mut file = match std::fs::File::create(lpath) {
            Ok(f) => f,
            Err(e) => {
                return Err(format!("cannot create '{}': {e}", lpath.display()));
            }
        };

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = file.set_permissions(std::fs::Permissions::from_mode(0o644));
        }

        let stream = self.session.stream_mut();
        let mut bytes_copied = 0u64;

        let res = async {
            loop {
                let mut header_buf = [0u8; 8];
                if stream.read_exact(&mut header_buf).await.is_err() {
                    return Err("failed to read sync response header".to_string());
                }

                let id = u32::from_le_bytes(header_buf[0..4].try_into().unwrap());
                let size = u32::from_le_bytes(header_buf[4..8].try_into().unwrap());

                if id == ID_DONE {
                    break;
                }

                if id != ID_DATA {
                    let msglen = size as usize;
                    let mut fail_buf = vec![0u8; msglen];
                    let _ = stream.read_exact(&mut fail_buf).await;
                    let fail_msg = String::from_utf8_lossy(&fail_buf);
                    return Err(format!(
                        "failed to copy '{rpath}' to '{}': remote {fail_msg}",
                        lpath.display()
                    ));
                }

                let data_len = size as usize;
                if data_len > adb_proto::sync::SYNC_DATA_MAX {
                    return Err(format!(
                        "msg.data.size too large: {data_len} (max {})",
                        adb_proto::sync::SYNC_DATA_MAX
                    ));
                }

                let mut chunk = vec![0u8; data_len];
                if stream.read_exact(&mut chunk).await.is_err() {
                    return Err("failed to read chunk payload".to_string());
                }

                use std::io::Write;
                if let Err(e) = file.write_all(&chunk) {
                    return Err(format!("cannot write '{}': {e}", lpath.display()));
                }

                bytes_copied += data_len as u64;
            }
            Ok(bytes_copied)
        }
        .await;

        if res.is_err() {
            let _ = std::fs::remove_file(lpath);
        }

        res
    }

    async fn send_recv_v2_req(&mut self, path: &str) -> Result<(), String> {
        if path.len() > 1024 {
            return Err(format!("SendRequest failed: path too long: {}", path.len()));
        }

        let path_bytes = path.as_bytes();
        let mut buf = Vec::with_capacity(8 + path_bytes.len() + 8);
        buf.extend_from_slice(&ID_RECV_V2.to_le_bytes());
        buf.extend_from_slice(&(path_bytes.len() as u32).to_le_bytes());
        buf.extend_from_slice(path_bytes);
        buf.extend_from_slice(&ID_RECV_V2.to_le_bytes());
        buf.extend_from_slice(&0u32.to_le_bytes());

        let stream = self.session.stream_mut();
        stream
            .write_all(&buf)
            .await
            .map_err(|e| format!("failed to send RECV_V2 request: {e}"))?;
        stream
            .flush()
            .await
            .map_err(|e| format!("failed to flush RECV_V2 request: {e}"))?;
        Ok(())
    }

    /// Pushes one or more local files to a remote destination path.
    pub async fn push<P: AsRef<std::path::Path>>(
        &mut self,
        sources: &[&str],
        destination: P,
        options: &PushOptions,
    ) -> Result<PushResult, SyncError> {
        if options.compression != CompressionType::None {
            return Err(SyncError::NotSupported(
                "compression option is not supported".to_string(),
            ));
        }
        if options.dry_run {
            return Err(SyncError::NotSupported(
                "dry-run option is not supported".to_string(),
            ));
        }
        if options.sync {
            return Err(SyncError::NotSupported(
                "sync option is not supported".to_string(),
            ));
        }

        let dst = destination.as_ref();
        let dst_str = dst.to_string_lossy().to_string();

        let mut dst_exists = true;
        let mut dst_isdir = false;

        match self.stat_with_fallback(&dst_str).await {
            Ok(st) => {
                dst_isdir = st.is_dir();
            }
            Err(SyncError::Errno { errno, .. }) if errno == 2 || errno == 95 => {
                dst_exists = false;
            }
            Err(err) => {
                let mut res = PushResult::default();
                res.errors.push(format!(
                    "stat failed when trying to push to {dst_str}: {err}"
                ));
                return Ok(res);
            }
        }

        if !dst_isdir {
            if sources.len() > 1 {
                let mut res = PushResult::default();
                res.errors
                    .push(format!("target '{dst_str}' is not a directory"));
                return Ok(res);
            } else {
                let dst_bytes = dst_str.as_bytes();
                let ends_with_sep = !dst_bytes.is_empty()
                    && (dst_bytes.last() == Some(&b'/') || dst_bytes.last() == Some(&b'\\'));
                if ends_with_sep && dst_exists {
                    let mut res = PushResult::default();
                    res.errors
                        .push(format!("failed to access '{dst_str}': Not a directory"));
                    return Ok(res);
                }
            }
        }

        let mut res = PushResult {
            success: true,
            ..Default::default()
        };
        let mut deferred_acks: VecDeque<(String, String, u64)> = VecDeque::new();

        for src in sources {
            let src_path = *src;
            let local_path = std::path::Path::new(src_path);

            let local_st = match std::fs::symlink_metadata(local_path) {
                Ok(st) => st,
                Err(e) => {
                    let msg = format!("cannot stat '{src_path}': {e}");
                    res.errors.push(msg.clone());
                    res.success = false;
                    res.outcomes.push(PushSourceOutcome {
                        source: src_path.to_string(),
                        destination: dst_str.clone(),
                        bytes_transferred: 0,
                        success: false,
                        error: Some(msg),
                    });
                    continue;
                }
            };

            let file_type = local_st.file_type();
            if file_type.is_dir() {
                return Err(SyncError::NotSupported(
                    "directory push is not supported".to_string(),
                ));
            }

            #[cfg(unix)]
            let mode = {
                use std::os::unix::fs::MetadataExt;
                local_st.mode()
            };
            #[cfg(not(unix))]
            let mode = {
                if file_type.is_symlink() {
                    0o120755
                } else if local_st.permissions().readonly() {
                    0o100444
                } else {
                    0o100644
                }
            };

            let is_reg_or_symlink = file_type.is_file() || file_type.is_symlink();
            if !is_reg_or_symlink {
                res.warnings.push(format!(
                    "skipping special file '{src_path}' (mode = 0o{mode:o})"
                ));
                continue;
            }

            let target_dst_str = if dst_isdir {
                let basename = local_path
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or(src_path);
                let dst_trimmed = dst_str.trim_end_matches('/');
                if dst_trimmed.is_empty() {
                    format!("/{basename}")
                } else {
                    format!("{dst_trimmed}/{basename}")
                }
            } else {
                dst_str.clone()
            };

            let mtime = local_st
                .modified()
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
                .duration_since(std::time::SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs() as u32;

            match self
                .sync_send(
                    src_path,
                    &target_dst_str,
                    mtime,
                    mode,
                    file_type.is_symlink(),
                )
                .await
            {
                Ok(bytes) => {
                    res.bytes_transferred += bytes;
                    deferred_acks.push_back((src_path.to_string(), target_dst_str.clone(), bytes));
                    if !self
                        .read_acknowledgements(&mut deferred_acks, &mut res, false)
                        .await
                    {
                        break;
                    }
                }
                Err(err_msg) => {
                    res.errors.push(err_msg.clone());
                    res.success = false;
                    res.outcomes.push(PushSourceOutcome {
                        source: src_path.to_string(),
                        destination: target_dst_str,
                        bytes_transferred: 0,
                        success: false,
                        error: Some(err_msg),
                    });
                    break;
                }
            }
        }

        if !deferred_acks.is_empty() {
            let _ = self
                .read_acknowledgements(&mut deferred_acks, &mut res, true)
                .await;
        }

        Ok(res)
    }

    async fn sync_send(
        &mut self,
        lpath: &str,
        rpath: &str,
        mtime: u32,
        mode: u32,
        is_symlink: bool,
    ) -> Result<u64, String> {
        if is_symlink {
            let link_target =
                std::fs::read_link(lpath).map_err(|e| format!("readlink '{lpath}' failed: {e}"))?;
            let mut contents = link_target.to_string_lossy().as_bytes().to_vec();
            contents.push(0);
            let len = contents.len() as u64;
            self.send_small_file(rpath, mode, lpath, mtime, &contents)
                .await?;
            Ok(len)
        } else {
            let file = std::fs::File::open(lpath)
                .map_err(|e| format!("opening '{lpath}' locally failed: {e}"))?;
            let metadata = file
                .metadata()
                .map_err(|e| format!("cannot stat '{lpath}': {e}"))?;
            let file_size = metadata.len();

            if file_size < SYNC_DATA_MAX as u64 {
                use std::io::Read;
                let mut data = Vec::with_capacity(file_size as usize);
                let mut reader = file;
                reader
                    .read_to_end(&mut data)
                    .map_err(|e| format!("reading '{lpath}' locally failed: {e}"))?;
                self.send_small_file(rpath, mode, lpath, mtime, &data)
                    .await?;
                Ok(file_size)
            } else {
                self.send_large_file(rpath, mode, lpath, mtime, file)
                    .await?;
                Ok(file_size)
            }
        }
    }

    async fn send_small_file(
        &mut self,
        rpath: &str,
        mode: u32,
        lpath: &str,
        mtime: u32,
        data: &[u8],
    ) -> Result<(), String> {
        let path_and_mode = format!("{rpath},{mode}");
        if path_and_mode.len() > 1024 {
            return Err(format!(
                "SendRequest failed: path too long: {}",
                path_and_mode.len()
            ));
        }

        let mut buf = Vec::with_capacity(8 + path_and_mode.len() + 8 + data.len() + 8);
        buf.extend_from_slice(&ID_SEND_V1.to_le_bytes());
        buf.extend_from_slice(&(path_and_mode.len() as u32).to_le_bytes());
        buf.extend_from_slice(path_and_mode.as_bytes());

        buf.extend_from_slice(&ID_DATA.to_le_bytes());
        buf.extend_from_slice(&(data.len() as u32).to_le_bytes());
        buf.extend_from_slice(data);

        buf.extend_from_slice(&ID_DONE.to_le_bytes());
        buf.extend_from_slice(&mtime.to_le_bytes());

        self.write_or_fail(lpath, rpath, &buf).await
    }

    async fn send_large_file(
        &mut self,
        rpath: &str,
        mode: u32,
        lpath: &str,
        mtime: u32,
        mut file: std::fs::File,
    ) -> Result<(), String> {
        if self.have_sendrecv_v2 {
            if rpath.len() > 1024 {
                return Err(format!(
                    "SendRequest failed: path too long: {}",
                    rpath.len()
                ));
            }
            let mut hdr_buf = Vec::with_capacity(8 + rpath.len() + 12);
            hdr_buf.extend_from_slice(&ID_SEND_V2.to_le_bytes());
            hdr_buf.extend_from_slice(&(rpath.len() as u32).to_le_bytes());
            hdr_buf.extend_from_slice(rpath.as_bytes());

            hdr_buf.extend_from_slice(&ID_SEND_V2.to_le_bytes());
            hdr_buf.extend_from_slice(&mode.to_le_bytes());
            hdr_buf.extend_from_slice(&0u32.to_le_bytes()); // flags = 0

            self.write_or_fail(lpath, rpath, &hdr_buf).await?;
        } else {
            let path_and_mode = format!("{rpath},{mode}");
            if path_and_mode.len() > 1024 {
                return Err(format!(
                    "SendRequest failed: path too long: {}",
                    path_and_mode.len()
                ));
            }
            let mut hdr_buf = Vec::with_capacity(8 + path_and_mode.len());
            hdr_buf.extend_from_slice(&ID_SEND_V1.to_le_bytes());
            hdr_buf.extend_from_slice(&(path_and_mode.len() as u32).to_le_bytes());
            hdr_buf.extend_from_slice(path_and_mode.as_bytes());

            self.write_or_fail(lpath, rpath, &hdr_buf).await?;
        }

        use std::io::Read;
        let mut chunk_buf = vec![0u8; SYNC_DATA_MAX];
        loop {
            let n = match file.read(&mut chunk_buf) {
                Ok(0) => break,
                Ok(n) => n,
                Err(e) => return Err(format!("reading '{lpath}' locally failed: {e}")),
            };

            let mut data_hdr = Vec::with_capacity(8 + n);
            data_hdr.extend_from_slice(&ID_DATA.to_le_bytes());
            data_hdr.extend_from_slice(&(n as u32).to_le_bytes());
            data_hdr.extend_from_slice(&chunk_buf[..n]);

            self.write_or_fail(lpath, rpath, &data_hdr).await?;
        }

        let mut done_hdr = [0u8; 8];
        done_hdr[0..4].copy_from_slice(&ID_DONE.to_le_bytes());
        done_hdr[4..8].copy_from_slice(&mtime.to_le_bytes());

        self.write_or_fail(lpath, rpath, &done_hdr).await
    }

    async fn write_or_fail(&mut self, lpath: &str, rpath: &str, data: &[u8]) -> Result<(), String> {
        let stream = self.session.stream_mut();
        if let Err(e) = stream.write_all(data).await {
            if e.kind() == std::io::ErrorKind::ConnectionReset {
                let mut status_buf = [0u8; 8];
                if stream.read_exact(&mut status_buf).await.is_ok() {
                    let id = u32::from_le_bytes(status_buf[0..4].try_into().unwrap());
                    let msglen = u32::from_le_bytes(status_buf[4..8].try_into().unwrap()) as usize;
                    if id == ID_FAIL && msglen <= SYNC_DATA_MAX {
                        let mut fail_buf = vec![0u8; msglen];
                        if stream.read_exact(&mut fail_buf).await.is_ok() {
                            let fail_msg = String::from_utf8_lossy(&fail_buf);
                            return Err(format!(
                                "failed to copy '{lpath}' to '{rpath}': remote {fail_msg}"
                            ));
                        }
                    }
                }
            }
            return Err(format!("failed to copy '{lpath}' to '{rpath}': {e}"));
        }
        let _ = stream.flush().await;
        Ok(())
    }

    async fn read_acknowledgements(
        &mut self,
        deferred_acks: &mut VecDeque<(String, String, u64)>,
        res: &mut PushResult,
        read_all: bool,
    ) -> bool {
        const MAX_DEFERRED_ACKS: usize = 128;
        let mut overall_ack_success = true;

        while !deferred_acks.is_empty() {
            let should_block = read_all || deferred_acks.len() >= MAX_DEFERRED_ACKS;

            let stream = self.session.stream_mut();

            if !should_block {
                let mut test_buf = [0u8; 1];
                match tokio::time::timeout(std::time::Duration::ZERO, stream.peek(&mut test_buf))
                    .await
                {
                    Ok(Ok(0)) | Err(_) => {
                        return overall_ack_success;
                    }
                    Ok(Err(_)) => {}
                    Ok(Ok(_)) => {}
                }
            }

            let mut hdr_buf = [0u8; 8];
            if stream.read_exact(&mut hdr_buf).await.is_err() {
                let err_msg = "failed to read copy response: EOF".to_string();
                res.errors.push(err_msg.clone());
                res.success = false;
                while let Some((from, to, bytes)) = deferred_acks.pop_front() {
                    res.outcomes.push(PushSourceOutcome {
                        source: from,
                        destination: to,
                        bytes_transferred: bytes,
                        success: false,
                        error: Some(err_msg.clone()),
                    });
                }
                return false;
            }

            let id = u32::from_le_bytes(hdr_buf[0..4].try_into().unwrap());
            let msglen = u32::from_le_bytes(hdr_buf[4..8].try_into().unwrap()) as usize;

            if id == ID_OKAY {
                let (from, to, bytes) = deferred_acks.pop_front().unwrap();
                if msglen != 0 {
                    let err_msg = format!("received ID_OKAY with msg_len ({msglen} != 0)");
                    res.errors.push(err_msg.clone());
                    res.success = false;
                    overall_ack_success = false;
                    res.outcomes.push(PushSourceOutcome {
                        source: from,
                        destination: to,
                        bytes_transferred: bytes,
                        success: false,
                        error: Some(err_msg),
                    });
                } else {
                    res.outcomes.push(PushSourceOutcome {
                        source: from,
                        destination: to,
                        bytes_transferred: bytes,
                        success: true,
                        error: None,
                    });
                }
            } else if id == ID_FAIL {
                if msglen > SYNC_DATA_MAX {
                    let (from, to, bytes) = deferred_acks.pop_front().unwrap();
                    let err_msg = format!("too-long message length from daemon: msglen = {msglen}");
                    res.errors.push(err_msg.clone());
                    res.success = false;
                    overall_ack_success = false;
                    res.outcomes.push(PushSourceOutcome {
                        source: from,
                        destination: to,
                        bytes_transferred: bytes,
                        success: false,
                        error: Some(err_msg),
                    });
                } else {
                    let mut fail_buf = vec![0u8; msglen];
                    if stream.read_exact(&mut fail_buf).await.is_err() {
                        let err_msg = "failed to read copy response: EOF".to_string();
                        res.errors.push(err_msg.clone());
                        res.success = false;
                        while let Some((from, to, bytes)) = deferred_acks.pop_front() {
                            res.outcomes.push(PushSourceOutcome {
                                source: from,
                                destination: to,
                                bytes_transferred: bytes,
                                success: false,
                                error: Some(err_msg.clone()),
                            });
                        }
                        return false;
                    }

                    let fail_msg = String::from_utf8_lossy(&fail_buf);
                    let (from, to, bytes) = deferred_acks.pop_front().unwrap();
                    let err_msg = format!("failed to copy '{from}' to '{to}': remote {fail_msg}");
                    res.errors.push(err_msg.clone());
                    res.success = false;
                    overall_ack_success = false;
                    res.outcomes.push(PushSourceOutcome {
                        source: from,
                        destination: to,
                        bytes_transferred: bytes,
                        success: false,
                        error: Some(err_msg),
                    });
                }
            } else {
                let (from, to, bytes) = deferred_acks.pop_front().unwrap();
                let err_msg = format!("unexpected response from daemon: id = 0x{id:08x}");
                res.errors.push(err_msg.clone());
                res.success = false;
                overall_ack_success = false;
                res.outcomes.push(PushSourceOutcome {
                    source: from,
                    destination: to,
                    bytes_transferred: bytes,
                    success: false,
                    error: Some(err_msg),
                });
            }
        }
        overall_ack_success
    }

    /// Sends `QUIT` request to close sync session gracefully.
    pub async fn quit(&mut self) -> Result<(), SyncError> {
        self.send_request(ID_QUIT, "").await
    }
}

fn set_time_and_mode(path: &std::path::Path, time: u64, mode: u32) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt;

        let times = libc::utimbuf {
            actime: time as _,
            modtime: time as _,
        };
        let c_path = CString::new(path.as_os_str().as_bytes()).map_err(|_| {
            std::io::Error::new(std::io::ErrorKind::InvalidInput, "path contains null byte")
        })?;
        let r1 = unsafe { libc::utime(c_path.as_ptr(), &times) };

        let perm_bits = (mode & 0o7777) as libc::mode_t;
        let r2 = unsafe { libc::chmod(c_path.as_ptr(), perm_bits) };

        if r1 != 0 {
            return Err(std::io::Error::last_os_error());
        }
        if r2 != 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(())
    }

    #[cfg(not(unix))]
    {
        use std::time::{Duration, UNIX_EPOCH};
        let file = std::fs::File::options().write(true).open(path)?;
        let st = UNIX_EPOCH + Duration::from_secs(time);
        let times = std::fs::FileTimes::new().set_modified(st).set_accessed(st);
        file.set_times(times)?;
        let mut perms = file.metadata()?.permissions();
        if (mode & 0o200) == 0 {
            perms.set_readonly(true);
            std::fs::set_permissions(path, perms)?;
        }
        Ok(())
    }
}
