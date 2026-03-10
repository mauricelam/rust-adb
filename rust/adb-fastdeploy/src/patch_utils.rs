//! Helper functions for fastdeploy.
//! Ported from `original/fastdeploy/deploypatchgenerator/patch_utils.cpp`.

use crate::apk_archive::ApkArchive;
use crate::proto::{ApkDump, ApkEntry, ApkMetaData};
use anyhow::{anyhow, Result};
use std::io::{Read, Write};
use std::path::Path;

/// Signature for the patch format.
pub const K_SIGNATURE: &[u8] = b"FASTDEPLOY";

/// Helper class that mirrors the PatchUtils from deploy agent.
pub struct PatchUtils;

impl PatchUtils {
    /// Builds the APKMetaData from the APK dump.
    pub fn get_device_apk_metadata(apk_dump: &ApkDump) -> ApkMetaData {
        let mut apk_metadata = ApkMetaData::default();
        apk_metadata.absolute_path = apk_dump.absolute_path.clone();

        let mut cur = &apk_dump.cd[..];
        while let Some((consumed, md5, offset, size)) =
            ApkArchive::parse_central_directory_record(cur)
        {
            cur = &cur[consumed..];

            let mut apk_entry = ApkEntry::default();
            apk_entry.md5 = md5;
            apk_entry.data_offset = offset;
            apk_entry.data_size = size;
            apk_metadata.entries.push(apk_entry);
        }
        apk_metadata
    }

    /// Builds the APKMetaData for a local APK file.
    pub fn get_host_apk_metadata<P: AsRef<Path>>(path: P) -> Result<ApkMetaData> {
        let mut archive = ApkArchive::open(&path)?;
        let dump = archive.extract_metadata()?;
        if dump.cd.is_empty() {
            return Err(anyhow!("Could not extract Central Directory from {:?}", path.as_ref()));
        }

        let mut apk_metadata = Self::get_device_apk_metadata(&dump);

        for entry in &mut apk_metadata.entries {
            let data_size = archive.calculate_local_file_entry_size(entry.data_offset, entry.data_size)?;
            if data_size == 0 {
                return Err(anyhow!("Empty local file entry in {:?}", path.as_ref()));
            }
            entry.data_size = data_size;
        }

        Ok(apk_metadata)
    }

    /// Writes the signature to the output.
    pub fn write_signature<W: Write>(mut output: W) -> Result<()> {
        output.write_all(K_SIGNATURE)?;
        Ok(())
    }

    /// Writes a long value to the output.
    pub fn write_long<W: Write>(value: i64, mut output: W) -> Result<()> {
        output.write_all(&value.to_le_bytes())?;
        Ok(())
    }

    /// Writes a string value to the output.
    pub fn write_string<W: Write>(value: &str, mut output: W) -> Result<()> {
        Self::write_long(value.len() as i64, &mut output)?;
        output.write_all(value.as_bytes())?;
        Ok(())
    }

    /// Copies data from input to output.
    pub fn pipe<R: Read, W: Write>(mut input: R, mut output: W, amount: u64) -> Result<()> {
        const BUFFER_SIZE: usize = 128 * 1024;
        let mut buffer = [0u8; BUFFER_SIZE];
        let mut transfer_amount = 0;
        while transfer_amount < amount {
            let chunk_amount = std::cmp::min(amount - transfer_amount, BUFFER_SIZE as u64);
            input.read_exact(&mut buffer[..chunk_amount as usize])?;
            output.write_all(&buffer[..chunk_amount as usize])?;
            transfer_amount += chunk_amount;
        }
        Ok(())
    }
}
