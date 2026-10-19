use adb_proto::{
    build_banner, calculate_apacket_checksum, Amessage, Apacket, A_CLSE, A_CNXN, A_OKAY, A_OPEN,
    A_VERSION, A_WRTE, MAX_PAYLOAD,
};
use std::io;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU32, Ordering};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

static NEXT_STREAM_ID: AtomicU32 = AtomicU32::new(1);

fn next_stream_id() -> u32 {
    NEXT_STREAM_ID.fetch_add(1, Ordering::Relaxed)
}

pub struct MockAdbdServer {
    listener: TcpListener,
    local_addr: SocketAddr,
}

impl MockAdbdServer {
    pub async fn bind(addr: &str) -> io::Result<Self> {
        let listener = TcpListener::bind(addr).await?;
        let local_addr = listener.local_addr()?;
        Ok(Self {
            listener,
            local_addr,
        })
    }

    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    pub async fn run(self) -> io::Result<()> {
        eprintln!("mock-adbd: server running on {}", self.local_addr);
        loop {
            match self.listener.accept().await {
                Ok((stream, addr)) => {
                    eprintln!("mock-adbd: accepted connection from {addr}");
                    tokio::spawn(async move {
                        if let Err(e) = handle_connection(stream).await {
                            eprintln!("mock-adbd: connection error: {e}");
                        }
                    });
                }
                Err(e) => {
                    eprintln!("mock-adbd: accept error: {e}");
                    return Err(e);
                }
            }
        }
    }
}

pub fn strip_trailing_nulls(bytes: &[u8]) -> &[u8] {
    let mut end = bytes.len();
    while end > 0 && bytes[end - 1] == 0 {
        end -= 1;
    }
    &bytes[..end]
}

pub async fn read_packet<R: AsyncReadExt + Unpin>(reader: &mut R) -> io::Result<Option<Apacket>> {
    let mut header_bytes = [0u8; Amessage::HEADER_SIZE];
    match reader.read_exact(&mut header_bytes).await {
        Ok(_) => {}
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }

    let msg = Amessage::from_le_bytes(&header_bytes);
    if !msg.is_valid() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid header magic",
        ));
    }
    if msg.data_length > MAX_PAYLOAD {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "payload length exceeds MAX_PAYLOAD",
        ));
    }

    let mut payload = vec![0u8; msg.data_length as usize];
    if msg.data_length > 0 {
        match reader.read_exact(&mut payload).await {
            Ok(_) => {}
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "truncated payload",
                ))
            }
            Err(e) => return Err(e),
        }
    }

    if msg.data_crc32 != 0 && calculate_apacket_checksum(&payload) != msg.data_crc32 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "checksum mismatch",
        ));
    }

    Ok(Some(Apacket { msg, payload }))
}

pub async fn write_packet<W: AsyncWriteExt + Unpin>(
    writer: &mut W,
    packet: &Apacket,
) -> io::Result<()> {
    writer.write_all(&packet.msg.to_le_bytes()).await?;
    if !packet.payload.is_empty() {
        writer.write_all(&packet.payload).await?;
    }
    writer.flush().await?;
    Ok(())
}

async fn handle_connection(mut stream: TcpStream) -> io::Result<()> {
    eprintln!("mock-adbd: accepted new TCP connection");
    // 1. Handshake Phase
    // Loop until we receive A_CNXN (ignore any preceding packets)
    let _host_cnxn = loop {
        match read_packet(&mut stream).await {
            Ok(Some(pkt)) if pkt.msg.command == A_CNXN => {
                eprintln!("mock-adbd: received host A_CNXN: {:?}", pkt.msg);
                break pkt;
            }
            Ok(Some(pkt)) => {
                eprintln!(
                    "mock-adbd: ignoring non-CNXN packet during handshake: {:?}",
                    pkt.msg.command
                );
                continue;
            }
            Ok(None) => {
                eprintln!("mock-adbd: stream closed before CNXN");
                return Ok(());
            }
            Err(e) => {
                eprintln!("mock-adbd: read error during handshake: {e}");
                return Err(e);
            }
        }
    };

    // Reply with device A_CNXN
    let banner = build_banner(
        "device",
        &[
            ("ro.product.name", "mock"),
            ("ro.product.model", "mock_model"),
            ("ro.product.device", "mock_device"),
        ],
        &[], // No features advertised (no shell_v2)
    );
    let cnxn_response = Apacket::new(A_CNXN, A_VERSION, MAX_PAYLOAD, banner.into_bytes());
    write_packet(&mut stream, &cnxn_response).await?;

    // 2. Stream Handling Phase
    while let Some(packet) = read_packet(&mut stream).await? {
        match packet.msg.command {
            A_OPEN => {
                let host_id = packet.msg.arg0;
                let payload_bytes = strip_trailing_nulls(&packet.payload);
                let service = String::from_utf8_lossy(payload_bytes);

                if let Some(cmd) = service.strip_prefix("shell:") {
                    let mock_id = next_stream_id();

                    // Reply A_OKAY
                    let okay = Apacket::new(A_OKAY, mock_id, host_id, vec![]);
                    write_packet(&mut stream, &okay).await?;

                    // Determine output
                    let response_text = if cmd == "echo" {
                        "\n".to_string()
                    } else if let Some(text) = cmd.strip_prefix("echo ") {
                        format!("{text}\n")
                    } else if let Some(text) = cmd.strip_prefix("echo\t") {
                        format!("{text}\n")
                    } else {
                        "mock-adbd: unsupported command\n".to_string()
                    };

                    // Send single A_WRTE
                    let wrte = Apacket::new(A_WRTE, mock_id, host_id, response_text.into_bytes());
                    write_packet(&mut stream, &wrte).await?;

                    // Honor 1-WRTE-in-flight rule: wait for host A_OKAY or A_CLSE
                    if let Some(host_ack) = read_packet(&mut stream).await? {
                        if host_ack.msg.command == A_OKAY || host_ack.msg.command == A_CLSE {
                            let clse = Apacket::new(A_CLSE, mock_id, host_id, vec![]);
                            write_packet(&mut stream, &clse).await?;
                        }
                    }
                } else {
                    // Unknown service: refuse stream with A_CLSE(0, host_id)
                    let clse = Apacket::new(A_CLSE, 0, host_id, vec![]);
                    write_packet(&mut stream, &clse).await?;
                }
            }
            A_CLSE => {
                // Stream closed by host
            }
            A_WRTE => {
                let host_id = packet.msg.arg0;
                let mock_id = packet.msg.arg1;
                let okay = Apacket::new(A_OKAY, mock_id, host_id, vec![]);
                write_packet(&mut stream, &okay).await?;
            }
            _ => {
                // Ignore unexpected packet in stream loop
            }
        }
    }

    Ok(())
}
