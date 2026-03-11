use std::fs::File;
use std::io::{Read, Write, Seek, SeekFrom};
use anyhow::{Result, anyhow, Context};
use crate::adb_client::{adb_connect, adb_status};
use adb_io::{read_exactly, write_exactly};
use adb_services::{K_MINADBD_SERVICES_EXIT_SUCCESS, K_MINADBD_SERVICES_EXIT_FAILURE};
use adb_socket_spec::NativeOwnedHandle;

#[cfg(unix)]
use std::os::unix::io::{FromRawFd, IntoRawFd};
#[cfg(windows)]
use std::os::windows::io::{FromRawSocket, IntoRawSocket};

const CHUNK_SIZE: usize = 64 * 1024;
const SIDELOAD_HOST_BLOCK_SIZE: usize = 64 * 1024;

trait SideloadConnector {
    fn connect(&self, service: &str) -> Result<(Box<dyn ReadWrite + Send>, u64)>;
}

trait ReadWrite: Read + Write {}
impl<T: Read + Write> ReadWrite for T {}

struct AdbSideloadConnector;
impl SideloadConnector for AdbSideloadConnector {
    fn connect(&self, service: &str) -> Result<(Box<dyn ReadWrite + Send>, u64)> {
        let (fd, transport_id) = adb_connect(service, false)?;
        let stream = get_stream(fd)?;
        Ok((Box::new(stream), transport_id))
    }
}

fn get_stream(fd: NativeOwnedHandle) -> std::io::Result<impl Read + Write + Send> {
    #[cfg(unix)]
    {
        Ok(unsafe { std::fs::File::from_raw_fd(fd.into_raw_fd()) })
    }
    #[cfg(windows)]
    {
        Ok(unsafe { std::net::TcpStream::from_raw_socket(fd.into_raw_socket() as _) })
    }
}

/// Connects to the sideload / rescue service on the device and sends over the
/// data in an OTA package.
pub fn adb_sideload_install(filename: &str, rescue_mode: bool) -> Result<()> {
    adb_sideload_install_internal(filename, rescue_mode, &AdbSideloadConnector)
}

fn adb_sideload_install_internal<C: SideloadConnector>(
    filename: &str,
    rescue_mode: bool,
    connector: &C,
) -> Result<()> {
    let mut package_file = File::open(filename)
        .with_context(|| format!("failed to open file {}", filename))?;
    let metadata = package_file.metadata()?;
    let size = metadata.len();

    let service = format!(
        "{}:{}:{}",
        if rescue_mode {
            "rescue-install"
        } else {
            "sideload-host"
        },
        size,
        SIDELOAD_HOST_BLOCK_SIZE
    );

    let connect_result = connector.connect(&service);
    let (mut device_stream, _transport_id) = match connect_result {
        Ok(res) => res,
        Err(e) => {
            if rescue_mode || size > i32::MAX as u64 {
                return Err(anyhow!("sideload connection failed: {}", e));
            }
            eprintln!("adb: sideload connection failed: {}", e);
            eprintln!("adb: trying pre-KitKat sideload method...");
            return adb_sideload_legacy(filename, &mut package_file, size as usize, connector);
        }
    };

    let mut buf = vec![0u8; SIDELOAD_HOST_BLOCK_SIZE + 1];
    let mut xfer: u64 = 0;
    let mut last_percent = -1i32;

    loop {
        if let Err(e) = read_exactly(&mut device_stream, &mut buf[..8]) {
            return Err(anyhow!("failed to read command: {}", e));
        }
        let cmd = std::str::from_utf8(&buf[..8]).unwrap_or("");

        if cmd == K_MINADBD_SERVICES_EXIT_SUCCESS || cmd == K_MINADBD_SERVICES_EXIT_FAILURE {
            println!("\rTotal xfer: {:.2}x{:>width$}",
                xfer as f64 / (if size > 0 { size as f64 } else { 1.0 }),
                "",
                width = filename.len() + 10);
            if cmd == K_MINADBD_SERVICES_EXIT_FAILURE {
                return Err(anyhow!("sideload failed"));
            }
            return Ok(());
        }

        let block: i64 = cmd.parse().map_err(|_| anyhow!("failed to parse block number: {}", cmd))?;
        let offset = block * SIDELOAD_HOST_BLOCK_SIZE as i64;
        if offset < 0 || offset >= size as i64 {
            return Err(anyhow!("failed to read block {} at offset {}, past end {}", block, offset, size));
        }

        let mut to_write = SIDELOAD_HOST_BLOCK_SIZE;
        if (offset as u64 + SIDELOAD_HOST_BLOCK_SIZE as u64) > size {
            to_write = (size - offset as u64) as usize;
        }

        package_file.seek(SeekFrom::Start(offset as u64))?;
        read_exactly(&mut package_file, &mut buf[..to_write])?;

        write_exactly(&mut device_stream, &mut buf[..to_write])?;
        xfer += to_write as u64;

        // For normal OTA packages, we expect to transfer every byte
        // twice, plus a bit of overhead (one read during
        // verification, one read of each byte for installation, plus
        // extra access to things like the zip central directory).
        // This estimate of the completion becomes 100% when we've
        // transferred ~2.13 (=100/47) times the package size.
        let percent = (xfer * 47 / (if size > 0 { size } else { 1 })) as i32;
        if percent != last_percent {
            print!("\rserving: '{}'  (~{}%)    ", filename, percent);
            std::io::stdout().flush()?;
            last_percent = percent;
        }
    }
}

fn adb_sideload_legacy<C: SideloadConnector>(
    filename: &str,
    package_file: &mut File,
    size: usize,
    connector: &C,
) -> Result<()> {
    let service = format!("sideload:{}", size);
    let (mut device_stream, _) = connector.connect(&service)?;

    let mut buf = vec![0u8; CHUNK_SIZE];
    let mut remaining = size;
    package_file.seek(SeekFrom::Start(0))?;

    while remaining > 0 {
        let xfer = std::cmp::min(remaining, CHUNK_SIZE);
        read_exactly(package_file, &mut buf[..xfer])?;
        if let Err(e) = write_exactly(&mut device_stream, &mut buf[..xfer]) {
            // In case of error, try to read status
            let mut err_msg = e.to_string();
            if let Err(status_err) = adb_status(&mut device_stream) {
                err_msg = status_err.to_string();
            }
            return Err(anyhow!("failed to write data: {}", err_msg));
        }
        remaining -= xfer;
        print!("\rsending: '{}' {:4}%    ", filename, (100 * (size - remaining) / size));
        std::io::stdout().flush()?;
    }
    println!();

    adb_status(&mut device_stream).map_err(|e| anyhow!("error response: {}", e))?;

    Ok(())
}

/// Connects to the rescue service on the device and requests a wipe of the
/// userdata partition.
pub fn adb_wipe_devices() -> Result<()> {
    adb_wipe_devices_internal(&AdbSideloadConnector)
}

fn adb_wipe_devices_internal<C: SideloadConnector>(connector: &C) -> Result<()> {
    let msg_size = K_MINADBD_SERVICES_EXIT_SUCCESS.len();
    let service = format!("rescue-wipe:userdata:{}", msg_size);
    let (mut device_stream, _) = connector.connect(&service)?;

    let mut message = vec![0u8; msg_size];
    read_exactly(&mut device_stream, &mut message)?;
    let message = std::str::from_utf8(&message).unwrap_or("");

    if message == K_MINADBD_SERVICES_EXIT_SUCCESS {
        return Ok(());
    }

    if message != K_MINADBD_SERVICES_EXIT_FAILURE {
        eprintln!("adb: got unexpected message from rescue wipe {}", message);
    }
    Err(anyhow!("wipe failed"))
}

/// Connects to the rescue service on the device and requests the value of a
/// property.
pub fn adb_rescue_getprop(prop: Option<&str>) -> Result<()> {
    let service = format!("rescue-getprop:{}", prop.unwrap_or_default());
    let (fd, _) = adb_connect(&service, false)?;
    let mut stream = get_stream(fd)?;
    let mut result = String::new();
    stream.read_to_string(&mut result)?;
    println!("{}", result);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    struct MockConnector {
        device_stream: Mutex<Option<Box<dyn ReadWrite + Send>>>,
    }

    use std::sync::Mutex;

    impl SideloadConnector for MockConnector {
        fn connect(&self, _service: &str) -> Result<(Box<dyn ReadWrite + Send>, u64)> {
            let stream = self.device_stream.lock().unwrap().take().unwrap();
            Ok((stream, 0))
        }
    }

    #[test]
    fn test_adb_sideload_install_host_success() {
        let (s1, s2) = sysdeps::net::adb_socketpair().unwrap();
        // Mock server side
        std::thread::spawn(move || {
            let mut s2 = s2;
            let mut buf = [0u8; SIDELOAD_HOST_BLOCK_SIZE];
            // Protocol doesn't include "OKAY" here because adb_connect already consumed it.
            // Device sends block number
            s2.write_all(b"00000000").unwrap();
            // Read data (10 bytes)
            s2.read_exact(&mut buf[..10]).unwrap();
            assert_eq!(&buf[..10], b"0123456789");
            // Send success
            s2.write_all(b"DONEDONE").unwrap();
        });

        let mut file = NamedTempFile::new().unwrap();
        file.write_all(b"0123456789").unwrap();

        let connector = MockConnector {
            device_stream: Mutex::new(Some(Box::new(s1))),
        };

        adb_sideload_install_internal(file.path().to_str().unwrap(), false, &connector).unwrap();
    }

    #[test]
    fn test_adb_sideload_legacy_success() {
        let (s1, s2) = sysdeps::net::adb_socketpair().unwrap();
        // Mock server side
        std::thread::spawn(move || {
            let mut s2 = s2;
            let mut buf = [0u8; 10];
            s2.read_exact(&mut buf).unwrap();
            assert_eq!(&buf, b"0123456789");
            s2.write_all(b"OKAY").unwrap();
        });

        let mut file = NamedTempFile::new().unwrap();
        file.write_all(b"0123456789").unwrap();
        let mut package_file = File::open(file.path()).unwrap();

        let connector = MockConnector {
            device_stream: Mutex::new(Some(Box::new(s1))),
        };

        adb_sideload_legacy("test.bin", &mut package_file, 10, &connector).unwrap();
    }

    #[test]
    fn test_adb_wipe_devices_success() {
        let (s1, s2) = sysdeps::net::adb_socketpair().unwrap();
        // Mock server side
        std::thread::spawn(move || {
            let mut s2 = s2;
            s2.write_all(b"DONEDONE").unwrap();
        });

        let connector = MockConnector {
            device_stream: Mutex::new(Some(Box::new(s1))),
        };

        adb_wipe_devices_internal(&connector).unwrap();
    }
}
