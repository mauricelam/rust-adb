/*
 * Copyright (C) 2023 The Android Open Source Project
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *      http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

//! Patch generation logic.
//! Ported from `original/fastdeploy/deploypatchgenerator/deploy_patch_generator.cpp`.

use crate::patch_utils::PatchUtils;
use crate::proto::{ApkEntry, ApkMetaData};
use anyhow::{anyhow, Result};
use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

pub struct SimpleEntry<'a> {
    pub local_entry: &'a ApkEntry,
    pub device_entry: &'a ApkEntry,
}

pub struct DeployPatchGenerator {
    pub is_verbose: bool,
}

#[derive(Default)]
struct PatchEntry {
    delta_from_device_data_start: i64,
    device_data_offset: i64,
    device_data_length: i64,
}

impl DeployPatchGenerator {
    pub fn new(is_verbose: bool) -> Self {
        Self { is_verbose }
    }

    pub fn create_patch<P: AsRef<Path>, W: Write>(
        &self,
        local_apk_path: P,
        device_apk_metadata: ApkMetaData,
        output: W,
    ) -> Result<()> {
        let local_apk_metadata = PatchUtils::get_host_apk_metadata(local_apk_path)?;
        self.create_patch_from_metadata(local_apk_metadata, device_apk_metadata, output)
    }

    pub fn create_patch_from_metadata<W: Write>(
        &self,
        local_apk_metadata: ApkMetaData,
        device_apk_metadata: ApkMetaData,
        output: W,
    ) -> Result<()> {
        let mut identical_entries = Vec::new();
        let total_size = self.build_identical_entries(
            &mut identical_entries,
            &local_apk_metadata,
            &device_apk_metadata,
        );

        self.report_savings(&identical_entries, total_size);

        self.generate_patch(
            &identical_entries,
            &local_apk_metadata.absolute_path,
            &device_apk_metadata.absolute_path,
            output,
        )
    }

    fn report_savings(&self, identical_entries: &[SimpleEntry], total_size: u64) {
        let mut total_equal_bytes = 0;
        let mut total_equal_files = 0;
        for entry in identical_entries {
            total_equal_bytes += entry.local_entry.data_size;
            total_equal_files += 1;
        }
        let saving_percent = if total_size > 0 {
            (total_equal_bytes as f64 * 100.0) / total_size as f64
        } else {
            0.0
        };
        eprintln!("Detected {} equal APK entries", total_equal_files);
        eprintln!(
            "{} bytes are equal out of {} ({:.2}%)",
            total_equal_bytes, total_size, saving_percent
        );
    }

    fn write_patch_entry<R: Read + Seek, W: Write>(
        &self,
        patch_entry: &PatchEntry,
        mut input: R,
        mut output: W,
        real_size_out: &mut u64,
    ) -> Result<()> {
        if patch_entry.delta_from_device_data_start == 0
            && patch_entry.device_data_offset == 0
            && patch_entry.device_data_length == 0
        {
            return Ok(());
        }

        PatchUtils::write_long(patch_entry.delta_from_device_data_start, &mut output)?;
        if patch_entry.delta_from_device_data_start > 0 {
            PatchUtils::pipe(
                &mut input,
                &mut output,
                patch_entry.delta_from_device_data_start as u64,
            )?;
        }
        let host_data_length = patch_entry.device_data_length;
        input.seek(SeekFrom::Current(host_data_length))?;

        PatchUtils::write_long(patch_entry.device_data_offset, &mut output)?;
        PatchUtils::write_long(patch_entry.device_data_length, &mut output)?;

        *real_size_out += (patch_entry.delta_from_device_data_start + host_data_length) as u64;
        Ok(())
    }

    fn generate_patch<W: Write>(
        &self,
        entries_to_use_on_device: &[SimpleEntry],
        local_apk_path: &str,
        device_apk_path: &str,
        mut output: W,
    ) -> Result<()> {
        let mut input = File::open(local_apk_path)?;
        let new_apk_size = input.seek(SeekFrom::End(0))?;
        input.seek(SeekFrom::Start(0))?;

        // Header
        PatchUtils::write_signature(&mut output)?;
        PatchUtils::write_long(new_apk_size as i64, &mut output)?;
        PatchUtils::write_string(device_apk_path, &mut output)?;

        let mut current_size_out = 0u64;
        let mut real_size_out = 0u64;

        let mut patch_entry = PatchEntry::default();

        for entry in entries_to_use_on_device {
            let host_data_offset = entry.local_entry.data_offset as u64;
            let host_data_length = entry.local_entry.data_size as i64;
            let device_data_offset = entry.device_entry.data_offset;
            let device_data_length = host_data_length;

            let delta_from_device_data_start = (host_data_offset - current_size_out) as i64;

            let is_contiguous_on_device = patch_entry.device_data_length > 0 &&
                device_data_offset == patch_entry.device_data_offset + patch_entry.device_data_length;

            if delta_from_device_data_start > 0 || !is_contiguous_on_device {
                self.write_patch_entry(&patch_entry, &mut input, &mut output, &mut real_size_out)?;
                patch_entry.delta_from_device_data_start = delta_from_device_data_start;
                patch_entry.device_data_offset = device_data_offset;
                patch_entry.device_data_length = device_data_length;
            } else {
                patch_entry.device_data_length += device_data_length;
            }

            current_size_out += (delta_from_device_data_start + host_data_length) as u64;
        }

        self.write_patch_entry(&patch_entry, &mut input, &mut output, &mut real_size_out)?;

        if real_size_out != current_size_out {
            return Err(anyhow!(
                "Size mismatch: current {} vs real {}",
                current_size_out,
                real_size_out
            ));
        }

        if new_apk_size > current_size_out {
            PatchUtils::write_long((new_apk_size - current_size_out) as i64, &mut output)?;
            PatchUtils::pipe(&mut input, &mut output, new_apk_size - current_size_out)?;
            PatchUtils::write_long(0, &mut output)?;
            PatchUtils::write_long(0, &mut output)?;
        }

        Ok(())
    }

    pub fn build_identical_entries<'a>(
        &self,
        out_identical_entries: &mut Vec<SimpleEntry<'a>>,
        local_apk_metadata: &'a ApkMetaData,
        device_apk_metadata: &'a ApkMetaData,
    ) -> u64 {
        let mut device_entries: HashMap<&[u8], Vec<&ApkEntry>> = HashMap::new();
        for entry in &device_apk_metadata.entries {
            device_entries.entry(&entry.md5).or_default().push(entry);
        }

        let mut total_size = 0u64;
        for local_entry in &local_apk_metadata.entries {
            total_size += local_entry.data_size as u64;

            if let Some(entries) = device_entries.get(&local_entry.md5[..]) {
                for device_entry in entries {
                    if device_entry.md5 == local_entry.md5 {
                        out_identical_entries.push(SimpleEntry {
                            local_entry,
                            device_entry,
                        });
                        break;
                    }
                }
            }
        }

        out_identical_entries.sort_by_key(|e| e.local_entry.data_offset);
        total_size
    }
}
