/*
 * Copyright (C) 2020 The Android Open Source Project
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

//! Property monitoring service.
//! Ported from `original/daemon/property_monitor.cpp`.

use crate::restart_service::{get_property, set_property};

use std::sync::{Arc, Mutex};

/// Callback type for property changes.
pub type PropertyMonitorCallback = Box<dyn Fn(String) -> bool + Send>;

struct PropertyData {
    property: String,
    last_value: String,
    callback: PropertyMonitorCallback,
}

/// Monitors system properties and executes callbacks on change.
pub struct PropertyMonitor {
    properties: Vec<PropertyData>,
}

impl PropertyMonitor {
    /// Creates a new `PropertyMonitor`.
    pub fn new() -> Self {
        Self {
            properties: Vec::new(),
        }
    }

    /// Adds a property to be monitored.
    pub fn add<F>(&mut self, property: String, callback: F)
    where
        F: Fn(String) -> bool + Send + 'static,
    {
        let last_value = get_property(&property, "");
        self.properties.push(PropertyData {
            property,
            last_value,
            callback: Box::new(callback),
        });
    }

    /// Runs the monitor, checking for changes in a loop.
    pub fn run(&mut self) {
        loop {
            let mut to_remove = Vec::new();
            for (i, data) in self.properties.iter_mut().enumerate() {
                let current_value = get_property(&data.property, "");
                if current_value != data.last_value {
                    data.last_value = current_value.clone();
                    if !(data.callback)(current_value) {
                        to_remove.push(i);
                    }
                }
            }

            for &i in to_remove.iter().rev() {
                self.properties.remove(i);
            }

            if self.properties.is_empty() {
                break;
            }

            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    }
}

impl Default for PropertyMonitor {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_property_monitor() {
        set_property("test.prop", "initial");
        let mut monitor = PropertyMonitor::new();
        let changed = Arc::new(Mutex::new(false));
        let changed_clone = changed.clone();

        monitor.add("test.prop".to_string(), move |val| {
            if val == "changed" {
                *changed_clone.lock().unwrap() = true;
                false // stop monitoring
            } else {
                true
            }
        });

        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(200));
            set_property("test.prop", "changed");
        });

        monitor.run();
        assert!(*changed.lock().unwrap());
    }
}
