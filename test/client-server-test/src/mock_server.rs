use std::io::{self, Read, Write};
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

fn handle_connection(client_stream: TcpStream, tx: Sender<String>) -> std::io::Result<()> {
    let timeout = Some(Duration::from_secs(10));
    let _ = client_stream.set_read_timeout(timeout);
    let _ = client_stream.set_write_timeout(timeout);

    let server_stream_res = TcpStream::connect("127.0.0.1:5037");
    if let Ok(ref s) = server_stream_res {
        let _ = s.set_read_timeout(timeout);
        let _ = s.set_write_timeout(timeout);
    }

    // MITM bi-directional forwarding
    let mut client_reader = client_stream.try_clone()?;
    let server_reader = server_stream_res.as_ref().ok().and_then(|s| s.try_clone().ok());

    let mut client_writer1 = client_stream.try_clone()?;
    let mut client_writer2 = client_stream;
    let mut server_writer = server_stream_res.ok();

    let t1 = thread::spawn(move || {
        let mut x = || -> std::io::Result<()> {
            loop {
                let mut len_buf = [0u8; 4];
                if let Err(e) = client_reader.read_exact(&mut len_buf) {
                    if e.kind() == io::ErrorKind::UnexpectedEof {
                        break;
                    }
                    return Err(e);
                }

                let len_str = std::str::from_utf8(&len_buf)
                    .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
                let len = u32::from_str_radix(len_str, 16)
                    .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

                let mut msg_buf = vec![0u8; len as usize];
                client_reader.read_exact(&mut msg_buf)?;

                let msg = String::from_utf8_lossy(&msg_buf).to_string();
                let _ = tx.send(msg.clone());

                if let Some(ref mut server_writer) = server_writer {
                    // Forward the command
                    let _ = server_writer.write_all(&len_buf);
                    let _ = server_writer.write_all(&msg_buf);
                } else {
                    // Mock responses for CI environments where ADB server might be missing
                    if msg == "host:version" {
                        client_writer1.write_all(b"OKAY00040029")?;
                    } else if msg == "host:features" || msg == "host:host-features" {
                        client_writer1.write_all(b"OKAY0008remount,")?;
                    } else if msg.contains("remount") {
                        client_writer1.write_all(b"OKAY")?;
                        client_writer1.write_all(b"remount succeeded")?;
                    } else {
                        client_writer1.write_all(b"FAIL000Enot connected")?;
                    }
                }
            }
            Ok(())
        };
        let _ = x();
    });

    let t2 = thread::spawn(move || {
        if let Some(mut server_reader) = server_reader {
            let _ = io::copy(&mut server_reader, &mut client_writer2);
        }
    });

    let _ = t1.join();
    let _ = t2.join();

    Ok(())
}
