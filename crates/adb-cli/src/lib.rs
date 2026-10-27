//! Argument parsing and CLI helpers for `adb`.

use adb_client::{
    resolve_server_address, AdbClient, ClientError, DeviceSelection, Outcome, ServerAddress,
    ServerAddressEnv, ServerAddressOptions, TrackDevicesMode, TransportSelector,
};
use std::io::Write;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

pub const HELP_TEXT: &str = "\
Android Debug Bridge version 1.0.41
Version 36.0.2-14143358
Installed as /tmp/adb
Running on Linux

global options:
 -a                       listen on all network interfaces, not just localhost
 -d                       use USB device (error if multiple devices connected)
 -e                       use TCP/IP device (error if multiple TCP/IP devices available)
 -s SERIAL                use device with given serial (overrides $ANDROID_SERIAL)
 -t ID                    use device with given transport id
 -H                       name of adb server host [default=localhost]
 -P                       port of adb server [default=5037]
 -L SOCKET                listen on given socket for adb server [default=tcp:localhost:<port>]
 --one-device SERIAL|USB  only allowed with 'start-server' or 'server nodaemon', server will only connect to one USB device, specified by a serial number or USB device address.
 --exit-on-write-error    exit if stdout is closed

general commands:
 devices [-l]             list connected devices (-l for long output)
 help                     show this help message
 version                  show version num

networking:
 connect HOST[:PORT]      connect to a device via TCP/IP [default port=5555]
 disconnect [HOST[:PORT]]
     disconnect from given TCP/IP device [default port=5555], or all
 pair HOST[:PORT] [PAIRING CODE]
     pair with a device for secure TCP/IP communication
 forward --list           list all forward socket connections
 forward [--no-rebind] LOCAL REMOTE
     forward socket connection using:
       tcp:<port> (<local> may be \"tcp:0\" to pick any open port)
       localabstract:<unix domain socket name>
       localreserved:<unix domain socket name>
       localfilesystem:<unix domain socket name>
       dev:<character device name>
       dev-raw:<character device name> (open device in raw mode)
       jdwp:<process pid> (remote only)
       vsock:<CID>:<port> (remote only)
       acceptfd:<fd> (listen only)
 forward --remove LOCAL   remove specific forward socket connection
 forward --remove-all     remove all forward socket connections
 reverse --list           list all reverse socket connections from device
 reverse [--no-rebind] REMOTE LOCAL
     reverse socket connection using:
       tcp:<port> (<remote> may be \"tcp:0\" to pick any open port)
       localabstract:<unix domain socket name>
       localreserved:<unix domain socket name>
       localfilesystem:<unix domain socket name>
 reverse --remove REMOTE  remove specific reverse socket connection
 reverse --remove-all     remove all reverse socket connections from device
 mdns check               check if mdns discovery is available
 mdns services            list all discovered services

file transfer:
 push [--sync] [-z ALGORITHM] [-Z] LOCAL... REMOTE
     copy local files/directories to device
     -n: dry run: push files to device without storing to the filesystem
     -q: suppress progress messages
     -Z: disable compression
     -z: enable compression with a specified algorithm (any/none/brotli/lz4/zstd)
     --sync: only push files that have different timestamps on the host than the device
 pull [-a] [-z ALGORITHM] [-Z] REMOTE... LOCAL
     copy files/dirs from device
     -a: preserve file timestamp and mode
     -q: suppress progress messages
     -Z: disable compression
     -z: enable compression with a specified algorithm (any/none/brotli/lz4/zstd)
 sync [-l] [-z ALGORITHM] [-Z] [all|data|odm|oem|product|system|system_ext|vendor]
     sync a local build from $ANDROID_PRODUCT_OUT to the device (default all)
     -l: list files that would be copied, but don't copy them
     -n: dry run: push files to device without storing to the filesystem
     -q: suppress progress messages
     -Z: disable compression
     -z: enable compression with a specified algorithm (any/none/brotli/lz4/zstd)

shell:
 shell [-e ESCAPE] [-n] [-Tt] [-x] [COMMAND...]
     run remote shell command (interactive shell if no command given)
     -e: choose escape character, or \"none\"; default '~'
     -n: don't read from stdin
     -T: disable pty allocation
     -t: allocate a pty if on a tty (-tt: force pty allocation)
     -x: disable remote exit codes and stdout/stderr separation
 emu COMMAND              run emulator console command

app installation (see also `adb shell cmd package help`):
 install [-lrtsdg] [--instant] PACKAGE
     push a single package to the device and install it
 install-multiple [-lrtsdpg] [--instant] PACKAGE...
     push multiple APKs to the device for a single package and install them
 install-multi-package [-lrtsdpg] [--instant] PACKAGE...
     push one or more packages to the device and install them atomically
     -r: replace existing application
     -t: allow test packages
     -d: allow version code downgrade (debuggable packages only)
     -p: partial application install (install-multiple only)
     -g: grant all runtime permissions
     --abi ABI: override platform's default ABI
     --instant: cause the app to be installed as an ephemeral install app
     --no-streaming: always push APK to device and invoke Package Manager as separate steps
     --streaming: force streaming APK directly into Package Manager
     --fastdeploy: use fast deploy
     --no-fastdeploy: prevent use of fast deploy
     --force-agent: force update of deployment agent when using fast deploy
     --date-check-agent: update deployment agent when local version is newer and using fast deploy
     --version-check-agent: update deployment agent when local version has different version code and using fast deploy
     (See also `adb shell pm help` for more options.)
 uninstall [-k] PACKAGE
     remove this app package from the device
     '-k': keep the data and cache directories

debugging:
 bugreport [PATH]
     write bugreport to given PATH [default=bugreport.zip];
     if PATH is a directory, the bug report is saved in that directory.
     devices that don't support zipped bug reports output to stdout.
 jdwp                     list pids of processes hosting a JDWP transport
 logcat                   show device log (logcat --help for more)

security:
 disable-verity           disable dm-verity checking on userdebug builds
 enable-verity            re-enable dm-verity checking on userdebug builds
 keygen FILE
     generate adb public/private key; private key stored in FILE,

scripting:
 wait-for[-TRANSPORT]-STATE...
     wait for device to be in a given state
     STATE: device, recovery, rescue, sideload, bootloader, or disconnect
     TRANSPORT: usb, local, or any [default=any]
 get-state                print offline | bootloader | device
 get-serialno             print <serial-number>
 get-devpath              print <device-path>
 remount [-R]
      remount partitions read-write. if a reboot is required, -R will
      will automatically reboot the device.
 reboot [bootloader|recovery|sideload|sideload-auto-reboot]
     reboot the device; defaults to booting system image but
     supports bootloader and recovery too. sideload reboots
     into recovery and automatically starts sideload mode,
     sideload-auto-reboot is the same but reboots after sideloading.
 sideload OTAPACKAGE      sideload the given full OTA package
 root                     restart adbd with root permissions
 unroot                   restart adbd without root permissions
 usb                      restart adbd listening on USB
 tcpip PORT               restart adbd listening on TCP on PORT

internal debugging:
 start-server             ensure that there is a server running
 kill-server              kill the server if it is running
 reconnect                kick connection from host side to force reconnect
 reconnect device         kick connection from device side to force reconnect
 reconnect offline        reset offline/unauthorized devices to force reconnect

usb:
 attach                   attach a detached USB device
 detach                   detach from a USB device to allow use by other processes
environment variables:
 $ADB_TRACE
     comma/space separated list of debug info to log:
     all,adb,sockets,packets,rwx,usb,sync,sysdeps,transport,jdwp,services,auth,fdevent,shell,incremental
 $ADB_VENDOR_KEYS         colon-separated list of keys (files or directories)
 $ANDROID_SERIAL          serial number to connect to (see -s)
 $ANDROID_LOG_TAGS        tags to be used by logcat (see logcat --help)
 $ADB_LOCAL_TRANSPORT_MAX_PORT max emulator scan port (default 5585, 16 emus)
 $ADB_MDNS_AUTO_CONNECT   comma-separated list of mdns services to allow auto-connect (default adb-tls-connect)

Online documentation: https://android.googlesource.com/platform/packages/modules/adb/+/refs/heads/main/docs/user/adb.1.md

";

/// Represents parsed global command-line options.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GlobalOptions {
    pub server_address_options: ServerAddressOptions,
    pub device_selection: DeviceSelection,
    pub server_address: ServerAddress,
    pub listen_all: bool,
    pub one_device: Option<String>,
    pub exit_on_write_error: bool,
}

/// Shared thread-safe byte buffer for test output assertions.
pub type MemoryStream = Arc<Mutex<Vec<u8>>>;

/// Abstract output streams for stdout and stderr injection.
pub struct Output {
    pub stdout: Box<dyn Write + Send>,
    pub stderr: Box<dyn Write + Send>,
}

impl Default for Output {
    fn default() -> Self {
        Self::stdio()
    }
}

impl Output {
    pub fn stdio() -> Self {
        Self {
            stdout: Box::new(std::io::stdout()),
            stderr: Box::new(std::io::stderr()),
        }
    }

    pub fn memory() -> (Self, MemoryStream, MemoryStream) {
        let out_buf = Arc::new(Mutex::new(Vec::new()));
        let err_buf = Arc::new(Mutex::new(Vec::new()));
        let stdout = Box::new(SharedWriter(out_buf.clone()));
        let stderr = Box::new(SharedWriter(err_buf.clone()));
        (Self { stdout, stderr }, out_buf, err_buf)
    }
}

struct SharedWriter(Arc<Mutex<Vec<u8>>>);

impl Write for SharedWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().write(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Parses global CLI options from command-line arguments and environment lookup.
///
/// Ports global-option loop at the start of `adb_commandline` in `original/client/commandline.cpp`.
pub fn parse_global<I, S, E>(args: I, env: E) -> Result<(GlobalOptions, Vec<String>), String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
    E: Fn(&str) -> Option<String>,
{
    let mut server_options = ServerAddressOptions::default();
    let mut serial = None;
    let mut transport_id = None;
    let mut usb = false;
    let mut local = false;
    let mut specified_d_e_s = false;
    let mut listen_all = false;
    let mut one_device = None;
    let mut exit_on_write_error = false;

    let mut args_iter = args.into_iter().map(|s| s.as_ref().to_string()).peekable();
    let mut remaining = Vec::new();

    while let Some(arg) = args_iter.next() {
        if arg == "server" || arg == "nodaemon" || arg == "fork-server" {
            continue;
        } else if arg == "--reply-fd" {
            if args_iter.next().is_none() {
                return Err("--reply-fd requires an argument".to_string());
            }
        } else if arg == "--one-device" {
            if let Some(val) = args_iter.next() {
                one_device = Some(val);
            } else {
                return Err("--one-device requires an argument".to_string());
            }
        } else if arg == "-a" {
            listen_all = true;
        } else if arg == "--exit-on-write-error" {
            exit_on_write_error = true;
        } else if arg == "-d" {
            usb = true;
            local = false;
            specified_d_e_s = true;
        } else if arg == "-e" {
            local = true;
            usb = false;
            specified_d_e_s = true;
        } else if let Some(rest) = arg.strip_prefix("-s") {
            specified_d_e_s = true;
            if !rest.is_empty() {
                if rest.chars().next().unwrap().is_ascii_digit() {
                    serial = Some(rest.to_string());
                } else {
                    return Err("-s requires an argument".to_string());
                }
            } else if let Some(next_arg) = args_iter.next() {
                serial = Some(next_arg);
            } else {
                return Err("-s requires an argument".to_string());
            }
        } else if let Some(rest) = arg.strip_prefix("-t") {
            let id_str = if !rest.is_empty() {
                if rest.chars().next().unwrap().is_ascii_digit() {
                    rest.to_string()
                } else {
                    return Err("-t requires an argument".to_string());
                }
            } else if let Some(next_arg) = args_iter.next() {
                next_arg
            } else {
                return Err("-t requires an argument".to_string());
            };

            if id_str.is_empty() || !id_str.chars().all(|c| c.is_ascii_digit()) {
                return Err("invalid transport id".to_string());
            }
            let id = id_str
                .parse::<u64>()
                .map_err(|_| "invalid transport id".to_string())?;
            transport_id = Some(id);
        } else if let Some(rest) = arg.strip_prefix("-H") {
            if !rest.is_empty() {
                server_options.host = Some(rest.to_string());
            } else if let Some(next_arg) = args_iter.next() {
                server_options.host = Some(next_arg);
            } else {
                return Err("-H requires an argument".to_string());
            }
        } else if let Some(rest) = arg.strip_prefix("-P") {
            if !rest.is_empty() {
                server_options.port = Some(rest.to_string());
            } else if let Some(next_arg) = args_iter.next() {
                server_options.port = Some(next_arg);
            } else {
                return Err("-P requires an argument".to_string());
            }
        } else if arg == "-L" {
            if let Some(next_arg) = args_iter.next() {
                server_options.socket = Some(next_arg);
            } else {
                return Err("-L requires an argument".to_string());
            }
        } else {
            remaining.push(arg);
            break;
        }
    }

    for arg in args_iter {
        remaining.push(arg);
    }

    let server_env = ServerAddressEnv {
        adb_server_socket: env("ADB_SERVER_SOCKET"),
        android_adb_server_address: env("ANDROID_ADB_SERVER_ADDRESS"),
        android_adb_server_port: env("ANDROID_ADB_SERVER_PORT"),
    };

    let server_address =
        resolve_server_address(&server_options, &server_env).map_err(|e| e.to_string())?;

    if !specified_d_e_s {
        if let Some(env_serial) = env("ANDROID_SERIAL") {
            if !env_serial.is_empty() {
                serial = Some(env_serial);
            }
        }
    }

    let selector = if let Some(id) = transport_id {
        TransportSelector::Id(id)
    } else if let Some(ref s) = serial {
        TransportSelector::Serial(s.clone())
    } else if usb {
        TransportSelector::Usb
    } else if local {
        TransportSelector::Local
    } else {
        TransportSelector::Any
    };

    let device_selection = DeviceSelection {
        serial,
        transport_id,
        usb,
        local,
        selector,
    };

    Ok((
        GlobalOptions {
            server_address_options: server_options,
            device_selection,
            server_address,
            listen_all,
            one_device,
            exit_on_write_error,
        },
        remaining,
    ))
}

fn is_recognized_unimplemented_command(name: &str) -> bool {
    if name.starts_with("wait-for-") {
        return true;
    }
    matches!(
        name,
        "abb"
            | "pair"
            | "emu"
            | "shell"
            | "exec-in"
            | "exec-out"
            | "kill-server"
            | "sideload"
            | "rescue"
            | "tcpip"
            | "remount"
            | "disable-verity"
            | "enable-verity"
            | "reboot"
            | "reboot-bootloader"
            | "reboot-fastboot"
            | "usb"
            | "root"
            | "unroot"
            | "bugreport"
            | "reverse"
            | "mdns"
            | "ls"
            | "push"
            | "pull"
            | "install"
            | "install-multiple"
            | "install-multi-package"
            | "uninstall"
            | "sync"
            | "get-state"
            | "get-serialno"
            | "get-devpath"
            | "logcat"
            | "lolcat"
            | "longcat"
            | "start-server"
            | "backup"
            | "restore"
            | "keygen"
            | "pubkey"
            | "jdwp"
            | "track-jdwp"
            | "track-app"
            | "raw"
            | "features"
            | "host-features"
            | "reconnect"
            | "inc-server"
            | "attach"
            | "detach"
            | "server-status"
            | "transport-id"
            | "help"
            | "--help"
            | "-h"
            | "/?"
            | "version"
            | "--version"
    )
}

/// Executes the CLI binary dispatcher using the given arguments, environment, and output streams.
pub async fn run_cli<E>(args: &[String], env: E, output: &mut Output) -> i32
where
    E: Fn(&str) -> Option<String>,
{
    let (global_opts, remaining) = match parse_global(args, &env) {
        Ok(res) => res,
        Err(err) => {
            let _ = writeln!(output.stderr, "adb: {err}");
            return 1;
        }
    };

    if remaining.is_empty() {
        let _ = write!(output.stdout, "{HELP_TEXT}");
        return 1;
    }

    let cmd_name = remaining[0].as_str();
    let cmd_args: Vec<&str> = remaining[1..].iter().map(|s| s.as_str()).collect();

    let client = AdbClient::builder(global_opts.server_address.clone()).build();

    match cmd_name {
        "devices" => {
            let res = parse_and_exec_devices(&client, &cmd_args).await;
            match res.result {
                Ok(stdout) => {
                    let _ = write!(output.stdout, "{stdout}");
                    0
                }
                Err(err) => {
                    let _ = writeln!(output.stderr, "adb: {err}");
                    1
                }
            }
        }
        "connect" => {
            let res = parse_and_exec_connect(&client, &cmd_args).await;
            match res.result {
                Ok(stdout) => {
                    let _ = write!(output.stdout, "{stdout}");
                    0
                }
                Err(err) => {
                    let _ = writeln!(output.stderr, "adb: {err}");
                    1
                }
            }
        }
        "disconnect" => {
            let res = parse_and_exec_disconnect(&client, &cmd_args).await;
            match res.result {
                Ok(stdout) => {
                    let _ = write!(output.stdout, "{stdout}");
                    0
                }
                Err(err) => {
                    let _ = writeln!(output.stderr, "adb: {err}");
                    1
                }
            }
        }
        "track-devices" => {
            let res = parse_and_exec_track_devices(&client, &cmd_args).await;
            match res.result {
                Ok(mut stream) => {
                    use tokio_stream::StreamExt;
                    while let Some(chunk_res) = stream.next().await {
                        match chunk_res {
                            Ok(chunk) => {
                                let _ = output.stdout.write_all(&chunk);
                                let _ = output.stdout.flush();
                            }
                            Err(err) => {
                                let _ = writeln!(output.stderr, "adb: {err}");
                                return 1;
                            }
                        }
                    }
                    0
                }
                Err(err) => {
                    let _ = writeln!(output.stderr, "adb: {err}");
                    1
                }
            }
        }
        "forward" => {
            let res =
                parse_and_exec_forward(&client, &global_opts.device_selection.selector, &cmd_args)
                    .await;
            match res.result {
                Ok(stdout) => {
                    let _ = write!(output.stdout, "{stdout}");
                    0
                }
                Err(err) => {
                    let _ = writeln!(output.stderr, "adb: {err}");
                    1
                }
            }
        }
        name if is_recognized_unimplemented_command(name) => {
            let _ = writeln!(
                output.stderr,
                "adb: command '{name}' is not implemented yet"
            );
            1
        }
        _ => {
            let _ = writeln!(output.stderr, "adb: unknown command {cmd_name}");
            1
        }
    }
}

/// Parses arguments for `adb devices` and executes `client.devices(long)`.
pub async fn parse_and_exec_devices(client: &AdbClient, args: &[&str]) -> Outcome<String> {
    let long = match args {
        [] => false,
        ["-l"] => true,
        _ => {
            return Outcome {
                result: Err(ClientError::InvalidUsage("adb devices [-l]".to_string())),
                events: Vec::new(),
            };
        }
    };
    client.devices(long).await
}

/// Parses arguments for `adb connect` and executes `client.connect(target)`.
pub async fn parse_and_exec_connect(client: &AdbClient, args: &[&str]) -> Outcome<String> {
    match args {
        [target] => client.connect(target).await,
        _ => Outcome {
            result: Err(ClientError::InvalidUsage(
                "usage: adb connect HOST[:PORT]".to_string(),
            )),
            events: Vec::new(),
        },
    }
}

/// Parses arguments for `adb disconnect` and executes `client.disconnect(target)`.
pub async fn parse_and_exec_disconnect(client: &AdbClient, args: &[&str]) -> Outcome<String> {
    match args {
        [] => client.disconnect(None).await,
        [target] => client.disconnect(Some(target)).await,
        _ => Outcome {
            result: Err(ClientError::InvalidUsage(
                "usage: adb disconnect [HOST[:PORT]]".to_string(),
            )),
            events: Vec::new(),
        },
    }
}

/// Parses arguments for `adb track-devices` and executes `client.track_devices(mode)`.
pub async fn parse_and_exec_track_devices(
    client: &AdbClient,
    args: &[&str],
) -> Outcome<Pin<Box<dyn tokio_stream::Stream<Item = Result<bytes::Bytes, ClientError>> + Send>>> {
    let mode = match args {
        [] => TrackDevicesMode::Normal,
        ["-l"] => TrackDevicesMode::Long,
        ["--proto-text"] => TrackDevicesMode::ProtoText,
        ["--proto-binary"] => TrackDevicesMode::ProtoBinary,
        _ => {
            return Outcome {
                result: Err(ClientError::InvalidUsage(
                    "usage: adb track-devices [-l][--proto-text][--proto-binary]".to_string(),
                )),
                events: Vec::new(),
            };
        }
    };
    client.track_devices(mode).await
}

/// Parses arguments for `adb forward` and executes appropriate forward method on `client`.
pub async fn parse_and_exec_forward(
    client: &AdbClient,
    selector: &TransportSelector,
    args: &[&str],
) -> Outcome<String> {
    if args.is_empty() {
        return Outcome {
            result: Err(ClientError::InvalidUsage(
                "forward requires an argument".to_string(),
            )),
            events: Vec::new(),
        };
    }

    match args[0] {
        "--list" => {
            if args.len() != 1 {
                return Outcome {
                    result: Err(ClientError::InvalidUsage(
                        "--list doesn't take any arguments".to_string(),
                    )),
                    events: Vec::new(),
                };
            }
            client.forward_list(selector).await
        }
        "--remove-all" => {
            if args.len() != 1 {
                return Outcome {
                    result: Err(ClientError::InvalidUsage(
                        "--remove-all doesn't take any arguments".to_string(),
                    )),
                    events: Vec::new(),
                };
            }
            client.forward_remove_all(selector).await
        }
        "--remove" => {
            if args.len() != 2 {
                return Outcome {
                    result: Err(ClientError::InvalidUsage(
                        "--remove requires an argument".to_string(),
                    )),
                    events: Vec::new(),
                };
            }
            client.forward_remove(selector, args[1]).await
        }
        "--no-rebind" => {
            if args.len() != 3 {
                return Outcome {
                    result: Err(ClientError::InvalidUsage(
                        "--no-rebind takes two arguments".to_string(),
                    )),
                    events: Vec::new(),
                };
            }
            client.forward_add(selector, args[1], args[2], true).await
        }
        local => {
            if args.len() != 2 {
                return Outcome {
                    result: Err(ClientError::InvalidUsage(
                        "forward takes two arguments".to_string(),
                    )),
                    events: Vec::new(),
                };
            }
            client.forward_add(selector, local, args[1], false).await
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_global_options_and_errors() {
        let no_env = |_k: &str| None;

        // Valid parsing
        let (opts, remaining) =
            parse_global(vec!["-s", "serial123", "-P", "5037", "devices"], no_env).unwrap();
        assert_eq!(opts.device_selection.serial.as_deref(), Some("serial123"));
        assert_eq!(opts.server_address.port, 5037);
        assert_eq!(remaining, vec!["devices"]);

        // -s attaches digit
        let (opts, remaining) = parse_global(vec!["-s12345", "shell"], no_env).unwrap();
        assert_eq!(opts.device_selection.serial.as_deref(), Some("12345"));
        assert_eq!(remaining, vec!["shell"]);

        // -t attaches digit
        let (opts, remaining) = parse_global(vec!["-t99", "shell"], no_env).unwrap();
        assert_eq!(opts.device_selection.transport_id, Some(99));
        assert_eq!(remaining, vec!["shell"]);

        // -s requires arg
        let err = parse_global(vec!["-s"], no_env).unwrap_err();
        assert_eq!(err, "-s requires an argument");

        // -t invalid
        let err = parse_global(vec!["-t", "abc"], no_env).unwrap_err();
        assert_eq!(err, "invalid transport id");

        // -H requires arg
        let err = parse_global(vec!["-H"], no_env).unwrap_err();
        assert_eq!(err, "-H requires an argument");

        // -L and -H incompatible
        let err =
            parse_global(vec!["-L", "tcp:localhost:5037", "-H", "localhost"], no_env).unwrap_err();
        assert_eq!(err, "-L is incompatible with -H or -P");

        // -P out of range
        let err = parse_global(vec!["-P", "99999"], no_env).unwrap_err();
        assert_eq!(
            err,
            "$ANDROID_ADB_SERVER_PORT must be a positive number less than 65535: got \"99999\""
        );
    }

    #[tokio::test]
    async fn test_run_cli_dispatch() {
        let no_env = |_k: &str| None;

        // No args -> help text to stdout, return 1
        let (mut output, out_buf, err_buf) = Output::memory();
        let code = run_cli(&[], no_env, &mut output).await;
        assert_eq!(code, 1);
        let stdout_str = String::from_utf8(out_buf.lock().unwrap().clone()).unwrap();
        let stderr_str = String::from_utf8(err_buf.lock().unwrap().clone()).unwrap();
        assert!(stdout_str.contains("global options:"));
        assert!(stderr_str.is_empty());

        // Unknown command -> error to stderr, return 1
        let (mut output, out_buf, err_buf) = Output::memory();
        let code = run_cli(&["nosuchcommand".into()], no_env, &mut output).await;
        assert_eq!(code, 1);
        let stdout_str = String::from_utf8(out_buf.lock().unwrap().clone()).unwrap();
        let stderr_str = String::from_utf8(err_buf.lock().unwrap().clone()).unwrap();
        assert!(stdout_str.is_empty());
        assert_eq!(stderr_str, "adb: unknown command nosuchcommand\n");

        // Unimplemented command -> error to stderr, return 1
        let (mut output, _out_buf, err_buf) = Output::memory();
        let code = run_cli(&["install".into()], no_env, &mut output).await;
        assert_eq!(code, 1);
        let stderr_str = String::from_utf8(err_buf.lock().unwrap().clone()).unwrap();
        assert_eq!(
            stderr_str,
            "adb: command 'install' is not implemented yet\n"
        );
    }
}
