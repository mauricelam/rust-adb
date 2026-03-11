//! Android Debug Bridge (Rust implementation)
//! Ported from `client/main.cpp` and `commandline.cpp`.

mod adb_client;
mod adb_install;
mod bugreport;
mod forward;
mod logcat;
/// Root and unroot command implementation.
pub mod root;
mod sideload;

use clap::{Parser, Subcommand};
use adb_protocol::TransportType;
use crate::adb_client::{adb_set_transport, adb_set_socket_spec, adb_query, adb_connect, format_host_command, adb_remount};
use std::io::{self, Read, Write};

#[derive(Parser)]
#[command(author, version, about = "Android Debug Bridge (Rust)", long_about = None)]
struct Cli {
    /// use USB device (error if multiple devices connected)
    #[arg(short = 'd')]
    usb: bool,

    /// use TCP/IP device (error if multiple TCP/IP devices available)
    #[arg(short = 'e')]
    tcp: bool,

    /// use device with given serial (overrides $ANDROID_SERIAL)
    #[arg(short = 's')]
    serial: Option<String>,

    /// use device with given transport id
    #[arg(short = 't')]
    transport_id: Option<u64>,

    /// name of adb server host [default=localhost]
    #[arg(short = 'H')]
    host: Option<String>,

    /// port of adb server [default=5037]
    #[arg(short = 'P')]
    port: Option<u16>,

    /// listen on given socket for adb server [default=tcp:localhost:5037]
    #[arg(short = 'L')]
    socket: Option<String>,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// list connected devices (-l for long output)
    Devices {
        /// long output
        #[arg(short = 'l')]
        long: bool,
    },
    /// show version num
    Version,
    /// connect to a device via TCP/IP [default port=5555]
    Connect {
        /// HOST[:PORT]
        host: String,
    },
    /// disconnect from given TCP/IP device [default port=5555], or all
    Disconnect {
        /// [HOST[:PORT]]
        host: Option<String>,
    },
    /// run remote shell command (interactive shell if no command given)
    Shell {
        /// COMMAND
        command: Vec<String>,
    },
    /// generate bugreport
    Bugreport {
        /// [PATH] | [--stream]
        args: Vec<String>,
    },
    /// remount partitions read-write. if a reboot is required, -R will
    Remount {
        /// reboot if required
        #[arg(short = 'R')]
        reboot: bool,
    /// show device log (logcat --help for more)
    Logcat {
        /// [ARGS]
        args: Vec<String>,
    },
    /// show device log
    Longcat {
        /// [ARGS]
        args: Vec<String>,
    },
    /// push a single package to the device and install it
    Install {
        /// [ARGS] APK
        args: Vec<String>,
    },
    /// remove a client package from the device
    Uninstall {
        /// [ARGS] PACKAGE
        args: Vec<String>,
    },
    /// sideload the given full OTA package
    Sideload {
        /// OTAPACKAGE
        filename: String,
    },
    /// rescue commands
    Rescue {
        #[command(subcommand)]
        command: RescueCommands,
    },
    /// forward socket connections
    Forward {
        /// [ARGS]
        args: Vec<String>,
    },
    /// reverse socket connections
    Reverse {
        /// [ARGS]
        args: Vec<String>,
    },
    /// restart adbd with root permissions
    Root,
    /// restart adbd without root permissions
    Unroot,
}

#[derive(Subcommand)]
enum RescueCommands {
    /// getprop [prop]
    Getprop {
        /// property name
        prop: Option<String>,
    },
    /// install <filename>
    Install {
        /// OTA package filename
        filename: String,
    },
    /// wipe userdata
    Wipe {
        /// wipe target (must be userdata)
        target: String,
    },
}

fn main() -> anyhow::Result<()> {
    env_logger::init();
    let cli = Cli::parse();

    let mut transport_type = TransportType::Any;
    if cli.usb {
        transport_type = TransportType::Usb;
    } else if cli.tcp {
        transport_type = TransportType::Local;
    }

    let serial = cli.serial.or_else(|| std::env::var("ANDROID_SERIAL").ok());

    adb_set_transport(transport_type, serial, cli.transport_id.unwrap_or(0));

    let socket_spec = if let Some(s) = cli.socket {
        s
    } else {
        let host = cli.host.or_else(|| std::env::var("ANDROID_ADB_SERVER_ADDRESS").ok()).unwrap_or_else(|| "localhost".to_string());
        let port = cli.port.or_else(|| std::env::var("ANDROID_ADB_SERVER_PORT").ok().and_then(|p| p.parse().ok())).unwrap_or(5037);
        format!("tcp:{}:{}", host, port)
    };
    adb_set_socket_spec(socket_spec);

    match cli.command {
        Commands::Devices { long } => {
            let query = if long { "host:devices-l" } else { "host:devices" };
            let result = adb_query(query)?;
            println!("List of devices attached");
            print!("{}", result);
        }
        Commands::Version => {
            println!("Android Debug Bridge version 1.0.41 (Rust)");
        }
        Commands::Connect { host } => {
            let query = format!("host:connect:{}", host);
            let result = adb_query(&query)?;
            println!("{}", result);
        }
        Commands::Disconnect { host } => {
            let query = format!("host:disconnect:{}", host.unwrap_or_default());
            let result = adb_query(&query)?;
            println!("{}", result);
        }
        Commands::Shell { command } => {
            let shell_command = if command.is_empty() {
                "".to_string()
            } else {
                command.join(" ")
            };
            let service = format_host_command(&format!("shell:{}", shell_command));
            let (fd, _) = adb_connect(&service, false)?;

            #[cfg(unix)]
            {
                use std::os::fd::FromRawFd;
                let mut stream = unsafe { std::fs::File::from_raw_fd(fd.into_raw_fd()) };
                let mut stdout = io::stdout();
                let mut buf = [0u8; 4096];
                loop {
                    let n = stream.read(&mut buf)?;
                    if n == 0 { break; }
                    stdout.write_all(&buf[..n])?;
                    stdout.flush()?;
                }
            }
            #[cfg(windows)]
            {
                use std::os::windows::io::FromRawSocket;
                let mut stream = unsafe { std::net::TcpStream::from_raw_socket(fd.into_raw_socket() as _) };
                let mut stdout = io::stdout();
                let mut buf = [0u8; 4096];
                loop {
                    let n = stream.read(&mut buf)?;
                    if n == 0 { break; }
                    stdout.write_all(&buf[..n])?;
                    stdout.flush()?;
                }
            }
        }
        Commands::Bugreport { args } => {
            let mut br = bugreport::Bugreport::new();
            br.do_it(&args)?;
        }
        Commands::Remount { reboot } => {
            let fd = adb_remount(reboot)?;
            #[cfg(unix)]
            {
                use std::os::unix::io::FromRawFd;
                let mut stream = unsafe { std::fs::File::from_raw_fd(fd.into_raw_fd()) };
                let mut stdout = io::stdout();
                let mut buf = [0u8; 4096];
                loop {
                    let n = stream.read(&mut buf)?;
                    if n == 0 { break; }
                    stdout.write_all(&buf[..n])?;
                    stdout.flush()?;
                }
            }
            #[cfg(windows)]
            {
                use std::os::windows::io::FromRawSocket;
                let mut stream = unsafe { std::net::TcpStream::from_raw_socket(fd.into_raw_socket() as _) };
                let mut stdout = io::stdout();
                let mut buf = [0u8; 4096];
                loop {
                    let n = stream.read(&mut buf)?;
                    if n == 0 { break; }
                    stdout.write_all(&buf[..n])?;
                    stdout.flush()?;
                }
            }
        }
        Commands::Logcat { args } => {
            logcat::Logcat::do_it(&args, false)?;
        }
        Commands::Longcat { args } => {
            logcat::Logcat::do_it(&args, true)?;
        }
        Commands::Install { args } => {
            adb_install::install_app_streamed(&args)?;
        }
        Commands::Uninstall { args } => {
            adb_install::uninstall_app(&args)?;
        }
        Commands::Sideload { filename } => {
            sideload::adb_sideload_install(&filename, false)?;
        }
        Commands::Rescue { command } => {
            match command {
                RescueCommands::Getprop { prop } => {
                    sideload::adb_rescue_getprop(prop.as_deref())?;
                }
                RescueCommands::Install { filename } => {
                    sideload::adb_sideload_install(&filename, true)?;
                }
                RescueCommands::Wipe { target } => {
                    if target != "userdata" {
                        anyhow::bail!("invalid rescue wipe argument: {}", target);
                    }
                    sideload::adb_wipe_devices()?;
                }
            }
        }
        Commands::Forward { args } => {
            forward::do_forward_reverse(&args, false)?;
        }
        Commands::Reverse { args } => {
            forward::do_forward_reverse(&args, true)?;
        }
        Commands::Root => {
            root::adb_root("root")?;
        }
        Commands::Unroot => {
            root::adb_root("unroot")?;
        }
    }

    Ok(())
}

#[cfg(unix)]
use std::os::unix::io::IntoRawFd;
#[cfg(windows)]
use std::os::windows::io::IntoRawSocket;
