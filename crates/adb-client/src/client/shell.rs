use super::error::ClientError;
use super::events::Outcome;
use super::AdbClient;
use crate::shell_interactive::PtyMode;
use crate::transport::TransportSelector;

/// Escapes a command argument for the shell following standard ADB single-quote escaping rules.
///
/// Ports `escape_arg` from `original/adb_utils.cpp`.
pub fn escape_arg(s: &str) -> String {
    let mut result = String::with_capacity(s.len() + 2);
    result.push('\'');
    for c in s.chars() {
        if c == '\'' {
            result.push_str("'\\''");
        } else {
            result.push(c);
        }
    }
    result.push('\'');
    result
}

/// Options for device shell execution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellOptions {
    /// -n: Close stdin immediately / do not forward stdin.
    pub close_stdin: bool,
    /// -x: Disable Shell v2 protocol and force historical Shell v1 protocol.
    pub disable_shell_protocol: bool,
    /// -T: Disable remote PTY allocation (force raw mode).
    pub no_pty: bool,
    /// -t / -tt / -T PTY mode selection.
    pub pty_mode: PtyMode,
    /// -e: Escape character (e.g. '~', None if 'none' or disabled).
    pub escape_char: Option<char>,
    /// Value for `TERM` environment variable (e.g. `xterm-256color`).
    pub term: Option<String>,
}

impl Default for ShellOptions {
    fn default() -> Self {
        Self {
            close_stdin: false,
            disable_shell_protocol: false,
            no_pty: false,
            pty_mode: PtyMode::Auto,
            escape_char: Some('~'),
            term: None,
        }
    }
}

impl AdbClient {
    /// Runs a command on the device using Shell v1 or v2 protocol.
    ///
    /// Ports `adb_shell` and `RemoteShell` from `original/client/commandline.cpp`.
    pub async fn shell<R, W1, W2>(
        &self,
        selector: &TransportSelector,
        args: &[&str],
        options: &ShellOptions,
        stdin: Option<R>,
        stdout: Option<W1>,
        stderr: Option<W2>,
    ) -> Outcome<i32>
    where
        R: tokio::io::AsyncRead + Unpin + Send + 'static,
        W1: tokio::io::AsyncWrite + Unpin + Send,
        W2: tokio::io::AsyncWrite + Unpin + Send,
    {
        let outcome = self.ensure_server().await;
        if let Err(err) = outcome.result {
            return Outcome {
                result: Err(err),
                events: outcome.events,
            };
        }

        let features_outcome = self
            .features(selector, &adb_proto::FeatureOptions::default())
            .await;
        let features_str = match features_outcome.result {
            Ok(f) => f,
            Err(err) => {
                return Outcome {
                    result: Err(err),
                    events: outcome.events,
                };
            }
        };

        let device_features = adb_proto::Features::parse(&features_str.replace('\n', ","));
        let device_supports_shell2 = device_features.contains(&adb_proto::Feature::Shell2);

        let command = args.join(" ");
        let is_interactive = command.is_empty();

        let use_shell_protocol = device_supports_shell2 && !options.disable_shell_protocol;

        if !use_shell_protocol {
            let service_string = format!("shell:{command}");
            if service_string.len() > adb_proto::limits::MAX_PAYLOAD_V1 {
                return Outcome {
                    result: Err(ClientError::QueryFailed(
                        "shell command too long".to_string(),
                    )),
                    events: outcome.events,
                };
            }
        }

        let stdin_isatty = stdin
            .as_ref()
            .and_then(|r| crate::shell_interactive::find_tty_fd(r))
            .is_some();

        let pty_dec = match crate::shell_interactive::determine_pty_allocation(
            options,
            is_interactive,
            stdin_isatty,
            device_supports_shell2,
        ) {
            Ok(dec) => dec,
            Err(err) => {
                return Outcome {
                    result: Err(err),
                    events: outcome.events,
                };
            }
        };

        let service_string = if pty_dec.use_shell_protocol {
            let mut v2_args = vec!["v2".to_string()];
            if let Some(term) = &options.term {
                v2_args.push(format!("TERM={term}"));
            }
            if !pty_dec.type_arg.is_empty() {
                v2_args.push(pty_dec.type_arg.to_string());
            }
            format!("shell,{}:{}", v2_args.join(","), command)
        } else {
            format!("shell:{command}")
        };

        let res = self
            .run_shell_session(
                selector,
                &service_string,
                &pty_dec,
                options,
                stdin,
                stdout,
                stderr,
            )
            .await;

        Outcome {
            result: res,
            events: outcome.events,
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn run_shell_session<R, W1, W2>(
        &self,
        selector: &TransportSelector,
        service_string: &str,
        pty_dec: &crate::shell_interactive::PtyDecision,
        options: &ShellOptions,
        stdin: Option<R>,
        mut stdout_writer: Option<W1>,
        mut stderr_writer: Option<W2>,
    ) -> Result<i32, ClientError>
    where
        R: tokio::io::AsyncRead + Unpin + Send + 'static,
        W1: tokio::io::AsyncWrite + Unpin + Send,
        W2: tokio::io::AsyncWrite + Unpin + Send,
    {
        use crate::session::SessionError;
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        if let Some(warning) = &pty_dec.warning {
            if let Some(ref mut w) = stderr_writer {
                let _ = w.write_all(warning.as_bytes()).await;
                let _ = w.flush().await;
            } else {
                eprint!("{warning}");
            }
        }

        let stdin_tty_fd = stdin
            .as_ref()
            .and_then(|r| crate::shell_interactive::find_tty_fd(r));

        let _raw_guard = if pty_dec.raw_stdin {
            stdin_tty_fd.and_then(crate::shell_interactive::RawTerminalGuard::new)
        } else {
            None
        };

        let mut session = self.session_factory.connect(&self.addr).await?;
        session.select_transport(selector).await?;
        session.send_service(service_string).await?;
        session.read_status().await?;

        let stream = session.into_inner();
        let (mut read_half, mut write_half) = tokio::io::split(stream);

        let mut stdin_reader = stdin;
        let mut stdin_eof = stdin_reader.is_none() || options.close_stdin;

        let mut read_buf = [0u8; 8192];
        let mut stdin_buf = [0u8; 8192];

        if pty_dec.use_shell_protocol {
            let mut decoder = adb_proto::ShellDecoder::new();
            let mut exit_code: Option<i32> = None;
            let mut escape_sm =
                crate::shell_interactive::EscapeStateMachine::new(if pty_dec.raw_stdin {
                    options.escape_char
                } else {
                    None
                });

            // Send initial window size if available and requesting PTY
            if pty_dec.type_arg == "pty" {
                if let Some(fd) = stdin_tty_fd {
                    if let Some((r, c, x, y)) = crate::shell_interactive::get_window_size(fd) {
                        let payload = crate::shell_interactive::format_window_size(r, c, x, y);
                        let winsz_pkt = adb_proto::ShellPacket::new(
                            adb_proto::ShellPacketId::WindowSizeChange,
                            payload,
                        );
                        if let Ok(encoded) = winsz_pkt.encode() {
                            let _ = write_half.write_all(&encoded).await;
                            let _ = write_half.flush().await;
                        }
                    }
                }
            }

            // SIGWINCH watcher channel
            #[cfg(unix)]
            let (winch_tx, mut winch_rx) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();
            #[cfg(unix)]
            let sigwinch_task = if pty_dec.type_arg == "pty" {
                stdin_tty_fd.map(|fd| {
                    tokio::spawn(async move {
                        if let Ok(mut sig) = tokio::signal::unix::signal(
                            tokio::signal::unix::SignalKind::window_change(),
                        ) {
                            while sig.recv().await.is_some() {
                                if let Some((r, c, x, y)) =
                                    crate::shell_interactive::get_window_size(fd)
                                {
                                    let payload =
                                        crate::shell_interactive::format_window_size(r, c, x, y);
                                    if winch_tx.send(payload).is_err() {
                                        break;
                                    }
                                }
                            }
                        }
                    })
                })
            } else {
                None
            };

            if stdin_reader.is_none() || options.close_stdin {
                let close_pkt =
                    adb_proto::ShellPacket::new(adb_proto::ShellPacketId::CloseStdin, vec![]);
                if let Ok(encoded) = close_pkt.encode() {
                    let _ = write_half.write_all(&encoded).await;
                    let _ = write_half.flush().await;
                }
                stdin_eof = true;
            }

            loop {
                tokio::select! {
                    n_res = read_half.read(&mut read_buf) => {
                        let n = n_res.map_err(|e| ClientError::Session(SessionError::ReadStatusFailed(e)))?;
                        if n == 0 {
                            break;
                        }
                        decoder.feed(&read_buf[..n]);

                        while let Ok(Some(pkt)) = decoder.pop_packet() {
                            match pkt.id {
                                adb_proto::ShellPacketId::Stdout => {
                                    if let Some(ref mut w) = stdout_writer {
                                        if w.write_all(&pkt.data).await.is_err() || w.flush().await.is_err() {
                                            exit_code = Some(128 + 13);
                                            break;
                                        }
                                    }
                                }
                                adb_proto::ShellPacketId::Stderr => {
                                    if let Some(ref mut w) = stderr_writer {
                                        if w.write_all(&pkt.data).await.is_err() || w.flush().await.is_err() {
                                            exit_code = Some(128 + 13);
                                            break;
                                        }
                                    }
                                }
                                adb_proto::ShellPacketId::Exit => {
                                    if let Ok(code) = pkt.parse_exit_code() {
                                        exit_code = Some(code as i32);
                                    }
                                }
                                _ => {}
                            }
                        }

                        if exit_code == Some(128 + 13) {
                            break;
                        }
                    }
                    winch_payload = async {
                        #[cfg(unix)]
                        {
                            winch_rx.recv().await
                        }
                        #[cfg(not(unix))]
                        {
                            std::future::pending::<Option<Vec<u8>>>().await
                        }
                    } => {
                        if let Some(payload) = winch_payload {
                            let winsz_pkt = adb_proto::ShellPacket::new(
                                adb_proto::ShellPacketId::WindowSizeChange,
                                payload,
                            );
                            if let Ok(encoded) = winsz_pkt.encode() {
                                let _ = write_half.write_all(&encoded).await;
                                let _ = write_half.flush().await;
                            }
                        }
                    }
                    n_res = async {
                        if let Some(ref mut r) = stdin_reader {
                            r.read(&mut stdin_buf).await
                        } else {
                            std::future::pending().await
                        }
                    }, if !stdin_eof => {
                        match n_res {
                            Ok(0) => {
                                stdin_eof = true;
                                let close_pkt = adb_proto::ShellPacket::new(
                                    adb_proto::ShellPacketId::CloseStdin,
                                    vec![],
                                );
                                if let Ok(encoded) = close_pkt.encode() {
                                    let _ = write_half.write_all(&encoded).await;
                                    let _ = write_half.flush().await;
                                }
                            }
                            Ok(n) => {
                                let esc_res = escape_sm.process_chunk(&stdin_buf[..n]);
                                match esc_res {
                                    crate::shell_interactive::EscapeResult::Output(out_bytes) => {
                                        if !out_bytes.is_empty() {
                                            let stdin_pkt = adb_proto::ShellPacket::new(
                                                adb_proto::ShellPacketId::Stdin,
                                                out_bytes,
                                            );
                                            if let Ok(encoded) = stdin_pkt.encode() {
                                                if write_half.write_all(&encoded).await.is_err() || write_half.flush().await.is_err() {
                                                    stdin_eof = true;
                                                }
                                            }
                                        }
                                    }
                                    crate::shell_interactive::EscapeResult::Disconnect => {
                                        const DISCONNECTED_MSG: &[u8] = b"\r\n[ disconnected ]\r\n";
                                        if let Some(ref mut w) = stderr_writer {
                                            let _ = w.write_all(DISCONNECTED_MSG).await;
                                            let _ = w.flush().await;
                                        } else {
                                            eprint!("\r\n[ disconnected ]\r\n");
                                        }
                                        #[cfg(unix)]
                                        if let Some(task) = sigwinch_task {
                                            task.abort();
                                        }
                                        #[cfg(unix)]
                                        drop(_raw_guard);
                                        return Ok(0);
                                    }
                                }
                            }
                            Err(_) => {
                                stdin_eof = true;
                            }
                        }
                    }
                }
            }

            #[cfg(unix)]
            if let Some(task) = sigwinch_task {
                task.abort();
            }

            Ok(exit_code.unwrap_or(255))
        } else {
            loop {
                tokio::select! {
                    n_res = read_half.read(&mut read_buf) => {
                        let n = n_res.map_err(|e| ClientError::Session(SessionError::ReadStatusFailed(e)))?;
                        if n == 0 {
                            break;
                        }
                        if let Some(ref mut w) = stdout_writer {
                            if w.write_all(&read_buf[..n]).await.is_err() || w.flush().await.is_err() {
                                break;
                            }
                        }
                    }
                    n_res = async {
                        if let Some(ref mut r) = stdin_reader {
                            r.read(&mut stdin_buf).await
                        } else {
                            std::future::pending().await
                        }
                    }, if !stdin_eof => {
                        match n_res {
                            Ok(0) => {
                                stdin_eof = true;
                            }
                            Ok(n) => {
                                if write_half.write_all(&stdin_buf[..n]).await.is_err() || write_half.flush().await.is_err() {
                                    stdin_eof = true;
                                }
                            }
                            Err(_) => {
                                stdin_eof = true;
                            }
                        }
                    }
                }
            }

            Ok(0)
        }
    }

    /// Runs a command on the device with stdin piped to the remote command (`exec-in`).
    pub async fn exec_in<R>(
        &self,
        selector: &TransportSelector,
        args: &[&str],
        mut stdin: R,
    ) -> Outcome<i32>
    where
        R: tokio::io::AsyncRead + Unpin + Send,
    {
        if args.is_empty() {
            return Outcome {
                result: Err(ClientError::InvalidUsage(
                    "usage: adb exec-in command".to_string(),
                )),
                events: Vec::new(),
            };
        }

        let mut cmd = format!("exec:{}", args[0]);
        for arg in &args[1..] {
            cmd.push(' ');
            cmd.push_str(&escape_arg(arg));
        }

        self.with_server(async move {
            use crate::session::SessionError;
            let mut session = self.session_factory.connect(&self.addr).await?;
            session.select_transport(selector).await?;
            session.send_service(&cmd).await?;
            session.read_status().await?;

            let mut stream = session.into_inner();
            tokio::io::copy(&mut stdin, &mut stream)
                .await
                .map_err(|e| ClientError::Session(SessionError::WriteFailure(e)))?;

            Ok(0)
        })
        .await
    }

    /// Runs a command on the device and streams stdout back (`exec-out`).
    pub async fn exec_out<W>(
        &self,
        selector: &TransportSelector,
        args: &[&str],
        mut stdout: W,
    ) -> Outcome<i32>
    where
        W: tokio::io::AsyncWrite + Unpin + Send,
    {
        if args.is_empty() {
            return Outcome {
                result: Err(ClientError::InvalidUsage(
                    "usage: adb exec-out command".to_string(),
                )),
                events: Vec::new(),
            };
        }

        let mut cmd = format!("exec:{}", args[0]);
        for arg in &args[1..] {
            cmd.push(' ');
            cmd.push_str(&escape_arg(arg));
        }

        self.with_server(async move {
            use crate::session::SessionError;
            let mut session = self.session_factory.connect(&self.addr).await?;
            session.select_transport(selector).await?;
            session.send_service(&cmd).await?;
            session.read_status().await?;

            let mut stream = session.into_inner();
            tokio::io::copy(&mut stream, &mut stdout)
                .await
                .map_err(|e| ClientError::Session(SessionError::ReadStatusFailed(e)))?;

            Ok(0)
        })
        .await
    }
}
