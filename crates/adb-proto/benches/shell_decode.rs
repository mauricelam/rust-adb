use adb_proto::shell::{ShellDecoder, ShellPacket, ShellPacketId};
use std::time::Instant;

fn main() {
    println!("Preparing benchmark dataset: 64 MiB total stdout packets (64 x 1 MiB) split into 4 KiB chunks...");

    const ONE_MIB: usize = 1024 * 1024;
    const TOTAL_PACKETS: usize = 64;
    const CHUNK_SIZE: usize = 4096;

    let packet_payload = vec![b'A'; ONE_MIB];
    let packet = ShellPacket::new(ShellPacketId::Stdout, packet_payload);
    let encoded = packet.encode().unwrap();

    let mut full_stream = Vec::with_capacity(encoded.len() * TOTAL_PACKETS);
    for _ in 0..TOTAL_PACKETS {
        full_stream.extend_from_slice(&encoded);
    }

    let chunks: Vec<&[u8]> = full_stream.chunks(CHUNK_SIZE).collect();

    println!("Starting decoding benchmark...");
    let start = Instant::now();

    let mut decoder = ShellDecoder::new();
    let mut decoded_count = 0;

    for chunk in &chunks {
        decoder.feed(chunk);
        while let Ok(Some(pkt)) = decoder.pop_packet() {
            decoded_count += 1;
            assert_eq!(pkt.id, ShellPacketId::Stdout);
            assert_eq!(pkt.data.len(), ONE_MIB);
        }
    }

    let elapsed = start.elapsed();
    let total_bytes = full_stream.len();
    let throughput_mb_s = (total_bytes as f64 / (1024.0 * 1024.0)) / elapsed.as_secs_f64();

    println!(
        "Decoded {} packets ({} bytes) in {:.4?} ({:.2} MiB/s)",
        decoded_count, total_bytes, elapsed, throughput_mb_s
    );
}
