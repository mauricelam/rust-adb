use std::io::{self, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

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
    let server_stream = TcpStream::connect("127.0.0.1:5037")?;

    // MITM bi-directional forwarding
    let mut client_reader = client_stream.try_clone()?;
    let mut server_reader = server_stream.try_clone()?;

    let mut client_writer = client_stream;
    let mut server_writer = server_stream;

    let t1 = thread::spawn(move || {
        let mut x = || -> std::io::Result<()> {
            let mut len_buf = [0u8; 4];
            client_reader.read_exact(&mut len_buf)?;

            let len_str = std::str::from_utf8(&len_buf)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
            let len = u32::from_str_radix(len_str, 16)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

            let mut msg_buf = vec![0u8; len as usize];
            client_reader.read_exact(&mut msg_buf)?;

            let msg = String::from_utf8_lossy(&msg_buf).to_string();
            let _ = tx.send(msg);

            // Forward the initial command
            server_writer.write_all(&len_buf)?;
            server_writer.write_all(&msg_buf)?;

            Ok(())
        };
        x().unwrap();
    });

    let t2 = thread::spawn(move || {
        let _ = io::copy(&mut server_reader, &mut client_writer);
    });

    let _ = t1.join();
    let _ = t2.join();

    Ok(())
}
