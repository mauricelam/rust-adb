/*
 * Copyright (C) 2007 The Android Open Source Project
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

use zerocopy::{FromBytes, Immutable, IntoBytes};

pub const fn mkid(a: u8, b: u8, c: u8, d: u8) -> u32 {
    (a as u32) | ((b as u32) << 8) | ((c as u32) << 16) | ((d as u32) << 24)
}

pub const ID_LSTAT_V1: u32 = mkid(b'S', b'T', b'A', b'T');
pub const ID_STAT_V2: u32 = mkid(b'S', b'T', b'A', b'2');
pub const ID_LSTAT_V2: u32 = mkid(b'L', b'S', b'T', b'2');

pub const ID_LIST_V1: u32 = mkid(b'L', b'I', b'S', b'T');
pub const ID_LIST_V2: u32 = mkid(b'L', b'I', b'S', b'2');
pub const ID_DENT_V1: u32 = mkid(b'D', b'E', b'N', b'T');
pub const ID_DENT_V2: u32 = mkid(b'D', b'N', b'T', b'2');

pub const ID_SEND_V1: u32 = mkid(b'S', b'E', b'N', b'D');
pub const ID_SEND_V2: u32 = mkid(b'S', b'N', b'D', b'2');
pub const ID_RECV_V1: u32 = mkid(b'R', b'E', b'C', b'V');
pub const ID_RECV_V2: u32 = mkid(b'R', b'C', b'V', b'2');
pub const ID_DONE: u32 = mkid(b'D', b'O', b'N', b'E');
pub const ID_DATA: u32 = mkid(b'D', b'A', b'T', b'A');
pub const ID_OKAY: u32 = mkid(b'O', b'K', b'A', b'Y');
pub const ID_FAIL: u32 = mkid(b'F', b'A', b'I', b'L');
pub const ID_QUIT: u32 = mkid(b'Q', b'U', b'I', b'T');

#[repr(C, packed)]
#[derive(Clone, Copy, Debug, FromBytes, IntoBytes, Immutable)]
pub struct SyncRequest {
    pub id: u32,           // ID_STAT, et cetera.
    pub path_length: u32,  // <= 1024
}

#[repr(C, packed)]
#[derive(Clone, Copy, Debug, FromBytes, IntoBytes, Immutable)]
pub struct SyncStatV1 {
    pub id: u32,
    pub mode: u32,
    pub size: u32,
    pub mtime: u32,
}

#[repr(C, packed)]
#[derive(Clone, Copy, Debug, FromBytes, IntoBytes, Immutable)]
pub struct SyncStatV2 {
    pub id: u32,
    pub error: u32,
    pub dev: u64,
    pub ino: u64,
    pub mode: u32,
    pub nlink: u32,
    pub uid: u32,
    pub gid: u32,
    pub size: u64,
    pub atime: i64,
    pub mtime: i64,
    pub ctime: i64,
}

#[repr(C, packed)]
#[derive(Clone, Copy, Debug, FromBytes, IntoBytes, Immutable)]
pub struct SyncDentV1 {
    pub id: u32,
    pub mode: u32,
    pub size: u32,
    pub mtime: u32,
    pub namelen: u32,
}

#[repr(C, packed)]
#[derive(Clone, Copy, Debug, FromBytes, IntoBytes, Immutable)]
pub struct SyncDentV2 {
    pub id: u32,
    pub error: u32,
    pub dev: u64,
    pub ino: u64,
    pub mode: u32,
    pub nlink: u32,
    pub uid: u32,
    pub gid: u32,
    pub size: u64,
    pub atime: i64,
    pub mtime: i64,
    pub ctime: i64,
    pub namelen: u32,
}

pub const SYNC_FLAG_NONE: u32 = 0;
pub const SYNC_FLAG_BROTLI: u32 = 1;
pub const SYNC_FLAG_LZ4: u32 = 2;
pub const SYNC_FLAG_ZSTD: u32 = 4;
pub const SYNC_FLAG_DRY_RUN: u32 = 0x8000_0000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompressionType {
    None,
    Any,
    Brotli,
    LZ4,
    Zstd,
}

#[repr(C, packed)]
#[derive(Clone, Copy, Debug, FromBytes, IntoBytes, Immutable)]
pub struct SyncSendV2 {
    pub id: u32,
    pub mode: u32,
    pub flags: u32,
}

#[repr(C, packed)]
#[derive(Clone, Copy, Debug, FromBytes, IntoBytes, Immutable)]
pub struct SyncRecvV2 {
    pub id: u32,
    pub flags: u32,
}

#[repr(C, packed)]
#[derive(Clone, Copy, Debug, FromBytes, IntoBytes, Immutable)]
pub struct SyncData {
    pub id: u32,
    pub size: u32,
}

#[repr(C, packed)]
#[derive(Clone, Copy, Debug, FromBytes, IntoBytes, Immutable)]
pub struct SyncStatus {
    pub id: u32,
    pub msglen: u32,
}

pub const SYNC_DATA_MAX: usize = 64 * 1024;
