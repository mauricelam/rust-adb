use std::io::{Read, Write};
use std::os::unix::io::OwnedFd;
use std::fs::File;
use crate::incremental_utils::K_BLOCK_SIZE;

const INCR_MAGIC: u32 = 0x494e4352;

const REQ_SERVING_COMPLETE: i16 = 0;
const REQ_BLOCK_MISSING: i16 = 1;
const REQ_PREFETCH: i16 = 2;
const REQ_DESTROY: i16 = 3;

const TYPE_DATA: i8 = 0;
// const TYPE_HASH: i8 = 1;

const COMPRESSION_NONE: i8 = 0;

#[repr(C, packed)]
struct RequestCommand {
    request_type: i16,
    file_id: i16,
    block_idx: i32,
}

#[repr(C, packed)]
struct ResponseHeader {
    file_id: i16,
    block_type: i8,
    compression_type: i8,
    block_idx: i32,
    block_size: i16,
}

struct IncrementalFile {
    _path: String,
    id: i16,
    size: u64,
    file: File,
    sent_blocks: Vec<bool>,
}

/// Server for incremental installation.
pub struct IncrementalServer {
    adb_fd: OwnedFd,
    files: Vec<IncrementalFile>,
}

impl IncrementalServer {
    /// Creates a new `IncrementalServer`.
    pub fn new(adb_fd: OwnedFd, file_paths: &[String]) -> anyhow::Result<Self> {
        let mut files = Vec::new();
        for (i, path) in file_paths.iter().enumerate() {
            let file = File::open(path)?;
            let size = file.metadata()?.len();
            let num_blocks = (size + K_BLOCK_SIZE as u64 - 1) / K_BLOCK_SIZE as u64;
            files.push(IncrementalFile {
                _path: path.clone(),
                id: i as i16,
                size,
                file,
                sent_blocks: vec![false; num_blocks as usize],
            });
        }
        Ok(Self { adb_fd, files })
    }

    /// Starts serving block requests.
    pub fn serve(&mut self) -> anyhow::Result<()> {
        let mut adb_file = unsafe { std::fs::File::from_raw_fd(self.adb_fd.as_raw_fd()) };
        let mut buffer = Vec::new();

        loop {
            let mut chunk = [0u8; 1024];
            let n = adb_file.read(&mut chunk)?;
            if n == 0 { break; }
            buffer.extend_from_slice(&chunk[..n]);

            while buffer.len() >= 12 { // Magic(4) + RequestCommand(8)
                let mut found_magic = false;
                for i in 0..buffer.len() - 4 {
                    let magic = u32::from_be_bytes(buffer[i..i+4].try_into().unwrap());
                    if magic == INCR_MAGIC {
                        if buffer.len() >= i + 12 {
                            let req_type = i16::from_be_bytes(buffer[i+4..i+6].try_into().unwrap());
                            let file_id = i16::from_be_bytes(buffer[i+6..i+8].try_into().unwrap());
                            let block_idx = i32::from_be_bytes(buffer[i+8..i+12].try_into().unwrap());

                            self.handle_request(req_type, file_id, block_idx, &mut adb_file)?;
                            buffer.drain(0..i+12);
                            found_magic = true;
                            break;
                        } else {
                            // Wait for more data
                            break;
                        }
                    }
                }
                if !found_magic {
                    buffer.clear();
                    break;
                }
            }
        }
        Ok(())
    }

    fn handle_request(&mut self, req_type: i16, file_id: i16, block_idx: i32, adb_file: &mut File) -> anyhow::Result<()> {
        match req_type {
            REQ_DESTROY => {
                anyhow::bail!("Destroy request received");
            }
            REQ_BLOCK_MISSING => {
                self.send_block(file_id, block_idx, adb_file)?;
            }
            REQ_PREFETCH => {
                // Prefetching not fully implemented yet
            }
            REQ_SERVING_COMPLETE => {
                return Ok(());
            }
            _ => {}
        }
        Ok(())
    }

    fn send_block(&mut self, file_id: i16, block_idx: i32, adb_file: &mut File) -> anyhow::Result<()> {
        let file_idx = file_id as usize;
        if file_idx >= self.files.len() { return Ok(()); }
        let file = &mut self.files[file_idx];
        if block_idx as usize >= file.sent_blocks.len() { return Ok(()); }

        let offset = block_idx as u64 * K_BLOCK_SIZE as u64;
        let mut data = vec![0u8; K_BLOCK_SIZE as usize];
        use std::os::unix::fs::FileExt;
        let n = file.file.read_at(&mut data, offset)?;
        let data = &data[..n];

        let header = ResponseHeader {
            file_id: file_id.to_be(),
            block_type: TYPE_DATA,
            compression_type: COMPRESSION_NONE,
            block_idx: block_idx.to_be(),
            block_size: (n as i16).to_be(),
        };

        let header_bytes: [u8; 10] = unsafe { std::mem::transmute(header) };
        let total_size = (header_bytes.len() + data.len()) as i32;

        adb_file.write_all(&total_size.to_be_bytes())?;
        adb_file.write_all(&header_bytes)?;
        adb_file.write_all(data)?;

        file.sent_blocks[block_idx as usize] = true;
        Ok(())
    }
}

#[cfg(unix)]
use std::os::unix::io::AsRawFd;
#[cfg(unix)]
use std::os::unix::io::FromRawFd;
