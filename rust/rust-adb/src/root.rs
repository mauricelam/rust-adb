use std::io::Read;
use crate::adb_client::{adb_connect, adb_get_transport, adb_set_transport, wait_for_device, AdbClientError};
use adb_protocol::TransportType;
use adb_socket_spec::NativeOwnedHandle;
use std::time::Duration;

#[cfg(unix)]
use std::os::unix::io::{FromRawFd, IntoRawFd};
#[cfg(windows)]
use std::os::windows::io::{FromRawSocket, IntoRawSocket};

/// Internal trait for root-related operations to allow for mock testing.
pub trait RootInternal {
    /// Connects to the restart service.
    fn connect(&self, command: &str) -> Result<(NativeOwnedHandle, u64), AdbClientError>;
    /// Waits for the device to disconnect and then reconnect.
    fn wait_for_restart(&self, transport_id: u64) -> Result<(), AdbClientError>;
}

struct RootInternalImpl;

impl RootInternal for RootInternalImpl {
    fn connect(&self, command: &str) -> Result<(NativeOwnedHandle, u64), AdbClientError> {
        let service = format!("{}:", command);
        adb_connect(&service, false)
    }

    fn wait_for_restart(&self, transport_id: u64) -> Result<(), AdbClientError> {
        let (prev_type, prev_serial, prev_id) = adb_get_transport();

        adb_set_transport(TransportType::Any, None, transport_id);
        wait_for_device("wait-for-disconnect", None)?;

        if prev_id == 0 {
            adb_set_transport(prev_type, prev_serial, 0);
            wait_for_device("wait-for-device", Some(Duration::from_millis(12000)))?;
        }

        Ok(())
    }
}

/// Restarts adbd with root permissions.
pub fn adb_root(command: &str) -> anyhow::Result<()> {
    adb_root_internal(command, &RootInternalImpl)
}

fn adb_root_internal(command: &str, internal: &dyn RootInternal) -> anyhow::Result<()> {
    let (fd, transport_id) = internal.connect(command)?;

    #[cfg(unix)]
    let mut stream = unsafe { std::fs::File::from_raw_fd(fd.into_raw_fd()) };
    #[cfg(windows)]
    let mut stream = unsafe { std::net::TcpStream::from_raw_socket(fd.into_raw_socket() as _) };

    let mut buf = String::new();
    stream.read_to_string(&mut buf)?;

    print!("{}", buf);

    if buf.contains("restarting") {
        internal.wait_for_restart(transport_id)?;
    }

    Ok(())
}

/// Restarts adbd listening on TCP on the given port.
pub fn adb_tcpip(port: i32) -> anyhow::Result<()> {
    let service = format!("tcpip:{}", port);
    let (fd, _) = adb_connect(&service, false)?;

    #[cfg(unix)]
    let mut stream = unsafe { std::fs::File::from_raw_fd(fd.into_raw_fd()) };
    #[cfg(windows)]
    let mut stream = unsafe { std::net::TcpStream::from_raw_socket(fd.into_raw_socket() as _) };

    let mut buf = String::new();
    stream.read_to_string(&mut buf)?;
    print!("{}", buf);

    Ok(())
}

/// Restarts adbd listening on USB.
pub fn adb_usb() -> anyhow::Result<()> {
    let (fd, _) = adb_connect("usb:", false)?;

    #[cfg(unix)]
    let mut stream = unsafe { std::fs::File::from_raw_fd(fd.into_raw_fd()) };
    #[cfg(windows)]
    let mut stream = unsafe { std::net::TcpStream::from_raw_socket(fd.into_raw_socket() as _) };

    let mut buf = String::new();
    stream.read_to_string(&mut buf)?;
    print!("{}", buf);

    Ok(())
}
