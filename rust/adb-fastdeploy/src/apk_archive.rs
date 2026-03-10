//! APK archive manipulation.
//! Ported from `original/fastdeploy/deploypatchgenerator/apk_archive.cpp`.

use crate::proto::ApkDump;
use anyhow::{anyhow, Result};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

const K_COMPRESS_STORED: u16 = 0;
const K_GPB_DD_FLAG_MASK: u16 = 0x0008;

const EOCD_SIGNATURE: u32 = 0x06054b50;
const EOCD_MIN_SIZE: u64 = 22;
const EOCD_MAX_SIZE: u64 = 65535 + EOCD_MIN_SIZE;

const CD_FILE_HEADER_MAGIC: u32 = 0x02014b50;
const LOCAL_FILE_HEADER_MAGIC: u32 = 0x04034b50;
const OPTIONAL_DATA_DESCRIPTOR_MAGIC: u32 = 0x08074b50;

/// A convenience struct to store the result of search operation when
/// locating the EoCDr, CDr, and Signature Block.
#[derive(Debug, Clone, Copy, Default)]
pub struct Location {
    /// Offset of the record.
    pub offset: u64,
    /// Size of the record.
    pub size: u64,
    /// Whether the record is valid.
    pub valid: bool,
}

/// Manipulates an APK archive.
pub struct ApkArchive {
    path: String,
    file: File,
    size: u64,
}

impl ApkArchive {
    /// Opens an APK archive.
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let path_str = path.as_ref().to_string_lossy().into_owned();
        let file = File::open(&path)?;
        let metadata = file.metadata()?;
        let size = metadata.len();
        Ok(Self {
            path: path_str,
            file,
            size,
        })
    }

    /// Extracts metadata from the APK.
    pub fn extract_metadata(&mut self) -> Result<ApkDump> {
        let cd_loc = self.get_cd_location()?;
        if !cd_loc.valid {
            return Err(anyhow!("Unable to find Central Directory Record"));
        }

        let mut dump = ApkDump::default();
        dump.absolute_path = self.path.clone();
        dump.cd = self.read_metadata(cd_loc)?;

        let sig_loc = self.get_signature_location(cd_loc.offset)?;
        if sig_loc.valid {
            dump.signature = self.read_metadata(sig_loc)?;
        }

        Ok(dump)
    }

    fn read_metadata(&mut self, loc: Location) -> Result<Vec<u8>> {
        let mut buf = vec![0u8; loc.size as usize];
        self.file.seek(SeekFrom::Start(loc.offset))?;
        self.file.read_exact(&mut buf)?;
        Ok(buf)
    }

    fn find_end_of_cd_record(&mut self) -> Result<i64> {
        let size_to_read = std::cmp::min(self.size, EOCD_MAX_SIZE);
        let read_offset = self.size - size_to_read;

        let mut buf = vec![0u8; size_to_read as usize];
        self.file.seek(SeekFrom::Start(read_offset))?;
        self.file.read_exact(&mut buf)?;

        for i in (0..=(size_to_read as usize - 4)).rev() {
            let sig = u32::from_le_bytes(buf[i..i + 4].try_into().unwrap());
            if sig == EOCD_SIGNATURE {
                return Ok((read_offset + i as u64) as i64);
            }
        }
        Ok(-1)
    }

    /// Retrieve the location of the Central Directory Record.
    pub fn get_cd_location(&mut self) -> Result<Location> {
        let eocd_offset = self.find_end_of_cd_record()?;
        if eocd_offset < 0 {
            return Ok(Location::default());
        }

        let mut buf = [0u8; 22];
        self.file.seek(SeekFrom::Start(eocd_offset as u64))?;
        self.file.read_exact(&mut buf)?;

        let cr_size = u32::from_le_bytes(buf[12..16].try_into().unwrap());
        let offset_to_cd_header = u32::from_le_bytes(buf[16..20].try_into().unwrap());

        Ok(Location {
            offset: offset_to_cd_header as u64,
            size: cr_size as u64,
            valid: true,
        })
    }

    /// Retrieve the location of the signature block starting from Central Directory Record.
    pub fn get_signature_location(&mut self, cd_record_offset: u64) -> Result<Location> {
        if cd_record_offset < 24 {
            return Ok(Location::default());
        }

        let signature_offset = cd_record_offset - 24;
        let mut buf = [0u8; 24];
        self.file.seek(SeekFrom::Start(signature_offset))?;
        self.file.read_exact(&mut buf)?;

        let signature_size = u64::from_le_bytes(buf[0..8].try_into().unwrap());
        let signature = &buf[8..24];

        if signature != b"APK Sig Block 42" {
            return Ok(Location::default());
        }

        Ok(Location {
            size: signature_size,
            offset: cd_record_offset - signature_size - 8,
            valid: true,
        })
    }

    /// Parses a Central Directory Record.
    pub fn parse_central_directory_record(
        input: &[u8],
    ) -> Option<(usize, Vec<u8>, i64, i64)> {
        if input.len() < 46 {
            return None;
        }

        let sig = u32::from_le_bytes(input[0..4].try_into().unwrap());
        if sig != CD_FILE_HEADER_MAGIC {
            return None;
        }

        let _gpb_flags = u16::from_le_bytes(input[8..10].try_into().unwrap());
        let compression_method = u16::from_le_bytes(input[10..12].try_into().unwrap());
        let compressed_size = u32::from_le_bytes(input[20..24].try_into().unwrap());
        let uncompressed_size = u32::from_le_bytes(input[24..28].try_into().unwrap());
        let file_name_length = u16::from_le_bytes(input[28..30].try_into().unwrap());
        let extra_field_length = u16::from_le_bytes(input[30..32].try_into().unwrap());
        let comment_length = u16::from_le_bytes(input[32..34].try_into().unwrap());
        let local_file_header_offset = u32::from_le_bytes(input[42..46].try_into().unwrap());

        let total_size = 46 + file_name_length as usize + extra_field_length as usize + comment_length as usize;
        if input.len() < total_size {
            return None;
        }

        let record_bytes = &input[0..total_size];
        let md5_hash = md5::compute(record_bytes).to_vec();

        let data_size = if compression_method == K_COMPRESS_STORED {
            uncompressed_size as i64
        } else {
            compressed_size as i64
        };

        Some((
            total_size,
            md5_hash,
            local_file_header_offset as i64,
            data_size,
        ))
    }

    /// Calculates the size of a Local File Entry.
    pub fn calculate_local_file_entry_size(
        &mut self,
        local_file_header_offset: i64,
        data_size: i64,
    ) -> Result<i64> {
        let mut buf = [0u8; 30];
        self.file.seek(SeekFrom::Start(local_file_header_offset as u64))?;
        self.file.read_exact(&mut buf)?;

        let sig = u32::from_le_bytes(buf[0..4].try_into().unwrap());
        if sig != LOCAL_FILE_HEADER_MAGIC {
            return Err(anyhow!("Invalid Local File Header signature"));
        }

        let gpb_flags = u16::from_le_bytes(buf[6..8].try_into().unwrap());
        let compression_method = u16::from_le_bytes(buf[8..10].try_into().unwrap());
        let compressed_size = u32::from_le_bytes(buf[18..22].try_into().unwrap());
        let uncompressed_size = u32::from_le_bytes(buf[22..26].try_into().unwrap());
        let file_name_length = u16::from_le_bytes(buf[26..28].try_into().unwrap());
        let extra_field_length = u16::from_le_bytes(buf[28..30].try_into().unwrap());

        let mut dd_size = 0;
        let local_data_size;

        if (gpb_flags & K_GPB_DD_FLAG_MASK) != 0 {
            let dd_offset = local_file_header_offset as u64 + 30 + file_name_length as u64 + extra_field_length as u64 + data_size as u64;
            self.file.seek(SeekFrom::Start(dd_offset))?;
            let mut dd_buf = [0u8; 16];
            self.file.read_exact(&mut dd_buf)?;

            let mut current_dd_offset = 0;
            if u32::from_le_bytes(dd_buf[0..4].try_into().unwrap()) == OPTIONAL_DATA_DESCRIPTOR_MAGIC {
                current_dd_offset = 4;
                dd_size += 4;
            }

            let compressed_size_dd = u32::from_le_bytes(dd_buf[current_dd_offset + 4..current_dd_offset + 8].try_into().unwrap());
            let uncompressed_size_dd = u32::from_le_bytes(dd_buf[current_dd_offset + 8..current_dd_offset + 12].try_into().unwrap());

            local_data_size = if compression_method == K_COMPRESS_STORED {
                uncompressed_size_dd as i64
            } else {
                compressed_size_dd as i64
            };
            dd_size += 12;
        } else {
            local_data_size = if compression_method == K_COMPRESS_STORED {
                uncompressed_size as i64
            } else {
                compressed_size as i64
            };
        }

        if local_data_size != data_size {
            return Err(anyhow!(
                "Data sizes mismatch: CDr: {} vs LHR/DD: {}",
                data_size,
                local_data_size
            ));
        }

        Ok(30 + file_name_length as i64 + extra_field_length as i64 + data_size + dd_size)
    }
}
