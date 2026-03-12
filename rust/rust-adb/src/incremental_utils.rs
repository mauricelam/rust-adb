use std::io::Read;
use std::fs::File;
use std::path::Path;

/// Block size for incremental installation.
pub const K_BLOCK_SIZE: i64 = 4096;
/// Digest size for incremental installation.
pub const K_DIGEST_SIZE: i64 = 32;

/// Calculates the number of blocks needed for a verity tree.
pub fn verity_tree_blocks_for_file(file_size: i64) -> i64 {
    if file_size == 0 {
        return 0;
    }

    let hash_per_block = K_BLOCK_SIZE / K_DIGEST_SIZE;
    let mut total_tree_block_count = 0;

    let mut hash_block_count = 1 + (file_size - 1) / K_BLOCK_SIZE;
    while hash_block_count > 1 {
        hash_block_count = (hash_block_count + hash_per_block - 1) / hash_per_block;
        total_tree_block_count += hash_block_count;
    }
    total_tree_block_count
}

/// Calculates the size of a verity tree in bytes.
pub fn verity_tree_size_for_file(file_size: i64) -> i64 {
    verity_tree_blocks_for_file(file_size) * K_BLOCK_SIZE
}

/// Converts a file offset to a block index.
pub fn offset_to_block_index(offset: i64) -> i32 {
    ((offset & !(K_BLOCK_SIZE - 1)) >> 12) as i32
}

/// Headers for an incremental ID signature.
pub struct IdSigHeaders {
    /// The signature data.
    pub signature: Vec<u8>,
    /// The size of the verity tree.
    pub tree_size: i32,
}

/// Reads the ID signature headers from a file.
pub fn read_id_sig_headers(mut file: &File) -> std::io::Result<IdSigHeaders> {
    let mut signature = Vec::new();
    let mut buf = [0u8; 4];

    file.read_exact(&mut buf)?;
    signature.extend_from_slice(&buf); // version

    // hashingInfo
    file.read_exact(&mut buf)?;
    let hashing_info_size = u32::from_le_bytes(buf) as usize;
    signature.extend_from_slice(&buf);
    let mut hashing_info = vec![0u8; hashing_info_size];
    file.read_exact(&mut hashing_info)?;
    signature.extend_from_slice(&hashing_info);

    // signingInfo
    file.read_exact(&mut buf)?;
    let signing_info_size = u32::from_le_bytes(buf) as usize;
    signature.extend_from_slice(&buf);
    let mut signing_info = vec![0u8; signing_info_size];
    file.read_exact(&mut signing_info)?;
    signature.extend_from_slice(&signing_info);

    file.read_exact(&mut buf)?;
    let tree_size = i32::from_le_bytes(buf);

    Ok(IdSigHeaders {
        signature,
        tree_size,
    })
}

/// Returns the priority blocks for a file.
pub fn priority_blocks_for_file(path: &Path) -> anyhow::Result<Vec<i32>> {
    let file = File::open(path)?;
    let file_size = file.metadata()?.len() as i64;

    // Minimal implementation for now - just start and end blocks
    // In a full implementation, we'd parse the ZIP structure as in original/client/incremental_utils.cpp
    let mut blocks = Vec::new();
    blocks.push(0);
    blocks.push(offset_to_block_index(file_size - 1));

    Ok(blocks)
}
