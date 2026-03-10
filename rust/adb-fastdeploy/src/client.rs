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

//! Client-side orchestration for fastdeploy.
//! Ported from `original/client/fastdeploy.cpp`.

use crate::proto::ApkMetaData;
use anyhow::{anyhow, Result};
use std::path::Path;

pub const K_REQUIRED_AGENT_VERSION: i64 = 0x00000003;
pub const K_PACKAGE_MISSING: i32 = 3;
pub const K_INVALID_AGENT_VERSION: i32 = 4;

pub const K_DEVICE_AGENT_FILE: &str = "/data/local/tmp/deployagent.jar";
pub const K_DEVICE_AGENT_SCRIPT: &str = "/data/local/tmp/deployagent";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentUpdateStrategy {
    Always,
    NewerTimeStamp,
    DifferentVersion,
}

pub struct FastDeploy {
    pub agent_update_strategy: AgentUpdateStrategy,
}

impl FastDeploy {
    pub fn new() -> Self {
        Self {
            agent_update_strategy: AgentUpdateStrategy::DifferentVersion,
        }
    }

    pub fn extract_metadata<P: AsRef<Path>>(
        &self,
        _apk_path: P,
    ) -> Result<Option<ApkMetaData>> {
        // In a real implementation, we would call adb shell to get the package name from APK,
        // check the agent version on device, push the agent if necessary, and then run the agent's dump command.
        // For this port, we provide the structure.
        Err(anyhow!("Not fully implemented: requires ADB client integration"))
    }
}

/// Parses the agent version from the shell output.
pub fn parse_agent_version(version_str: &str) -> i64 {
    i64::from_str_radix(version_str.trim(), 16).unwrap_or(-1)
}

/// Gets the package name from an APK file.
/// This would typically use `zip` crate to read AndroidManifest.xml and parse it.
pub fn get_package_name_from_apk<P: AsRef<Path>>(_apk_path: P) -> Result<String> {
    // Placeholder for actual APK manifest parsing logic.
    Err(anyhow!("Not implemented: APK manifest parsing"))
}
