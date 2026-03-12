//! Android Debug Bridge (Rust implementation)
//! Ported from `client/main.cpp` and `commandline.cpp`.

mod adb_client;
mod adb_install;
mod bugreport;
mod forward;
mod logcat;
mod pairing;
mod root;
mod sideload;
mod sync;

use clap::{Parser, Subcommand};
use adb_protocol::TransportType;
use crate::adb_client::{adb_set_transport, adb_set_socket_spec, adb_query, adb_connect, format_host_command, adb_remount, wait_for_device, adb_command};
use crate::forward::ForwardCommand;
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
    },
    /// copy local files/directories to device
    Push {
        /// source files
        srcs: Vec<String>,
        /// destination directory
        dst: String,
    },
    /// copy files/dirs from device
    Pull {
        /// source files
        srcs: Vec<String>,
        /// destination directory
        #[arg(default_value = ".")]
        dst: String,
    },
    /// sync a local build from $ANDROID_PRODUCT_OUT to the device
    Sync {
        /// partition to sync (all, data, odm, oem, product, system, system_ext, vendor)
        partition: Option<String>,
    },
    /// manage port forwarding
    Forward {
        /// list all forward socket connections
        #[arg(long)]
        list: bool,
        /// remove all forward socket connections
        #[arg(long)]
        remove_all: bool,
        /// remove specific forward socket connection
        #[arg(long)]
        remove: Option<String>,
        /// prevent rebinding of existing redirection
        #[arg(long)]
        no_rebind: bool,
        /// [LOCAL] [REMOTE]
        args: Vec<String>,
    },
    /// manage reverse redirections
    Reverse {
        /// list all reverse socket connections
        #[arg(long)]
        list: bool,
        /// remove all reverse socket connections
        #[arg(long)]
        remove_all: bool,
        /// remove specific reverse socket connection
        #[arg(long)]
        remove: Option<String>,
        /// prevent rebinding of existing redirection
        #[arg(long)]
        no_rebind: bool,
        /// [REMOTE] [LOCAL]
        args: Vec<String>,
    },
    /// install a single package
    Install {
        /// package to install
        pkg: String,
        /// additional arguments
        args: Vec<String>,
    },
    /// remove this app package from the device
    Uninstall {
        /// keep the data and cache directories
        #[arg(short = 'k')]
        keep_data: bool,
        /// package to uninstall
        pkg: String,
    },
    /// show device log
    Logcat {
        /// logcat arguments
        args: Vec<String>,
    },
    /// show device log (long format)
    Longcat {
        /// logcat arguments
        args: Vec<String>,
    },
    /// restart adbd with root permissions
    Root,
    /// restart adbd without root permissions
    Unroot,
    /// restart adbd listening on TCP
    Tcpip {
        /// port
        port: i32,
    },
    /// restart adbd listening on USB
    Usb,
    /// wait for device to be in a given state
    #[command(external_subcommand)]
    WaitFor(Vec<String>),
    /// reboot the device
    Reboot {
        /// [bootloader|recovery|sideload|sideload-auto-reboot]
        arg: Option<String>,
    },
    /// sideload the given full OTA package
    Sideload {
        /// OTA package
        filename: String,
    },
    /// rescue commands
    Rescue {
        /// rescue arguments
        args: Vec<String>,
    },
    /// pair with a device for secure TCP/IP communication
    Pair {
        /// HOST[:PORT]
        host: String,
        /// PAIRING CODE
        password: Option<String>,
    },
    /// print <serial-number>
    #[command(name = "get-serialno")]
    GetSerialno,
    /// print <device-path>
    #[command(name = "get-devpath")]
    GetDevpath,
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
        Commands::Push { srcs, dst } => {
            sync::adb_push(&srcs, &dst)?;
        }
        Commands::Pull { srcs, dst } => {
            sync::adb_pull(&srcs, &dst)?;
        }
        Commands::Sync { partition } => {
            sync::adb_sync(partition.as_deref())?;
        }
        Commands::Forward { list, remove_all, remove, no_rebind, args } => {
            let cmd = if list {
                ForwardCommand::List
            } else if remove_all {
                ForwardCommand::RemoveAll
            } else if let Some(local) = remove {
                ForwardCommand::Remove(local)
            } else if args.len() == 2 {
                ForwardCommand::Add(args[0].clone(), args[1].clone(), no_rebind)
            } else {
                anyhow::bail!("invalid forward arguments");
            };
            forward::adb_forward_command(cmd, false)?;
        }
        Commands::Reverse { list, remove_all, remove, no_rebind, args } => {
            let cmd = if list {
                ForwardCommand::List
            } else if remove_all {
                ForwardCommand::RemoveAll
            } else if let Some(remote) = remove {
                ForwardCommand::Remove(remote)
            } else if args.len() == 2 {
                ForwardCommand::Add(args[0].clone(), args[1].clone(), no_rebind)
            } else {
                anyhow::bail!("invalid reverse arguments");
            };
            forward::adb_forward_command(cmd, true)?;
        }
        Commands::Install { pkg, args } => {
            adb_install::adb_install(&pkg, &args)?;
        }
        Commands::Uninstall { keep_data, pkg } => {
            adb_install::adb_uninstall(&pkg, keep_data)?;
        }
        Commands::Logcat { args } => {
            logcat::adb_logcat(&args, false)?;
        }
        Commands::Longcat { args } => {
            logcat::adb_logcat(&args, true)?;
        }
        Commands::Root => {
            root::adb_root("root")?;
        }
        Commands::Unroot => {
            root::adb_root("unroot")?;
        }
        Commands::Tcpip { port } => {
            root::adb_tcpip(port)?;
        }
        Commands::Usb => {
            root::adb_usb()?;
        }
        Commands::WaitFor(args) => {
            if args.is_empty() {
                anyhow::bail!("wait-for requires a state");
            }
            // Clap doesn't support hyphens in subcommands easily with external_subcommand
            // but we can join them if needed. Actually WaitFor is called with ["wait-for-device"] etc.
            wait_for_device(&args[0], None)?;
        }
        Commands::Reboot { arg } => {
            let service = if let Some(a) = arg {
                format!("reboot:{}", a)
            } else {
                "reboot:".to_string()
            };
            adb_command(&format_host_command(&service))?;
        }
        Commands::Sideload { filename } => {
            sideload::adb_sideload(&filename)?;
        }
        Commands::Rescue { args } => {
            sideload::adb_rescue(&args)?;
        }
        Commands::Pair { host, password } => {
            let password = if let Some(p) = password {
                p
            } else {
                print!("Enter pairing code: ");
                io::stdout().flush()?;
                let mut p = String::new();
                io::stdin().read_line(&mut p)?;
                p.trim().to_string()
            };
            pairing::adb_pair(&host, &password)?;
        }
        Commands::GetSerialno => {
            let result = adb_query(&format_host_command("get-serialno"))?;
            println!("{}", result);
        }
        Commands::GetDevpath => {
            let result = adb_query(&format_host_command("get-devpath"))?;
            println!("{}", result);
        }
    }

    Ok(())
}

#[cfg(unix)]
use std::os::unix::io::IntoRawFd;
#[cfg(windows)]
use std::os::windows::io::IntoRawSocket;
