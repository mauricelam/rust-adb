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

//! Protobuf messages for fastdeploy.
//! Ported from `original/fastdeploy/proto/ApkEntry.proto`.

/// Dump of Central Directory and Signature Block.
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct ApkDump {
    /// Package name.
    #[prost(string, tag = "1")]
    pub name: ::prost::alloc::string::String,
    /// Central Directory bytes.
    #[prost(bytes = "vec", tag = "2")]
    pub cd: ::prost::alloc::vec::Vec<u8>,
    /// Signature block bytes.
    #[prost(bytes = "vec", tag = "3")]
    pub signature: ::prost::alloc::vec::Vec<u8>,
    /// Absolute path to the APK on device.
    #[prost(string, tag = "4")]
    pub absolute_path: ::prost::alloc::string::String,
}

/// Information about a single entry in the APK.
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct ApkEntry {
    /// MD5 hash of the entry.
    #[prost(bytes = "vec", tag = "1")]
    pub md5: ::prost::alloc::vec::Vec<u8>,
    /// Offset to the local file header.
    #[prost(int64, tag = "2")]
    pub data_offset: i64,
    /// Size of the local file entry (header + data + optional data descriptor).
    #[prost(int64, tag = "3")]
    pub data_size: i64,
}

/// Metadata about an APK.
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct ApkMetaData {
    /// Absolute path to the APK.
    #[prost(string, tag = "1")]
    pub absolute_path: ::prost::alloc::string::String,
    /// Entries in the APK.
    #[prost(message, repeated, tag = "2")]
    pub entries: ::prost::alloc::vec::Vec<ApkEntry>,
}
