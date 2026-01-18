use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

#[derive(Debug)]
pub enum Traffic {
    FromClient(Vec<u8>),
    FromServer(Vec<u8>),
}

pub fn start_mock_server(
    upstream_port: u16,
) -> std::io::Result<(u16, Receiver<Traffic>, thread::JoinHandle<()>)> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();

    let (tx, rx) = mpsc::channel();

    let jh = thread::spawn(move || {
        for stream in listener.incoming() {
            if let Ok(stream) = stream {
                let tx_clone = tx.clone();
                thread::spawn(move || {
                    let _ = handle_connection(stream, upstream_port, tx_clone);
                });
            } else {
                break;
            }
        }
    });

    Ok((port, rx, jh))
}

fn handle_connection(
    mut client_stream: TcpStream,
    upstream_port: u16,
    tx: Sender<Traffic>,
) -> std::io::Result<()> {
    let mut server_stream = TcpStream::connect(format!("127.0.0.1:{}", upstream_port))?;

    let mut client_reader = client_stream.try_clone()?;
    let mut server_reader = server_stream.try_clone()?;

    let tx_clone = tx.clone();
    thread::spawn(move || {
        let mut buffer = [0; 1024];
        loop {
            match client_reader.read(&mut buffer) {
                Ok(0) => break,
                Ok(n) => {
                    if server_stream.write_all(&buffer[..n]).is_err() {
                        break;
                    }
                    let _ = tx_clone.send(Traffic::FromClient(buffer[..n].to_vec()));
                }
                Err(_) => break,
            }
        }
    });

    thread::spawn(move || {
        let mut buffer = [0; 1024];
        loop {
            match server_reader.read(&mut buffer) {
                Ok(0) => break,
                Ok(n) => {
                    if client_stream.write_all(&buffer[..n]).is_err() {
                        break;
                    }
                    let _ = tx.send(Traffic::FromServer(buffer[..n].to_vec()));
                }
                Err(_) => break,
            }
        }
    });

    Ok(())
}
