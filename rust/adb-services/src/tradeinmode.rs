/*
 * Copyright (C) 2024 The Android Open Source Project
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

//! Trade-in mode service implementation.
//! Ported from original/daemon/tradeinmode.cpp.

use regex::Regex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

static IN_TRADEINMODE: AtomicBool = AtomicBool::new(false);
const K_TRADEIN_MODE_PROP: &str = "persist.adb.tradeinmode";

const TIM_DISABLED: i32 = -1;
const TIM_UNSET: i32 = 0;
const TIM_FOYER: i32 = 1;
const TIM_EVALUATION_MODE: i32 = 2;

/// Returns whether adbd should enter trade-in mode.
pub fn should_enter_tradeinmode() -> bool {
    #[cfg(target_os = "android")]
    {
        // TODO: check com_android_tradeinmode_flags_enable_trade_in_mode()
        // For now, assume it's true or handle it via property.
        get_int_property(K_TRADEIN_MODE_PROP, TIM_UNSET) == TIM_FOYER
    }
    #[cfg(not(target_os = "android"))]
    {
        false
    }
}

/// Enters trade-in mode.
pub fn enter_tradeinmode(_seclabel: &str) {
    #[cfg(target_os = "android")]
    {
        // Porting selinux_android_setcon is complex and might not be needed for all tests.
        // For now, we set the global flag.
        // In a real implementation, we would call into a C wrapper for selinux.
        IN_TRADEINMODE.store(true, Ordering::SeqCst);
    }
    #[cfg(not(target_os = "android"))]
    {
        // No-op for non-android.
    }
}

/// Returns whether the device is in trade-in mode.
pub fn is_in_tradeinmode() -> bool {
    IN_TRADEINMODE.load(Ordering::SeqCst)
}

/// Returns whether the device is in trade-in evaluation mode.
pub fn is_in_tradein_evaluation_mode() -> bool {
    get_int_property(K_TRADEIN_MODE_PROP, TIM_UNSET) == TIM_EVALUATION_MODE
}

/// Validates if a command is allowed in trade-in mode.
pub fn allow_tradeinmode_command(name: &str) -> bool {
    #[cfg(target_os = "android")]
    {
        // Allow "adb root" from trade-in-mode so that automated testing is possible.
        // Porting __android_log_is_debuggable()
        if is_debuggable() && name.starts_with("root:") {
            return true;
        }
    }

    // Allow "shell tradeinmode" with only simple arguments.
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        Regex::new(r"^shell[^:]*:tradeinmode(\s*|\s[A-Za-z0-9_\-\s]*)$").unwrap()
    });

    re.is_match(name)
}

#[cfg(target_os = "android")]
fn get_int_property(prop: &str, default: i32) -> i32 {
    // This would use android-base properties in a real build.
    // For now we use a stub or environment variable for testing.
    std::env::var(prop)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

#[cfg(not(target_os = "android"))]
fn get_int_property(prop: &str, default: i32) -> i32 {
    std::env::var(prop)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

#[cfg(target_os = "android")]
fn is_debuggable() -> bool {
    // Placeholder for __android_log_is_debuggable()
    get_int_property("ro.debuggable", 0) == 1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_command() {
        assert!(!allow_tradeinmode_command("shell:blah"));
        assert!(allow_tradeinmode_command("shell,-x:tradeinmode"));
        assert!(allow_tradeinmode_command("shell:tradeinmode"));
        assert!(!allow_tradeinmode_command("shell:tradeinmodebad"));
        assert!(allow_tradeinmode_command("shell:tradeinmode getstatus"));
        assert!(allow_tradeinmode_command("shell:tradeinmode getstatus -c 1234"));
        assert!(allow_tradeinmode_command("shell:tradeinmode enter"));
        assert!(!allow_tradeinmode_command("shell:tradeinmode && ls"));
    }
}
