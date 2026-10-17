use clap::Parser;
use serde::{Deserialize, Serialize};
use std::future::Future;
use std::io;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Instant;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufWriter};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;

#[derive(Parser, Debug)]
#[command(author, version, about = "Logging TCP proxy for recording ADB traffic")]
pub struct Args {
    /// Socket address to listen on
    #[arg(short, long)]
    pub listen: String,

    /// Socket address to forward traffic to
    #[arg(short, long)]
    pub target: String,

    /// File path to write recorded JSON Lines traffic
    #[arg(short, long)]
    pub out: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    ClientToServer,
    ServerToClient,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Record {
    pub direction: Direction,
    pub timestamp_us: u64,
    pub bytes: String,
}

pub struct ProxyServer {
    listener: TcpListener,
    target_addr: String,
    out_path: PathBuf,
}

impl ProxyServer {
    pub async fn bind(
        listen_addr: &str,
        target_addr: impl Into<String>,
        out_path: impl Into<PathBuf>,
    ) -> io::Result<Self> {
        let listener = TcpListener::bind(listen_addr).await?;
        Ok(Self {
            listener,
            target_addr: target_addr.into(),
            out_path: out_path.into(),
        })
    }

    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.listener.local_addr()
    }

    pub async fn run(self) -> io::Result<()> {
        self.run_until(std::future::pending()).await
    }

    pub async fn run_until(self, shutdown: impl Future<Output = ()>) -> io::Result<()> {
        let (record_tx, mut record_rx) = mpsc::unbounded_channel::<Record>();
        let out_path = self.out_path.clone();

        let writer_handle = tokio::spawn(async move {
            let file = tokio::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&out_path)
                .await?;
            let mut writer = BufWriter::new(file);

            while let Some(record) = record_rx.recv().await {
                let line = serde_json::to_string(&record)
                    .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
                writer.write_all(line.as_bytes()).await?;
                writer.write_all(b"\n").await?;
                writer.flush().await?;
            }
            writer.flush().await?;
            Ok::<(), io::Error>(())
        });

        let start_time = Instant::now();
        let target_addr = self.target_addr;
        let listener = self.listener;

        tokio::select! {
            _ = shutdown => {},
            res = async {
                loop {
                    let (client_stream, _) = listener.accept().await?;
                    let target_addr = target_addr.clone();
                    let record_tx = record_tx.clone();
                    tokio::spawn(async move {
                        if let Err(e) = handle_connection(client_stream, target_addr, record_tx, start_time).await {
                            eprintln!("Connection error: {e}");
                        }
                    });
                }
                #[allow(unreachable_code)]
                Ok::<(), io::Error>(())
            } => {
                if let Err(e) = res {
                    eprintln!("Listener error: {e}");
                }
            }
        }

        drop(record_tx);
        if let Ok(res) = writer_handle.await {
            res?;
        }

        Ok(())
    }
}

async fn handle_connection(
    mut client_stream: TcpStream,
    target_addr: String,
    record_tx: mpsc::UnboundedSender<Record>,
    start_time: Instant,
) -> io::Result<()> {
    let mut target_stream = TcpStream::connect(target_addr).await?;

    let (client_read, client_write) = client_stream.split();
    let (target_read, target_write) = target_stream.split();

    let c2s = copy_and_record(
        client_read,
        target_write,
        Direction::ClientToServer,
        start_time,
        record_tx.clone(),
    );

    let s2c = copy_and_record(
        target_read,
        client_write,
        Direction::ServerToClient,
        start_time,
        record_tx,
    );

    tokio::try_join!(c2s, s2c)?;
    Ok(())
}

async fn copy_and_record<R, W>(
    mut reader: R,
    mut writer: W,
    direction: Direction,
    start_time: Instant,
    record_tx: mpsc::UnboundedSender<Record>,
) -> io::Result<()>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut buf = [0u8; 8192];
    loop {
        let n = reader.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        let chunk = &buf[..n];
        let timestamp_us = start_time.elapsed().as_micros() as u64;
        let bytes_hex = hex::encode(chunk);
        let record = Record {
            direction,
            timestamp_us,
            bytes: bytes_hex,
        };
        let _ = record_tx.send(record);

        writer.write_all(chunk).await?;
        writer.flush().await?;
    }
    let _ = writer.shutdown().await;
    Ok(())
}
