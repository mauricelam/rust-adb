#![deny(missing_docs)]

//! FastDeploy implementation for ADB.

/// APK archive manipulation.
pub mod apk_archive;
/// Client-side orchestration for fastdeploy.
pub mod client;
/// Patch generation logic.
pub mod deploy_patch_generator;
/// Helper functions for fastdeploy.
pub mod patch_utils;
/// Protobuf messages for fastdeploy.
pub mod proto;

#[cfg(test)]
mod tests;
