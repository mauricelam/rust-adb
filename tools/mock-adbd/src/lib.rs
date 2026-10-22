use adb_proto::{
    negotiate_max_payload, Banner, Command, Feature, Features, Header, ShellDecoder, ShellPacket,
    ShellPacketId, A_VERSION, MAX_PAYLOAD,
};
use std::collections::VecDeque;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;

#[derive(Debug, Clone)]
pub struct MockAdbdOptions {
    pub enable_shell_v2: bool,
}

impl Default for MockAdbdOptions {
    fn default() -> Self {
        Self {
            enable_shell_v2: true,
        }
    }
}

pub struct MockAdbdServer {
    local_addr: SocketAddr,
    handle: JoinHandle<()>,
}

impl MockAdbdServer {
    pub async fn bind(addr: &str) -> std::io::Result<Self> {
        Self::bind_with_options(addr, MockAdbdOptions::default()).await
    }

    pub async fn bind_with_options(addr: &str, options: MockAdbdOptions) -> std::io::Result<Self> {
        let listener = TcpListener::bind(addr).await?;
        let local_addr = listener.local_addr()?;

        let handle = tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let options = options.clone();
                tokio::spawn(async move {
                    let _ = handle_connection_with_options(stream, options).await;
                });
            }
        });

        Ok(Self { local_addr, handle })
    }

    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    pub fn shutdown(self) {
        self.handle.abort();
    }
}

pub async fn read_packet<R: AsyncRead + Unpin>(
    reader: &mut R,
) -> std::io::Result<Option<(Header, Vec<u8>)>> {
    let mut header_buf = [0u8; Header::SIZE];
    match reader.read_exact(&mut header_buf).await {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }

    let header = Header::decode(&header_buf)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;

    let mut payload = vec![0u8; header.data_length as usize];
    if header.data_length > 0 {
        match reader.read_exact(&mut payload).await {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
            Err(e) => return Err(e),
        }
    }

    Ok(Some((header, payload)))
}

pub async fn write_packet<W: AsyncWrite + Unpin>(
    writer: &mut W,
    command: Command,
    arg0: u32,
    arg1: u32,
    payload: &[u8],
) -> std::io::Result<()> {
    let header = Header::new(command, arg0, arg1, payload.len() as u32, 0);
    writer.write_all(&header.encode()).await?;
    if !payload.is_empty() {
        writer.write_all(payload).await?;
    }
    writer.flush().await?;
    Ok(())
}

fn parse_echo_command(cmd: &str) -> Option<String> {
    let raw_text = if cmd == "echo" {
        ""
    } else {
        cmd.strip_prefix("echo ")?
    };

    let mut result = String::new();
    let mut in_single_quote = false;
    let mut i = 0;
    let bytes = raw_text.as_bytes();
    while i < bytes.len() {
        if bytes[i] == b'\'' {
            if in_single_quote && i + 3 < bytes.len() && &bytes[i..i + 4] == b"'\\''" {
                result.push('\'');
                i += 4;
                continue;
            }
            in_single_quote = !in_single_quote;
            i += 1;
            continue;
        }
        result.push(bytes[i] as char);
        i += 1;
    }

    Some(format!("{result}\n"))
}

fn parse_stderr_command(cmd: &str) -> Option<String> {
    let raw_text = if cmd == "stderr" {
        ""
    } else {
        cmd.strip_prefix("stderr ")?
    };

    let text = raw_text.trim();
    let text = if (text.starts_with('"') && text.ends_with('"') && text.len() >= 2)
        || (text.starts_with('\'') && text.ends_with('\'') && text.len() >= 2)
    {
        &text[1..text.len() - 1]
    } else {
        text
    };

    Some(format!("{text}\n"))
}

fn parse_exit_command(cmd: &str) -> Option<u8> {
    if cmd == "exit" {
        return Some(0);
    }
    let arg = cmd.strip_prefix("exit ")?.trim();
    if arg.is_empty() {
        return Some(0);
    }
    arg.parse::<u8>()
        .ok()
        .or_else(|| arg.parse::<u32>().ok().map(|n| (n % 256) as u8))
}

pub async fn handle_connection(stream: TcpStream) -> std::io::Result<()> {
    handle_connection_with_options(stream, MockAdbdOptions::default()).await
}

pub async fn handle_connection_with_options(
    stream: TcpStream,
    options: MockAdbdOptions,
) -> std::io::Result<()> {
    let (mut reader, mut writer) = stream.into_split();

    // 1. Handshake: Read host CNXN packet
    let host_cnxn = loop {
        match read_packet(&mut reader).await {
            Ok(Some((header, payload))) => {
                if header.command == Command::Cnxn {
                    break (header, payload);
                }
            }
            Ok(None) => return Ok(()),
            Err(_) => return Ok(()),
        }
    };

    let host_max_payload = host_cnxn.0.arg1 as usize;
    let max_payload = negotiate_max_payload(host_max_payload, MAX_PAYLOAD);

    let mut features = Features::new();
    features.push(Feature::Unknown("exec".to_string()));
    if options.enable_shell_v2 {
        features.push(Feature::Shell2);
    }

    let banner = Banner {
        system_type: "device".to_string(),
        serial: "".to_string(),
        properties: vec![
            ("ro.product.name".to_string(), "mock_device".to_string()),
            ("ro.product.model".to_string(), "mock_model".to_string()),
            ("ro.product.device".to_string(), "mock_device".to_string()),
        ],
        features,
        ends_with_semicolon: true,
    };
    let banner_bytes = banner.to_bytes();

    if write_packet(
        &mut writer,
        Command::Cnxn,
        A_VERSION,
        max_payload as u32,
        &banner_bytes,
    )
    .await
    .is_err()
    {
        return Ok(());
    }

    let next_stream_id = Arc::new(AtomicU32::new(1));

    // 2. Stream loop
    loop {
        let (header, payload) = match read_packet(&mut reader).await {
            Ok(Some(pkt)) => pkt,
            Ok(None) => break,
            Err(_) => break,
        };

        match header.command {
            Command::Open => {
                let host_local_id = header.arg0;
                let service_raw = String::from_utf8_lossy(&payload);
                let service = service_raw.trim_matches('\0');

                if let Some(rest) = service.strip_prefix("shell,") {
                    let mock_local_id = next_stream_id.fetch_add(1, Ordering::SeqCst);

                    let (args_str, cmd_str) = match rest.find(':') {
                        Some(idx) => (&rest[..idx], &rest[idx + 1..]),
                        None => (rest, ""),
                    };

                    let args: Vec<&str> = args_str.split(',').collect();
                    if args.contains(&"v2") {
                        let is_pty = args.contains(&"pty");
                        let term_val = args.iter().find_map(|a| a.strip_prefix("TERM="));

                        if write_packet(
                            &mut writer,
                            Command::Okay,
                            mock_local_id,
                            host_local_id,
                            &[],
                        )
                        .await
                        .is_err()
                        {
                            break;
                        }

                        if handle_shell_v2_command(
                            &mut reader,
                            &mut writer,
                            mock_local_id,
                            host_local_id,
                            cmd_str,
                            is_pty,
                            term_val,
                        )
                        .await
                        .is_err()
                        {
                            break;
                        }
                        continue;
                    }
                }

                if let Some(cmd) = service.strip_prefix("shell:") {
                    let mock_local_id = next_stream_id.fetch_add(1, Ordering::SeqCst);

                    // Accept stream
                    if write_packet(
                        &mut writer,
                        Command::Okay,
                        mock_local_id,
                        host_local_id,
                        &[],
                    )
                    .await
                    .is_err()
                    {
                        break;
                    }

                    // Process shell command
                    let output = parse_echo_command(cmd);

                    let response_payload = match output {
                        Some(text) => text.into_bytes(),
                        None => b"mock-adbd: unsupported command\n".to_vec(),
                    };

                    // Send WRTE
                    if write_packet(
                        &mut writer,
                        Command::Wrte,
                        mock_local_id,
                        host_local_id,
                        &response_payload,
                    )
                    .await
                    .is_err()
                    {
                        break;
                    }

                    // Wait for OKAY from host (1-WRTE-in-flight rule)
                    let mut got_okay = false;
                    while !got_okay {
                        match read_packet(&mut reader).await {
                            Ok(Some((ack_header, _))) => {
                                if ack_header.command == Command::Okay
                                    && ack_header.arg0 == host_local_id
                                    && ack_header.arg1 == mock_local_id
                                {
                                    got_okay = true;
                                }
                            }
                            Ok(None) => return Ok(()),
                            Err(_) => return Ok(()),
                        }
                    }

                    // Send CLSE
                    if write_packet(
                        &mut writer,
                        Command::Clse,
                        mock_local_id,
                        host_local_id,
                        &[],
                    )
                    .await
                    .is_err()
                    {
                        break;
                    }
                } else if let Some(cmd) = service.strip_prefix("exec:") {
                    let mock_local_id = next_stream_id.fetch_add(1, Ordering::SeqCst);

                    if write_packet(
                        &mut writer,
                        Command::Okay,
                        mock_local_id,
                        host_local_id,
                        &[],
                    )
                    .await
                    .is_err()
                    {
                        break;
                    }

                    if let Some(echo_text) = parse_echo_command(cmd) {
                        let payload = echo_text.into_bytes();
                        if write_packet(
                            &mut writer,
                            Command::Wrte,
                            mock_local_id,
                            host_local_id,
                            &payload,
                        )
                        .await
                        .is_ok()
                        {
                            loop {
                                match read_packet(&mut reader).await {
                                    Ok(Some((ack, _)))
                                        if ack.command == Command::Okay
                                            && ack.arg0 == host_local_id
                                            && ack.arg1 == mock_local_id =>
                                    {
                                        break
                                    }
                                    Ok(None) | Err(_) => return Ok(()),
                                    _ => {}
                                }
                            }
                        }
                        let _ = write_packet(
                            &mut writer,
                            Command::Clse,
                            mock_local_id,
                            host_local_id,
                            &[],
                        )
                        .await;
                    } else if cmd.starts_with("cat") {
                        loop {
                            let (hdr, _) = match read_packet(&mut reader).await {
                                Ok(Some(pkt)) => pkt,
                                _ => break,
                            };
                            if hdr.command == Command::Wrte {
                                let _ = write_packet(
                                    &mut writer,
                                    Command::Okay,
                                    mock_local_id,
                                    host_local_id,
                                    &[],
                                )
                                .await;
                            } else if hdr.command == Command::Clse {
                                break;
                            }
                        }
                        let _ = write_packet(
                            &mut writer,
                            Command::Clse,
                            mock_local_id,
                            host_local_id,
                            &[],
                        )
                        .await;
                    } else {
                        let payload = b"mock-adbd: exec unsupported\n".to_vec();
                        let _ = write_packet(
                            &mut writer,
                            Command::Wrte,
                            mock_local_id,
                            host_local_id,
                            &payload,
                        )
                        .await;
                        let _ = write_packet(
                            &mut writer,
                            Command::Clse,
                            mock_local_id,
                            host_local_id,
                            &[],
                        )
                        .await;
                    }
                } else {
                    // Refuse unknown service with CLSE(0, host_local_id)
                    if write_packet(&mut writer, Command::Clse, 0, host_local_id, &[])
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
            }
            Command::Clse => {
                // Host requested close
            }
            _ => {
                // Ignore other packets
            }
        }
    }

    Ok(())
}

struct StreamCtx<'a, R, W> {
    reader: &'a mut R,
    writer: &'a mut W,
    mock_local_id: u32,
    host_local_id: u32,
    is_pty: bool,
}

impl<'a, R, W> StreamCtx<'a, R, W>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    async fn send_shell_packet_and_wait_okay(
        &mut self,
        packet: &ShellPacket,
        decoder: &mut ShellDecoder,
        pending_winsz: &mut VecDeque<ShellPacket>,
    ) -> std::io::Result<bool> {
        let encoded = match packet.encode_to_vec_checked() {
            Ok(bytes) => bytes,
            Err(_) => return Ok(false),
        };

        if write_packet(
            self.writer,
            Command::Wrte,
            self.mock_local_id,
            self.host_local_id,
            &encoded,
        )
        .await
        .is_err()
        {
            return Ok(false);
        }

        loop {
            let (header, payload) = match read_packet(self.reader).await {
                Ok(Some(pkt)) => pkt,
                _ => return Ok(false),
            };

            match header.command {
                Command::Okay => {
                    if header.arg0 == self.host_local_id && header.arg1 == self.mock_local_id {
                        return Ok(true);
                    }
                }
                Command::Wrte => {
                    if header.arg0 == self.host_local_id && header.arg1 == self.mock_local_id {
                        if write_packet(
                            self.writer,
                            Command::Okay,
                            self.mock_local_id,
                            self.host_local_id,
                            &[],
                        )
                        .await
                        .is_err()
                        {
                            return Ok(false);
                        }

                        decoder.feed(&payload);
                        loop {
                            match decoder.pop_packet() {
                                Ok(Some(pkt))
                                    if pkt.id == ShellPacketId::WindowSizeChange && self.is_pty =>
                                {
                                    let text = String::from_utf8_lossy(&pkt.data)
                                        .trim_end_matches('\0')
                                        .to_string();
                                    let winsz_pkt = ShellPacket::new(
                                        ShellPacketId::Stdout,
                                        format!("WINSZ {text}\n").into_bytes(),
                                    );
                                    pending_winsz.push_back(winsz_pkt);
                                }
                                Ok(Some(_)) => {}
                                Ok(None) => break,
                                Err(_) => return Ok(false),
                            }
                        }
                    }
                }
                Command::Clse => {
                    return Ok(false);
                }
                _ => {}
            }
        }
    }
}

async fn handle_shell_v2_cat<R, W>(
    ctx: &mut StreamCtx<'_, R, W>,
    decoder: &mut ShellDecoder,
) -> std::io::Result<()>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut pending_winsz = VecDeque::new();
    let mut stdin_closed = false;

    while !stdin_closed {
        while let Some(winsz_pkt) = pending_winsz.pop_front() {
            if !ctx
                .send_shell_packet_and_wait_okay(&winsz_pkt, decoder, &mut pending_winsz)
                .await?
            {
                return Ok(());
            }
        }

        let (header, payload) = match read_packet(ctx.reader).await {
            Ok(Some(pkt)) => pkt,
            _ => return Ok(()),
        };

        match header.command {
            Command::Wrte => {
                if header.arg0 == ctx.host_local_id && header.arg1 == ctx.mock_local_id {
                    if write_packet(
                        ctx.writer,
                        Command::Okay,
                        ctx.mock_local_id,
                        ctx.host_local_id,
                        &[],
                    )
                    .await
                    .is_err()
                    {
                        return Ok(());
                    }

                    decoder.feed(&payload);
                    loop {
                        let pkt = match decoder.pop_packet() {
                            Ok(Some(p)) => p,
                            Ok(None) => break,
                            Err(_) => return Ok(()),
                        };

                        match pkt.id {
                            ShellPacketId::Stdin => {
                                let stdout_pkt = ShellPacket::new(ShellPacketId::Stdout, pkt.data);
                                if !ctx
                                    .send_shell_packet_and_wait_okay(
                                        &stdout_pkt,
                                        decoder,
                                        &mut pending_winsz,
                                    )
                                    .await?
                                {
                                    return Ok(());
                                }
                            }
                            ShellPacketId::CloseStdin => {
                                stdin_closed = true;
                                break;
                            }
                            ShellPacketId::WindowSizeChange if ctx.is_pty => {
                                let text = String::from_utf8_lossy(&pkt.data)
                                    .trim_end_matches('\0')
                                    .to_string();
                                let winsz_pkt = ShellPacket::new(
                                    ShellPacketId::Stdout,
                                    format!("WINSZ {text}\n").into_bytes(),
                                );
                                if !ctx
                                    .send_shell_packet_and_wait_okay(
                                        &winsz_pkt,
                                        decoder,
                                        &mut pending_winsz,
                                    )
                                    .await?
                                {
                                    return Ok(());
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
            Command::Clse => {
                return Ok(());
            }
            _ => {}
        }
    }

    while let Some(winsz_pkt) = pending_winsz.pop_front() {
        if !ctx
            .send_shell_packet_and_wait_okay(&winsz_pkt, decoder, &mut pending_winsz)
            .await?
        {
            return Ok(());
        }
    }

    let exit_pkt = ShellPacket::exit_code(0);
    let _ = ctx
        .send_shell_packet_and_wait_okay(&exit_pkt, decoder, &mut pending_winsz)
        .await?;

    let _ = write_packet(
        ctx.writer,
        Command::Clse,
        ctx.mock_local_id,
        ctx.host_local_id,
        &[],
    )
    .await;

    Ok(())
}

async fn handle_shell_v2_command<R, W>(
    reader: &mut R,
    writer: &mut W,
    mock_local_id: u32,
    host_local_id: u32,
    cmd: &str,
    is_pty: bool,
    term_val: Option<&str>,
) -> std::io::Result<()>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut decoder = ShellDecoder::new();
    let mut pending_winsz = VecDeque::new();
    let mut ctx = StreamCtx {
        reader,
        writer,
        mock_local_id,
        host_local_id,
        is_pty,
    };

    if cmd == "close-no-exit" {
        let _ = write_packet(
            ctx.writer,
            Command::Clse,
            ctx.mock_local_id,
            ctx.host_local_id,
            &[],
        )
        .await;
        return Ok(());
    }

    if cmd == "cat" {
        return handle_shell_v2_cat(&mut ctx, &mut decoder).await;
    }

    let (output_pkt, exit_code) = if let Some(echo_text) = parse_echo_command(cmd) {
        (
            Some(ShellPacket::new(
                ShellPacketId::Stdout,
                echo_text.into_bytes(),
            )),
            0,
        )
    } else if let Some(stderr_text) = parse_stderr_command(cmd) {
        (
            Some(ShellPacket::new(
                ShellPacketId::Stderr,
                stderr_text.into_bytes(),
            )),
            0,
        )
    } else if let Some(code) = parse_exit_command(cmd) {
        (None, code)
    } else if cmd == "env-term" {
        let term_text = format!("{}\n", term_val.unwrap_or("unset"));
        (
            Some(ShellPacket::new(
                ShellPacketId::Stdout,
                term_text.into_bytes(),
            )),
            0,
        )
    } else {
        (
            Some(ShellPacket::new(
                ShellPacketId::Stderr,
                b"mock-adbd: unsupported command\n".to_vec(),
            )),
            127,
        )
    };

    if let Some(pkt) = output_pkt {
        if !ctx
            .send_shell_packet_and_wait_okay(&pkt, &mut decoder, &mut pending_winsz)
            .await?
        {
            return Ok(());
        }
    }

    while let Some(winsz_pkt) = pending_winsz.pop_front() {
        if !ctx
            .send_shell_packet_and_wait_okay(&winsz_pkt, &mut decoder, &mut pending_winsz)
            .await?
        {
            return Ok(());
        }
    }

    let exit_pkt = ShellPacket::exit_code(exit_code);
    let _ = ctx
        .send_shell_packet_and_wait_okay(&exit_pkt, &mut decoder, &mut pending_winsz)
        .await?;

    let _ = write_packet(
        ctx.writer,
        Command::Clse,
        ctx.mock_local_id,
        ctx.host_local_id,
        &[],
    )
    .await;

    Ok(())
}
