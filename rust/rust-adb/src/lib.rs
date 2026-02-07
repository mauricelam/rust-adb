use adb_io::{read_protocol_string, send_protocol_string, read_orderly_shutdown};
pub use adb_protocol::TransportType;
pub use adb_socket_spec::NativeOwnedHandle;
use adb_socket_spec::socket_spec_connect;
use std::io::{Read, Write};
use std::sync::{Mutex, OnceLock};
use thiserror::Error;

#[cfg(unix)]
use std::os::unix::io::{AsRawFd, FromRawFd, IntoRawFd};
#[cfg(windows)]
use std::os::windows::io::{AsRawSocket, FromRawSocket, IntoRawSocket};

#[derive(Error, Debug)]
pub enum AdbClientError {
    #[error("Protocol fault: {0}")]
    ProtocolFault(String),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Socket spec error: {0}")]
    SocketSpec(String),
    #[error("Service failed: {0}")]
    ServiceFailed(String),
}

pub type Result<T> = std::result::Result<T, AdbClientError>;

static G_ADB_TRANSPORT: Mutex<TransportType> = Mutex::new(TransportType::Any);
static G_ADB_SERIAL: Mutex<Option<String>> = Mutex::new(None);
static G_ADB_TRANSPORT_ID: Mutex<u64> = Mutex::new(0);
static G_ADB_SERVER_SOCKET_SPEC: OnceLock<String> = OnceLock::new();

pub fn adb_set_transport(transport_type: TransportType, serial: Option<String>, transport_id: u64) {
    *G_ADB_TRANSPORT.lock().unwrap() = transport_type;
    *G_ADB_SERIAL.lock().unwrap() = serial;
    *G_ADB_TRANSPORT_ID.lock().unwrap() = transport_id;
}

pub fn adb_get_transport() -> (TransportType, Option<String>, u64) {
    (
        *G_ADB_TRANSPORT.lock().unwrap(),
        G_ADB_SERIAL.lock().unwrap().clone(),
        *G_ADB_TRANSPORT_ID.lock().unwrap(),
    )
}

pub fn adb_set_socket_spec(spec: String) {
    let _ = G_ADB_SERVER_SOCKET_SPEC.set(spec);
}

pub fn adb_status<R: Read>(mut reader: R) -> Result<()> {
    let mut buf = [0u8; 4];
    reader.read_exact(&mut buf)?;

    if &buf == b"OKAY" {
        return Ok(());
    }

    if &buf != b"FAIL" {
        return Err(AdbClientError::ProtocolFault(format!(
            "status {:02x} {:02x} {:02x} {:02x}",
            buf[0], buf[1], buf[2], buf[3]
        )));
    }

    let error = read_protocol_string(&mut reader).map_err(|e| AdbClientError::ProtocolFault(e.to_string()))?;
    Err(AdbClientError::ServiceFailed(error))
}

fn switch_socket_transport<RW: Read + Write>(mut rw: RW) -> Result<u64> {
    let transport_id = *G_ADB_TRANSPORT_ID.lock().unwrap();
    let serial = G_ADB_SERIAL.lock().unwrap().clone();
    let transport_type = *G_ADB_TRANSPORT.lock().unwrap();

    let mut read_transport = true;

    let service = if transport_id != 0 {
        read_transport = false;
        format!("host:transport-id:{}", transport_id)
    } else if let Some(s) = serial {
        format!("host:tport:serial:{}", s)
    } else {
        let t_type = match transport_type {
            TransportType::Usb => "usb",
            TransportType::Local => "local",
            TransportType::Any => "any",
            TransportType::Host => return Ok(0), // No switch necessary
        };
        format!("host:tport:{}", t_type)
    };

    send_protocol_string(&mut rw, &service).map_err(AdbClientError::Io)?;
    adb_status(&mut rw)?;

    let mut result = transport_id;
    if read_transport {
        let mut buf = [0u8; 8];
        rw.read_exact(&mut buf)?;
        result = u64::from_le_bytes(buf);
    }

    Ok(result)
}

pub fn adb_connect(service: &str, force_switch: bool) -> Result<(NativeOwnedHandle, u64)> {
    let spec = G_ADB_SERVER_SOCKET_SPEC
        .get()
        .cloned()
        .unwrap_or_else(|| "tcp:5037".to_string());

    let fd = socket_spec_connect(&spec, None, None).map_err(AdbClientError::SocketSpec)?;

    let mut transport_id = 0;
    if !service.starts_with("host") || force_switch {
        #[cfg(unix)]
        let mut stream = unsafe { std::fs::File::from_raw_fd(fd.as_raw_fd()) };
        #[cfg(windows)]
        let mut stream = unsafe { std::net::TcpStream::from_raw_socket(fd.as_raw_socket() as _) };

        transport_id = switch_socket_transport(&mut stream)?;
        std::mem::forget(stream);
    }

    #[cfg(unix)]
    let mut stream = unsafe { std::fs::File::from_raw_fd(fd.as_raw_fd()) };
    #[cfg(windows)]
    let mut stream = unsafe { std::net::TcpStream::from_raw_socket(fd.as_raw_socket() as _) };

    send_protocol_string(&mut stream, service).map_err(AdbClientError::Io)?;
    adb_status(&mut stream)?;

    std::mem::forget(stream);

    Ok((fd, transport_id))
}

pub fn adb_query(service: &str) -> Result<String> {
    let (fd, _) = adb_connect(service, false)?;

    #[cfg(unix)]
    let mut stream = unsafe { std::fs::File::from_raw_fd(fd.into_raw_fd()) };
    #[cfg(windows)]
    let mut stream = unsafe { std::net::TcpStream::from_raw_socket(fd.into_raw_socket() as _) };

    let result = read_protocol_string(&mut stream).map_err(|e| AdbClientError::ProtocolFault(e.to_string()))?;

    let _ = read_orderly_shutdown(&mut stream);

    Ok(result)
}

pub fn format_host_command(command: &str) -> String {
    let transport_id = *G_ADB_TRANSPORT_ID.lock().unwrap();
    let serial = G_ADB_SERIAL.lock().unwrap().clone();
    let transport_type = *G_ADB_TRANSPORT.lock().unwrap();

    if transport_id != 0 {
        return format!("host-transport-id:{}:{}", transport_id, command);
    } else if let Some(s) = serial {
        return format!("host-serial:{}:{}", s, command);
    }

    let prefix = match transport_type {
        TransportType::Usb => "host-usb",
        TransportType::Local => "host-local",
        TransportType::Any => "host",
        TransportType::Host => "host",
    };
    format!("{}:{}", prefix, command)
}
