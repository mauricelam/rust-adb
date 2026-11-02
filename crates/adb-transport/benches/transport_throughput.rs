use adb_proto::{Command, MAX_PAYLOAD};
use adb_transport::{AsyncStreamTransport, Packet, Transport};
use std::time::Instant;
use tokio::io::duplex;

/// Measures throughput of streaming large payload data (50 MiB) over local async pipe.
#[tokio::main]
async fn main() {
    const TOTAL_BYTES: usize = 50 * 1024 * 1024; // 50 MiB
    const CHUNK_SIZE: usize = MAX_PAYLOAD; // 1 MiB chunks matching negotiated max payload size

    println!(
        "Benchmarking transport throughput: streaming {} MiB over async duplex stream in {} KiB chunks...",
        TOTAL_BYTES / (1024 * 1024),
        CHUNK_SIZE / 1024
    );

    // Duplex buffer sized to 2 * MAX_PAYLOAD to allow pipeline buffering
    let (pipe_client, pipe_server) = duplex(2 * MAX_PAYLOAD);
    let mut client_transport = AsyncStreamTransport::new(pipe_client);
    let mut server_transport = AsyncStreamTransport::new(pipe_server);
    client_transport.set_skip_checksum(true);
    server_transport.set_skip_checksum(true);

    let num_chunks = TOTAL_BYTES / CHUNK_SIZE;
    let payload = vec![0x5au8; CHUNK_SIZE];

    let start = Instant::now();

    let writer_task = tokio::spawn(async move {
        let mut stall_cycles = 0usize;
        for i in 0..num_chunks {
            let write_start = Instant::now();
            let packet = Packet::new(Command::Wrte, 1, 1, payload.clone(), true);
            client_transport.write_packet(&packet).await.unwrap();
            // A stall cycle occurs if a single write takes longer than 100ms
            if write_start.elapsed().as_millis() > 100 {
                stall_cycles += 1;
            }
            if (i + 1) % 10 == 0 || i + 1 == num_chunks {
                print!(".");
            }
        }
        println!();
        stall_cycles
    });

    let reader_task = tokio::spawn(async move {
        let mut received_bytes = 0usize;
        while received_bytes < TOTAL_BYTES {
            let packet = server_transport.read_packet().await.unwrap();
            assert_eq!(packet.header.command, Command::Wrte);
            received_bytes += packet.payload.len();
        }
        received_bytes
    });

    let (writer_res, reader_res) = tokio::join!(writer_task, reader_task);
    let stall_cycles = writer_res.expect("writer task failed");
    let received_bytes = reader_res.expect("reader task failed");

    let elapsed = start.elapsed();
    let throughput_mib_s = (received_bytes as f64 / (1024.0 * 1024.0)) / elapsed.as_secs_f64();

    println!(
        "Transferred {} bytes ({} MiB) in {:.4?} ({:.2} MiB/s, {} stall cycles)",
        received_bytes,
        received_bytes / (1024 * 1024),
        elapsed,
        throughput_mib_s,
        stall_cycles
    );

    assert_eq!(received_bytes, TOTAL_BYTES);
    assert_eq!(stall_cycles, 0, "Throughput demonstrated stall cycles!");
}
