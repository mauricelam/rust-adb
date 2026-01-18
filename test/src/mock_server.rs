use std::io::{self, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

pub fn start_mock_server() -> std::io::Result<(u16, Receiver<String>)> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();

    let (tx, rx) = mpsc::channel();

    thread::spawn(move || {
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

    Ok((port, rx))
}

fn handle_connection(client_stream: TcpStream, tx: Sender<String>) -> std::io::Result<()> {
    let server_stream = TcpStream::connect("127.0.0.1:5037")?;

    // MITM bi-directional forwarding
    let mut client_reader = client_stream.try_clone()?;
    let mut server_reader = server_stream.try_clone()?;

    let mut client_writer = client_stream;
    let mut server_writer = server_stream;

    let t1 = thread::spawn(move || {
        // This thread reads from the client and forwards to the server.
        loop {
            let mut len_buf = [0u8; 4];
            // read_exact will return an error if the client closes the connection,
            // which is how we break the loop.
            if client_reader.read_exact(&mut len_buf).is_err() {
                break;
            }

            let len_str = match std::str::from_utf8(&len_buf) {
                Ok(s) => s,
                Err(_) => break, // Invalid UTF8
            };
            let len = match u32::from_str_radix(len_str, 16) {
                Ok(l) => l,
                Err(_) => break, // Invalid hex
            };

            let mut msg_buf = vec![0u8; len as usize];
            if client_reader.read_exact(&mut msg_buf).is_err() {
                break;
            }

            let msg = String::from_utf8_lossy(&msg_buf).to_string();
            // Send the captured message to the test thread for assertion.
            if tx.send(msg).is_err() {
                // Receiver has been dropped, test is likely over.
                break;
            }

            // Forward the message to the real adb server.
            if server_writer.write_all(&len_buf).is_err() {
                break;
            }
            if server_writer.write_all(&msg_buf).is_err() {
                break;
            }
        }
    });

    let t2 = thread::spawn(move || {
        // This thread reads from the server and forwards to the client.
        // It will finish when the server closes its end of the connection.
        let _ = io::copy(&mut server_reader, &mut client_writer);
    });

    // Wait for both threads to finish.
    let _ = t1.join();
    let _ = t2.join();

    Ok(())
}
