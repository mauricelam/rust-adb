//! Argument parsing and CLI helpers for `adb`.

mod version;
pub use version::{
    get_os_version, get_version_string, ADB_SERVER_VERSION, ADB_VERSION_MAJOR, ADB_VERSION_MINOR,
    BUILD_NUMBER, PLATFORM_TOOLS_VERSION,
};

use adb_client::{
    resolve_server_address, AdbClient, ClientError, ClientEvent, DeviceSelection,
    ExternalBinaryLauncher, Outcome, ServerAddress, ServerAddressEnv, ServerAddressOptions,
    TrackDevicesMode, TransportSelector,
};
use std::io::Write;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

/// Static usage documentation string.
pub const HELP_TEXT: &str = include_str!("help.txt");

/// Returns the full usage text prefixed by the version header block and a blank line.
///
/// Ports `help()` in `original/client/commandline.cpp:87`.
pub fn get_help_string() -> String {
    format!("{}\n{}", get_version_string(), HELP_TEXT)
}

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
            | "logcat"
            | "lolcat"
            | "longcat"
            | "backup"
            | "restore"
            | "keygen"
            | "pubkey"
            | "jdwp"
            | "track-jdwp"
            | "track-app"
            | "raw"
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

fn print_events(events: &[ClientEvent], stderr: &mut Box<dyn Write + Send>) {
    for event in events {
        let _ = writeln!(stderr, "{event}");
    }
}

fn print_error(err: &ClientError, stderr: &mut Box<dyn Write + Send>) {
    if err.kind() == adb_client::ClientErrorKind::Usage {
        let _ = writeln!(stderr, "adb: {err}");
    } else {
        let _ = writeln!(stderr, "error: {err}");
    }
}

fn print_error_prog(err: &ClientError, stderr: &mut Box<dyn Write + Send>) {
    let _ = writeln!(stderr, "adb: {err}");
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

    // Cites no-args branch in original/client/commandline.cpp:1733
    if remaining.is_empty() {
        let _ = write!(output.stdout, "{}", get_help_string());
        return 1;
    }

    let cmd_name = remaining[0].as_str();
    let cmd_args: Vec<&str> = remaining[1..].iter().map(|s| s.as_str()).collect();

    // Cites `help` branch in original/client/commandline.cpp:2167
    if cmd_name == "help" || cmd_name == "--help" || cmd_name == "/?" {
        let _ = write!(output.stdout, "{}", get_help_string());
        return 0;
    }

    // Cites `version` branch in original/client/commandline.cpp:2170
    if cmd_name == "version" || cmd_name == "--version" {
        let _ = write!(output.stdout, "{}", get_version_string());
        return 0;
    }

    let mut builder = AdbClient::builder(global_opts.server_address.clone());
    if let Some(server_bin) = env("ADB_SERVER_BIN") {
        if !server_bin.is_empty() {
            builder = builder.launcher(ExternalBinaryLauncher::new(server_bin));
        }
    }
    let client = builder.build();

    match cmd_name {
        // Cites `start-server` branch in original/client/commandline.cpp:2102
        "start-server" => {
            let outcome = client.check_server_version().await;
            print_events(&outcome.events, &mut output.stderr);
            match outcome.result {
                Ok(()) => 0,
                Err(err) => {
                    print_error(&err, &mut output.stderr);
                    1
                }
            }
        }
        // Cites `get-state`, `get-serialno`, `get-devpath` in original/client/commandline.cpp:2094
        "get-state" => {
            let outcome = client
                .get_state(&global_opts.device_selection.selector)
                .await;
            print_events(&outcome.events, &mut output.stderr);
            match outcome.result {
                Ok(stdout) => {
                    let _ = write!(output.stdout, "{stdout}");
                    0
                }
                Err(err) => {
                    print_error(&err, &mut output.stderr);
                    1
                }
            }
        }
        "get-serialno" => {
            let outcome = client
                .get_serialno(&global_opts.device_selection.selector)
                .await;
            print_events(&outcome.events, &mut output.stderr);
            match outcome.result {
                Ok(stdout) => {
                    let _ = write!(output.stdout, "{stdout}");
                    0
                }
                Err(err) => {
                    print_error(&err, &mut output.stderr);
                    1
                }
            }
        }
        "get-devpath" => {
            let outcome = client
                .get_devpath(&global_opts.device_selection.selector)
                .await;
            print_events(&outcome.events, &mut output.stderr);
            match outcome.result {
                Ok(stdout) => {
                    let _ = write!(output.stdout, "{stdout}");
                    0
                }
                Err(err) => {
                    print_error(&err, &mut output.stderr);
                    1
                }
            }
        }
        // Cites `transport-id` in original/client/commandline.cpp:1775
        "transport-id" => {
            let outcome = client
                .transport_id(&global_opts.device_selection.selector)
                .await;
            print_events(&outcome.events, &mut output.stderr);
            match outcome.result {
                Ok(stdout) => {
                    let _ = write!(output.stdout, "{stdout}");
                    0
                }
                Err(err) => {
                    print_error_prog(&err, &mut output.stderr);
                    1
                }
            }
        }
        // Cites `features` in original/client/commandline.cpp:2173
        "features" => {
            let outcome = client
                .features(
                    &global_opts.device_selection.selector,
                    &adb_proto::FeatureOptions::default(),
                )
                .await;
            print_events(&outcome.events, &mut output.stderr);
            match outcome.result {
                Ok(stdout) => {
                    let _ = write!(output.stdout, "{stdout}");
                    0
                }
                Err(err) => {
                    print_error_prog(&err, &mut output.stderr);
                    1
                }
            }
        }
        // Cites `host-features` in original/client/commandline.cpp:2183
        "host-features" => {
            let outcome = client.host_features().await;
            print_events(&outcome.events, &mut output.stderr);
            match outcome.result {
                Ok(stdout) => {
                    let _ = write!(output.stdout, "{stdout}");
                    0
                }
                Err(err) => {
                    print_error(&err, &mut output.stderr);
                    1
                }
            }
        }
        // Cites `kill-server` branch in original/client/commandline.cpp:1844
        "kill-server" => {
            let outcome = client.kill_server().await;
            print_events(&outcome.events, &mut output.stderr);
            match outcome.result {
                Ok(()) => 0,
                Err(err) => {
                    print_error(&err, &mut output.stderr);
                    1
                }
            }
        }
        // Cites `devices` branch in original/client/commandline.cpp:1758
        "devices" => {
            let outcome = parse_and_exec_devices(&client, &cmd_args).await;
            print_events(&outcome.events, &mut output.stderr);
            match outcome.result {
                Ok(stdout) => {
                    let _ = write!(output.stdout, "{stdout}");
                    0
                }
                Err(err) => {
                    print_error(&err, &mut output.stderr);
                    1
                }
            }
        }
        // Cites `connect` branch in original/client/commandline.cpp:1784
        "connect" => {
            let outcome = parse_and_exec_connect(&client, &cmd_args).await;
            print_events(&outcome.events, &mut output.stderr);
            match outcome.result {
                Ok(stdout) => {
                    let _ = writeln!(output.stdout, "{stdout}");
                    0
                }
                Err(err) => {
                    print_error(&err, &mut output.stderr);
                    1
                }
            }
        }
        // Cites `disconnect` branch in original/client/commandline.cpp:1789
        "disconnect" => {
            let outcome = parse_and_exec_disconnect(&client, &cmd_args).await;
            print_events(&outcome.events, &mut output.stderr);
            match outcome.result {
                Ok(stdout) => {
                    let _ = writeln!(output.stdout, "{stdout}");
                    0
                }
                Err(err) => {
                    print_error(&err, &mut output.stderr);
                    1
                }
            }
        }
        // Cites `track-devices` branch in original/client/commandline.cpp:2142
        "track-devices" => {
            let outcome = parse_and_exec_track_devices(&client, &cmd_args).await;
            print_events(&outcome.events, &mut output.stderr);
            match outcome.result {
                Ok(mut stream) => {
                    use tokio_stream::StreamExt;
                    while let Some(chunk_res) = stream.next().await {
                        match chunk_res {
                            Ok(chunk) => {
                                if output.stdout.write_all(&chunk).is_err()
                                    || output.stdout.flush().is_err()
                                {
                                    return 1;
                                }
                            }
                            Err(err) => {
                                print_error(&err, &mut output.stderr);
                                return 1;
                            }
                        }
                    }
                    0
                }
                Err(err) => {
                    print_error(&err, &mut output.stderr);
                    1
                }
            }
        }
        "forward" => {
            let outcome =
                parse_and_exec_forward(&client, &global_opts.device_selection.selector, &cmd_args)
                    .await;
            print_events(&outcome.events, &mut output.stderr);
            match outcome.result {
                Ok(stdout) => {
                    let _ = write!(output.stdout, "{stdout}");
                    0
                }
                Err(err) => {
                    print_error(&err, &mut output.stderr);
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

    #[test]
    fn test_selector_precedence() {
        let env_with_serial = |k: &str| {
            if k == "ANDROID_SERIAL" {
                Some("env_serial_123".to_string())
            } else {
                None
            }
        };

        // 1. ANDROID_SERIAL fallback when no flags passed
        let (opts, _) = parse_global(vec!["get-state"], env_with_serial).unwrap();
        assert_eq!(
            opts.device_selection.selector,
            TransportSelector::Serial("env_serial_123".to_string())
        );

        // 2. -d overrides ANDROID_SERIAL
        let (opts, _) = parse_global(vec!["-d", "get-state"], env_with_serial).unwrap();
        assert_eq!(opts.device_selection.selector, TransportSelector::Usb);

        // 3. -e overrides ANDROID_SERIAL
        let (opts, _) = parse_global(vec!["-e", "get-state"], env_with_serial).unwrap();
        assert_eq!(opts.device_selection.selector, TransportSelector::Local);

        // 4. -s overrides ANDROID_SERIAL and -d/-e
        let (opts, _) =
            parse_global(vec!["-d", "-s", "cli_serial", "get-state"], env_with_serial).unwrap();
        assert_eq!(
            opts.device_selection.selector,
            TransportSelector::Serial("cli_serial".to_string())
        );

        // 5. -t overrides -s, -d, -e, and ANDROID_SERIAL
        let (opts, _) = parse_global(
            vec!["-d", "-s", "cli_serial", "-t", "42", "get-state"],
            env_with_serial,
        )
        .unwrap();
        assert_eq!(opts.device_selection.selector, TransportSelector::Id(42));
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
        assert!(stdout_str.contains("Android Debug Bridge version 1.0.41"));
        assert!(stdout_str.contains("global options:"));
        assert!(stderr_str.is_empty());

        // adb version -> version string to stdout, return 0
        let (mut output, out_buf, err_buf) = Output::memory();
        let code = run_cli(&["version".into()], no_env, &mut output).await;
        assert_eq!(code, 0);
        let stdout_str = String::from_utf8(out_buf.lock().unwrap().clone()).unwrap();
        let stderr_str = String::from_utf8(err_buf.lock().unwrap().clone()).unwrap();
        assert!(stdout_str.contains("Android Debug Bridge version 1.0.41"));
        assert!(!stdout_str.contains("global options:"));
        assert!(stderr_str.is_empty());

        // adb help, --help, /? -> full help text to stdout, return 0
        for help_flag in ["help", "--help", "/?"] {
            let (mut output, out_buf, err_buf) = Output::memory();
            let code = run_cli(&[help_flag.into()], no_env, &mut output).await;
            assert_eq!(code, 0, "failed for {help_flag}");
            let stdout_str = String::from_utf8(out_buf.lock().unwrap().clone()).unwrap();
            let stderr_str = String::from_utf8(err_buf.lock().unwrap().clone()).unwrap();
            assert!(stdout_str.contains("Android Debug Bridge version 1.0.41"));
            assert!(stdout_str.contains("global options:"));
            assert!(stderr_str.is_empty());
        }

        // adb --version -> version string to stdout, return 0
        let (mut output, out_buf, err_buf) = Output::memory();
        let code = run_cli(&["--version".into()], no_env, &mut output).await;
        assert_eq!(code, 0);
        let stdout_str = String::from_utf8(out_buf.lock().unwrap().clone()).unwrap();
        let stderr_str = String::from_utf8(err_buf.lock().unwrap().clone()).unwrap();
        assert!(stdout_str.contains("Android Debug Bridge version 1.0.41"));
        assert!(!stdout_str.contains("global options:"));
        assert!(stderr_str.is_empty());

        // adb -h -> unknown command to stderr, return 1 (matching reference C++ behavior)
        let (mut output, out_buf, err_buf) = Output::memory();
        let code = run_cli(&["-h".into()], no_env, &mut output).await;
        assert_eq!(code, 1);
        let stdout_str = String::from_utf8(out_buf.lock().unwrap().clone()).unwrap();
        let stderr_str = String::from_utf8(err_buf.lock().unwrap().clone()).unwrap();
        assert!(stdout_str.is_empty());
        assert_eq!(stderr_str, "adb: unknown command -h\n");

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
