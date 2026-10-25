use crate::client::AdbClient;
use crate::session::{Session, SessionError};
use crate::transport::TransportSelector;
use adb_proto::sync::{
    ID_DENT_V1, ID_DENT_V2, ID_DONE, ID_LIST_V1, ID_LIST_V2, ID_LSTAT_V1, ID_LSTAT_V2, ID_QUIT,
    ID_STAT_V2,
};
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

/// An active ADB sync connection session supporting `ls`, `stat`, `lstat`, and `stat_with_fallback`.
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

    /// Sends `QUIT` request to close sync session gracefully.
    pub async fn quit(&mut self) -> Result<(), SyncError> {
        self.send_request(ID_QUIT, "").await
    }
}
