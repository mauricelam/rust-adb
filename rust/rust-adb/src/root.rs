use crate::adb_client::{adb_connect, adb_get_transport, adb_set_transport, format_host_command};
use adb_protocol::TransportType;
use adb_socket_spec::NativeOwnedHandle;
use anyhow::Result;
use std::io::{self, Read, Write};

/// Root and unroot command implementation.
pub struct Root;

impl Root {
    /// Executes the root or unroot command.
    pub fn adb_root(command: &str) -> Result<()> {
        let mut real = RootReal {};
        Self::adb_root_internal(&mut real, command)
    }

    /// Internal implementation of the root or unroot command.
    pub fn adb_root_internal<T: RootInternal>(internal: &mut T, command: &str) -> Result<()> {
        let service = format!("{}:", command);
        let (fd, transport_id) = internal.adb_connect(&service)?;
        let mut stream = internal.stream_from_fd(fd);

        let mut buf = [0u8; 256];
        let mut total_read = 0;
        loop {
            match stream.read(&mut buf[total_read..]) {
                Ok(0) => break,
                Ok(n) => {
                    total_read += n;
                    if total_read >= buf.len() {
                        break;
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e.into()),
            }
        }

        let output = String::from_utf8_lossy(&buf[..total_read]);
        internal.print(&output)?;

        if output.contains("restarting") {
            let (previous_type, previous_serial, previous_id) = internal.adb_get_transport();

            internal.adb_set_transport(TransportType::Any, None, transport_id);
            let _ = internal.wait_for_device("wait-for-disconnect");

            if previous_id == 0 {
                internal.adb_set_transport(previous_type, previous_serial, 0);
                let _ = internal.wait_for_device("wait-for-device");
            }
        }

        Ok(())
    }
}

/// Internal trait for root operations, allowing for mocking in tests.
pub trait RootInternal {
    /// Connects to the ADB server and executes a service.
    fn adb_connect(&mut self, service: &str) -> Result<(NativeOwnedHandle, u64)>;
    /// Returns the currently configured transport.
    fn adb_get_transport(&self) -> (TransportType, Option<String>, u64);
    /// Sets the transport to be used for subsequent ADB commands.
    fn adb_set_transport(
        &mut self,
        transport_type: TransportType,
        serial: Option<String>,
        transport_id: u64,
    );
    /// Waits for a device to reach a certain state.
    fn wait_for_device(&mut self, service: &str) -> Result<()>;
    /// Creates a stream from a file descriptor.
    fn stream_from_fd(&self, fd: NativeOwnedHandle) -> Box<dyn ReadWrite>;
    /// Prints a message to the user.
    fn print(&mut self, message: &str) -> Result<()>;
}

/// Trait combining Read and Write.
pub trait ReadWrite: Read + Write {}
impl<T: Read + Write> ReadWrite for T {}

struct RootReal;

impl RootInternal for RootReal {
    fn adb_connect(&mut self, service: &str) -> Result<(NativeOwnedHandle, u64)> {
        adb_connect(service, false).map_err(Into::into)
    }

    fn adb_get_transport(&self) -> (TransportType, Option<String>, u64) {
        adb_get_transport()
    }

    fn adb_set_transport(
        &mut self,
        transport_type: TransportType,
        serial: Option<String>,
        transport_id: u64,
    ) {
        adb_set_transport(transport_type, serial, transport_id);
    }

    fn wait_for_device(&mut self, service: &str) -> Result<()> {
        let cmd = format_host_command(service);
        let _ = adb_connect(&cmd, false)?;
        Ok(())
    }

    fn stream_from_fd(&self, fd: NativeOwnedHandle) -> Box<dyn ReadWrite> {
        #[cfg(unix)]
        {
            use std::os::unix::io::{FromRawFd, IntoRawFd};
            Box::new(unsafe { std::fs::File::from_raw_fd(fd.into_raw_fd()) })
        }
        #[cfg(windows)]
        {
            use std::os::windows::io::{FromRawSocket, IntoRawSocket};
            Box::new(unsafe { std::net::TcpStream::from_raw_socket(fd.into_raw_socket() as _) })
        }
    }

    fn print(&mut self, message: &str) -> Result<()> {
        print!("{}", message);
        io::stdout().flush()?;
        Ok(())
    }
}

/// Root or unroot the device.
pub fn adb_root(command: &str) -> Result<()> {
    Root::adb_root(command)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    struct MockRoot {
        output: Vec<u8>,
        transport_type: TransportType,
        serial: Option<String>,
        transport_id: u64,
        calls: Arc<Mutex<Vec<String>>>,
    }

    impl RootInternal for MockRoot {
        fn adb_connect(&mut self, service: &str) -> Result<(NativeOwnedHandle, u64)> {
            self.calls.lock().unwrap().push(format!("connect:{}", service));
            // Return a dummy handle.
            #[cfg(unix)]
            {
                use std::os::unix::io::FromRawFd;
                let fd = unsafe { libc::dup(0) };
                Ok((unsafe { NativeOwnedHandle::from_raw_fd(fd) }, 123))
            }
            #[cfg(windows)]
            {
                use std::os::windows::io::FromRawSocket;
                // On windows we might need something else but for now let's try to find a valid socket handle
                // or just use 0 if it works. But 0 is usually not a valid socket.
                Ok((unsafe { NativeOwnedHandle::from_raw_socket(0) }, 123))
            }
        }

        fn adb_get_transport(&self) -> (TransportType, Option<String>, u64) {
            (self.transport_type, self.serial.clone(), self.transport_id)
        }

        fn adb_set_transport(
            &mut self,
            transport_type: TransportType,
            serial: Option<String>,
            transport_id: u64,
        ) {
            self.calls.lock().unwrap().push(format!(
                "set_transport:{:?}:{:?}:{}",
                transport_type, serial, transport_id
            ));
            self.transport_type = transport_type;
            self.serial = serial;
            self.transport_id = transport_id;
        }

        fn wait_for_device(&mut self, service: &str) -> Result<()> {
            self.calls.lock().unwrap().push(format!("wait:{}", service));
            Ok(())
        }

        fn stream_from_fd(&self, _fd: NativeOwnedHandle) -> Box<dyn ReadWrite> {
            Box::new(io::Cursor::new(self.output.clone()))
        }

        fn print(&mut self, message: &str) -> Result<()> {
            self.calls.lock().unwrap().push(format!("print:{}", message));
            Ok(())
        }
    }

    #[test]
    fn test_adb_root_no_restart() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let mut mock = MockRoot {
            output: b"adbd is already running as root\n".to_vec(),
            transport_type: TransportType::Usb,
            serial: Some("test".to_string()),
            transport_id: 0,
            calls: calls.clone(),
        };

        Root::adb_root_internal(&mut mock, "root").unwrap();

        let calls = calls.lock().unwrap();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0], "connect:root:");
        assert_eq!(calls[1], "print:adbd is already running as root\n");
    }

    #[test]
    fn test_adb_root_restart() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let mut mock = MockRoot {
            output: b"restarting adbd as root\n".to_vec(),
            transport_type: TransportType::Usb,
            serial: Some("test".to_string()),
            transport_id: 0,
            calls: calls.clone(),
        };

        Root::adb_root_internal(&mut mock, "root").unwrap();

        let calls = calls.lock().unwrap();
        assert_eq!(calls.len(), 6);
        assert_eq!(calls[0], "connect:root:");
        assert_eq!(calls[1], "print:restarting adbd as root\n");
        assert_eq!(calls[2], "set_transport:Any:None:123");
        assert_eq!(calls[3], "wait:wait-for-disconnect");
        assert_eq!(calls[4], "set_transport:Usb:Some(\"test\"):0");
        assert_eq!(calls[5], "wait:wait-for-device");
    }

    #[test]
    fn test_adb_unroot_restart() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let mut mock = MockRoot {
            output: b"restarting adbd as non root\n".to_vec(),
            transport_type: TransportType::Usb,
            serial: None,
            transport_id: 0,
            calls: calls.clone(),
        };

        Root::adb_root_internal(&mut mock, "unroot").unwrap();

        let calls = calls.lock().unwrap();
        assert_eq!(calls.len(), 6);
        assert_eq!(calls[0], "connect:unroot:");
        assert_eq!(calls[1], "print:restarting adbd as non root\n");
        assert_eq!(calls[2], "set_transport:Any:None:123");
        assert_eq!(calls[3], "wait:wait-for-disconnect");
        assert_eq!(calls[4], "set_transport:Usb:None:0");
        assert_eq!(calls[5], "wait:wait-for-device");
    }
}
