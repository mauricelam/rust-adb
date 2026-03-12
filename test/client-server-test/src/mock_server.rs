use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Duration;

pub fn start_mock_server() -> std::io::Result<(u16, Receiver<String>, thread::JoinHandle<()>)> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();

    let (tx, rx) = mpsc::channel();

    let jh = thread::spawn(move || {
        for stream in listener.incoming() {
            if let Ok(stream) = stream {
                let tx_clone = tx.clone();
                thread::spawn(move || {
                    let _ = handle_connection(stream, tx_clone);
                });
            } else {
                break;
            }
        }
    });

    Ok((port, rx, jh))
}

fn handle_connection(mut stream: TcpStream, tx: Sender<String>) -> std::io::Result<()> {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let mut reader = stream.try_clone()?;

    loop {
        let mut len_buf = [0u8; 4];
        if reader.read_exact(&mut len_buf).is_err() {
            break;
        }

        let len_str = match std::str::from_utf8(&len_buf) {
            Ok(s) => s,
            Err(_) => break,
        };
        let len = match u32::from_str_radix(len_str, 16) {
            Ok(l) => l,
            Err(_) => break,
        };

        let mut msg_buf = vec![0u8; len as usize];
        if reader.read_exact(&mut msg_buf).is_err() {
            break;
        }

        let msg = String::from_utf8_lossy(&msg_buf).to_string();
        let _ = tx.send(msg.clone());

        if msg == "host:version" {
            let _ = stream.write_all(b"OKAY00040029");
        } else if msg.starts_with("host:features") || msg.starts_with("host:host-features") {
            let _ = stream.write_all(b"OKAY0008remount,");
        } else if msg.starts_with("host:devices") {
            let _ = stream.write_all(b"OKAY0000");
        } else if msg.starts_with("host:track-devices") {
            let _ = stream.write_all(b"OKAY");
        } else if msg.starts_with("host:transport") || msg.starts_with("host:tport") {
            let _ = stream.write_all(b"OKAY");
        } else if msg.contains("remount") {
            let _ = stream.write_all(b"OKAY");
        } else {
            let _ = stream.write_all(b"FAIL000Enot connected");
            break;
        }
    }
    let _ = stream.shutdown(std::net::Shutdown::Both);
    Ok(())
}
