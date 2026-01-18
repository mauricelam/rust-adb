use std::net::{TcpListener, TcpStream};
use std::io::{Read, Write};
use std::thread;
use std::sync::mpsc::{self, Sender, Receiver};

pub fn start_mock_server() -> std::io::Result<(u16, Receiver<String>)> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();

    let (tx, rx) = mpsc::channel();

    thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            handle_connection(&mut stream, tx);
        }
    });

    Ok((port, rx))
}

fn handle_connection(stream: &mut TcpStream, tx: Sender<String>) {
    let mut len_buf = [0u8; 4];
    if stream.read_exact(&mut len_buf).is_err() {
        return;
    }

    let len = match std::str::from_utf8(&len_buf) {
        Ok(s) => match u32::from_str_radix(s, 16) {
            Ok(val) => val,
            Err(_) => return,
        },
        Err(_) => return,
    };

    let mut msg_buf = vec![0u8; len as usize];
    if stream.read_exact(&mut msg_buf).is_err() {
        return;
    }

    let msg = String::from_utf8_lossy(&msg_buf).to_string();

    // The receiver may have been dropped, so we ignore the error.
    let _ = tx.send(msg);

    // Send a valid response for `host:devices`.
    // The format is "OKAY" followed by a 4-byte hex length, then the payload.
    // An empty list of devices is an empty payload.
    let response = b"OKAY0000";
    let _ = stream.write_all(response);
}
