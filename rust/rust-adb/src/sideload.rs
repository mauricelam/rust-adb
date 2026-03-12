use std::io::{Read, Write, self, Seek, SeekFrom};
use crate::adb_client::{adb_connect, format_host_command, adb_command};
use adb_io::read_exactly;

#[cfg(unix)]
use std::os::unix::io::{FromRawFd, IntoRawFd};
#[cfg(windows)]
use std::os::windows::io::{FromRawSocket, IntoRawSocket};

const SIDELOAD_HOST_BLOCK_SIZE: usize = 65536;

/// Sideloads an OTA package.
pub fn adb_sideload(filename: &str) -> anyhow::Result<()> {
    adb_sideload_install(filename, false)
}

/// Rescue commands.
pub fn adb_rescue(args: &[String]) -> anyhow::Result<()> {
    if args.is_empty() {
        anyhow::bail!("rescue requires at least one argument");
    }

    match args[0].as_str() {
        "getprop" => {
            let service = if args.len() == 1 {
                "rescue-getprop:".to_string()
            } else {
                format!("rescue-getprop:{}", args[1])
            };
            adb_command(&format_host_command(&service))?;
            Ok(())
        }
        "install" => {
            if args.len() != 2 {
                anyhow::bail!("rescue install requires two arguments");
            }
            adb_sideload_install(&args[1], true)
        }
        "wipe" => {
            if args.len() != 2 || args[1] != "userdata" {
                anyhow::bail!("invalid rescue wipe arguments");
            }
            let service = format!("rescue-wipe:userdata:{}", "DONEDONE".len());
            let (fd, _) = adb_connect(&format_host_command(&service), false)?;
            #[cfg(unix)]
            let mut stream = unsafe { std::fs::File::from_raw_fd(fd.into_raw_fd()) };
            #[cfg(windows)]
            let mut stream = unsafe { std::net::TcpStream::from_raw_socket(fd.into_raw_socket() as _) };

            let mut buf = vec![0u8; "DONEDONE".len()];
            read_exactly(&mut stream, &mut buf)?;
            let msg = String::from_utf8_lossy(&buf);
            if msg == "DONEDONE" {
                Ok(())
            } else {
                anyhow::bail!("rescue wipe failed: {}", msg)
            }
        }
        _ => anyhow::bail!("unknown rescue argument"),
    }
}

fn adb_sideload_install(filename: &str, rescue_mode: bool) -> anyhow::Result<()> {
    let mut file = std::fs::File::open(filename)?;
    let size = file.metadata()?.len();

    let service = if rescue_mode {
        format!("rescue-install:{}:{}", size, SIDELOAD_HOST_BLOCK_SIZE)
    } else {
        format!("sideload-host:{}:{}", size, SIDELOAD_HOST_BLOCK_SIZE)
    };

    let (fd, _) = adb_connect(&format_host_command(&service), false)?;
    #[cfg(unix)]
    let mut stream = unsafe { std::fs::File::from_raw_fd(fd.into_raw_fd()) };
    #[cfg(windows)]
    let mut stream = unsafe { std::net::TcpStream::from_raw_socket(fd.into_raw_socket() as _) };

    let mut buf = [0u8; SIDELOAD_HOST_BLOCK_SIZE];
    let mut xfer = 0u64;
    let mut last_percent = -1;

    loop {
        let mut cmd_buf = [0u8; 8];
        read_exactly(&mut stream, &mut cmd_buf)?;
        let cmd = String::from_utf8_lossy(&cmd_buf);

        if cmd == "DONEDONE" {
            println!("\rserving: '{}'  (100%)    ", filename);
            return Ok(());
        }
        if cmd == "FAILFAIL" {
            anyhow::bail!("sideload failed");
        }

        let block: i64 = cmd.trim().parse()?;
        let offset = block as u64 * SIDELOAD_HOST_BLOCK_SIZE as u64;

        if offset >= size {
            anyhow::bail!("invalid block {} requested", block);
        }

        let mut to_write = SIDELOAD_HOST_BLOCK_SIZE as u64;
        if offset + to_write > size {
            to_write = size - offset;
        }

        file.seek(SeekFrom::Start(offset))?;
        file.read_exact(&mut buf[..to_write as usize])?;
        stream.write_all(&buf[..to_write as usize])?;

        xfer += to_write;
        let percent = (xfer * 100 / size) as i32;
        if percent != last_percent {
            print!("\rserving: '{}'  (~{}%)    ", filename, percent);
            io::stdout().flush()?;
            last_percent = percent;
        }
    }
}
