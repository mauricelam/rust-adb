use crate::client::{AdbClient, ClientError, SessionFactory};
use crate::session::{Session, SessionError};
use crate::transport::TransportSelector;
use adb_proto::{ShellDecoder, ShellPacket, ShellPacketId};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

#[cfg(unix)]
const SIGPIPE: i32 = libc::SIGPIPE;
#[cfg(not(unix))]
const SIGPIPE: i32 = 13;

const MAX_PAYLOAD_V1: usize = 4096;

/// Escapes an argument string for the shell by single-quoting it and escaping internal single quotes.
///
/// Ports `escape_arg` from `original/adb_utils.cpp`.
pub fn escape_arg(s: &str) -> String {
    let mut result = String::with_capacity(s.len() + 2);
    result.push('\'');

    let mut base = 0;
    while let Some(found) = s[base..].find('\'') {
        let found_idx = base + found;
        result.push_str(&s[base..found_idx]);
        result.push_str("'\\''");
        base = found_idx + 1;
    }
    result.push_str(&s[base..]);
    result.push('\'');

    result
}

/// Options for shell execution.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ShellOptions {
    /// -n: close stdin immediately
    pub close_stdin: bool,
    /// -x: disable shell v2 protocol (historical v1)
    pub disable_shell_protocol: bool,
    /// -T: disable PTY allocation (force raw shell)
    pub force_raw: bool,
}

impl AdbClient {
    /// Runs a non-interactive shell command on the device.
    ///
    /// Ports `adb_shell` and `RemoteShell` from `original/client/commandline.cpp`.
    #[allow(clippy::too_many_arguments)]
    pub async fn shell<R, W1, W2>(
        &self,
        selector: &TransportSelector,
        args: &[&str],
        options: &ShellOptions,
        term: Option<&str>,
        stdin: Option<R>,
        stdout: W1,
        stderr: W2,
        session_factory: &dyn SessionFactory,
    ) -> Result<i32, ClientError>
    where
        R: AsyncRead + Unpin + Send + 'static,
        W1: AsyncWrite + Unpin + Send,
        W2: AsyncWrite + Unpin + Send,
    {
        shell(
            self.addr(),
            selector,
            args,
            options,
            term,
            stdin,
            stdout,
            stderr,
            session_factory,
        )
        .await
    }

    /// Executes an `exec-in` command on the device, forwarding `stdin` to the remote command.
    ///
    /// Ports `exec-in` handling from `original/client/commandline.cpp`.
    pub async fn exec_in<R>(
        &self,
        selector: &TransportSelector,
        args: &[&str],
        stdin: R,
        session_factory: &dyn SessionFactory,
    ) -> Result<i32, ClientError>
    where
        R: AsyncRead + Unpin + Send,
    {
        exec_in(self.addr(), selector, args, stdin, session_factory).await
    }

    /// Executes an `exec-out` command on the device, streaming remote command stdout to local `stdout`.
    ///
    /// Ports `exec-out` handling from `original/client/commandline.cpp`.
    pub async fn exec_out<W>(
        &self,
        selector: &TransportSelector,
        args: &[&str],
        stdout: W,
        session_factory: &dyn SessionFactory,
    ) -> Result<i32, ClientError>
    where
        W: AsyncWrite + Unpin + Send,
    {
        exec_out(self.addr(), selector, args, stdout, session_factory).await
    }
}

/// Runs a non-interactive shell command on the target device at `addr`.
#[allow(clippy::too_many_arguments)]
pub async fn shell<R, W1, W2>(
    addr: &crate::server_address::ServerAddress,
    selector: &TransportSelector,
    args: &[&str],
    options: &ShellOptions,
    term: Option<&str>,
    stdin: Option<R>,
    mut stdout: W1,
    mut stderr: W2,
    session_factory: &dyn SessionFactory,
) -> Result<i32, ClientError>
where
    R: AsyncRead + Unpin + Send + 'static,
    W1: AsyncWrite + Unpin + Send,
    W2: AsyncWrite + Unpin + Send,
{
    // Query device features to check for shell_v2 support
    let client = AdbClient::new(addr.clone());
    let features_str = client.features(selector, session_factory).await?;
    let device_has_shell2 = features_str.lines().any(|line| line == "shell_v2");

    let use_shell_protocol = device_has_shell2 && !options.disable_shell_protocol;

    let command = args.join(" ");

    let service_string = if use_shell_protocol {
        let mut s_args = vec!["v2".to_string()];
        if let Some(t) = term {
            s_args.push(format!("TERM={t}"));
        }
        s_args.push("raw".to_string());
        format!("shell,{}:{command}", s_args.join(","))
    } else {
        format!("shell:{command}")
    };

    if service_string.len() > MAX_PAYLOAD_V1 && !use_shell_protocol {
        return Err(ClientError::InvalidUsage(
            "error: shell command too long".to_string(),
        ));
    }

    if !use_shell_protocol {
        let msg = if !device_has_shell2 {
            "error: device only supports allocating a pty"
        } else {
            "error: -x only supports allocating a pty"
        };
        return Err(ClientError::InvalidUsage(msg.to_string()));
    }

    let session = Session::connect_and_switch_factory(
        addr,
        selector,
        &service_string,
        false,
        session_factory,
    )
    .await?;

    let stream = session.into_inner();
    let (mut read_half, mut write_half) = tokio::io::split(stream);

    let close_stdin = options.close_stdin;
    let stdin_handle = tokio::spawn(async move {
        if !close_stdin {
            if let Some(mut r) = stdin {
                let mut buf = [0u8; 8192];
                loop {
                    let n = match r.read(&mut buf).await {
                        Ok(0) => break,
                        Ok(n) => n,
                        Err(_) => break,
                    };
                    let stdin_pkt = ShellPacket::new(ShellPacketId::Stdin, buf[..n].to_vec());
                    if write_half
                        .write_all(&stdin_pkt.encode_to_vec())
                        .await
                        .is_err()
                    {
                        return;
                    }
                }
            }
        }
        let close_pkt = ShellPacket::new(ShellPacketId::CloseStdin, vec![]);
        let _ = write_half.write_all(&close_pkt.encode_to_vec()).await;
        let _ = write_half.flush().await;
    });

    let mut decoder = ShellDecoder::new();
    let mut exit_code: Option<u8> = None;
    let mut read_buf = [0u8; 8192];
    let mut write_err = false;

    'outer: loop {
        let n = match read_half.read(&mut read_buf).await {
            Ok(0) => break,
            Ok(n) => n,
            Err(_) => break,
        };
        decoder.feed(&read_buf[..n]);

        loop {
            match decoder.pop_packet() {
                Ok(Some(pkt)) => match pkt.id {
                    ShellPacketId::Stdout => {
                        if stdout.write_all(&pkt.data).await.is_err() {
                            write_err = true;
                            break 'outer;
                        }
                    }
                    ShellPacketId::Stderr => {
                        if stderr.write_all(&pkt.data).await.is_err() {
                            write_err = true;
                            break 'outer;
                        }
                    }
                    ShellPacketId::Exit => {
                        if let Ok(code) = pkt.parse_exit_code() {
                            exit_code = Some(code);
                            break 'outer;
                        }
                    }
                    _ => {}
                },
                Ok(None) => break,
                Err(_) => break 'outer,
            }
        }
        if write_err {
            break;
        }
    }

    stdin_handle.abort();

    if write_err {
        return Ok(128 + SIGPIPE);
    }

    let code = match exit_code {
        Some(c) => c as i32,
        None => 255,
    };

    Ok(code)
}

/// Executes an `exec-in` command on the device, forwarding `stdin` to the remote command.
pub async fn exec_in<R>(
    addr: &crate::server_address::ServerAddress,
    selector: &TransportSelector,
    args: &[&str],
    mut stdin: R,
    session_factory: &dyn SessionFactory,
) -> Result<i32, ClientError>
where
    R: AsyncRead + Unpin + Send,
{
    if args.is_empty() {
        return Err(ClientError::InvalidUsage(
            "usage: adb exec-in command".to_string(),
        ));
    }

    let mut cmd = format!("exec:{}", args[0]);
    for arg in &args[1..] {
        cmd.push(' ');
        cmd.push_str(&escape_arg(arg));
    }

    let session =
        Session::connect_and_switch_factory(addr, selector, &cmd, false, session_factory).await?;
    let mut stream = session.into_inner();

    let mut buf = [0u8; 8192];
    loop {
        let n = stdin
            .read(&mut buf)
            .await
            .map_err(|e| ClientError::Session(SessionError::WriteFailure(e)))?;
        if n == 0 {
            break;
        }
        stream
            .write_all(&buf[..n])
            .await
            .map_err(|e| ClientError::Session(SessionError::WriteFailure(e)))?;
    }

    Ok(0)
}

/// Executes an `exec-out` command on the device, streaming remote command stdout to local `stdout`.
pub async fn exec_out<W>(
    addr: &crate::server_address::ServerAddress,
    selector: &TransportSelector,
    args: &[&str],
    mut stdout: W,
    session_factory: &dyn SessionFactory,
) -> Result<i32, ClientError>
where
    W: AsyncWrite + Unpin + Send,
{
    if args.is_empty() {
        return Err(ClientError::InvalidUsage(
            "usage: adb exec-out command".to_string(),
        ));
    }

    let mut cmd = format!("exec:{}", args[0]);
    for arg in &args[1..] {
        cmd.push(' ');
        cmd.push_str(&escape_arg(arg));
    }

    let session =
        Session::connect_and_switch_factory(addr, selector, &cmd, false, session_factory).await?;
    let mut stream = session.into_inner();

    let mut buf = [0u8; 8192];
    loop {
        let n = stream
            .read(&mut buf)
            .await
            .map_err(|e| ClientError::Session(SessionError::ReadStatusFailed(e)))?;
        if n == 0 {
            break;
        }
        stdout
            .write_all(&buf[..n])
            .await
            .map_err(|e| ClientError::Session(SessionError::WriteFailure(e)))?;
    }

    Ok(0)
}
