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

//! Property monitor implementation.
//! Ported from original/daemon/property_monitor.cpp.

use crate::restart_service::{get_property, set_property};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub type PropertyMonitorCallback = Box<dyn Fn(String) -> bool + Send>;

struct PropertyData {
    callback: PropertyMonitorCallback,
    last_value: String,
}

pub struct PropertyMonitor {
    properties: HashMap<String, PropertyData>,
}

impl PropertyMonitor {
    pub fn new() -> Self {
        Self {
            properties: HashMap::new(),
        }
    }

    pub fn add<F>(&mut self, property: String, callback: F)
    where
        F: Fn(String) -> bool + Send + 'static,
    {
        let initial_value = get_property(&property, "");
        let mut data = PropertyData {
            callback: Box::new(callback),
            last_value: initial_value.clone(),
        };

        // Initial callback
        (data.callback)(initial_value);

        self.properties.insert(property, data);
    }

    pub fn run(&mut self) {
        loop {
            // In a real android system, this would use __system_property_wait.
            // For our port, we poll the mock property system.
            std::thread::sleep(Duration::from_millis(10));

            for (name, data) in self.properties.iter_mut() {
                let current_value = get_property(name, "");
                if current_value != data.last_value {
                    data.last_value = current_value.clone();
                    if !(data.callback)(current_value) {
                        return;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use std::thread;

    struct PropertyChanges {
        changes: Arc<Mutex<HashMap<String, Vec<String>>>>,
    }

    fn mangle_property_name(name: &str) -> String {
        format!("{}.{:?}.{}", name, thread::current().id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos())
    }

    fn register_callback(pm: &mut PropertyMonitor, output: &PropertyChanges, property_name: String) {
        let changes = output.changes.clone();
        let name_clone = property_name.clone();
        pm.add(property_name, move |value| {
            let mut lock = changes.lock().unwrap();
            lock.entry(name_clone.clone()).or_insert_with(Vec::new).push(value);
            true
        });
    }

    #[test]
    fn test_initial() {
        let mut pm = PropertyMonitor::new();
        let output = PropertyChanges {
            changes: Arc::new(Mutex::new(HashMap::new())),
        };

        let foo = mangle_property_name("debug.property_monitor_test.initial");
        let never_set = mangle_property_name("debug.property_monitor_test.never_set");

        register_callback(&mut pm, &output, foo.clone());
        set_property(&foo, "foo");

        register_callback(&mut pm, &output, never_set.clone());

        // Run in a separate thread and then stop
        let exit_prop = mangle_property_name("debug.property_monitor_test.exit");
        set_property(&exit_prop, "0");
        pm.add(exit_prop.clone(), |value| value != "1");

        let handle = thread::spawn(move || {
            pm.run();
        });

        thread::sleep(Duration::from_millis(50));
        set_property(&exit_prop, "1");
        handle.join().unwrap();

        let lock = output.changes.lock().unwrap();
        assert_eq!(lock.len(), 2);
        assert_eq!(lock[&foo].len(), 2);
        assert_eq!(lock[&foo][0], "");
        assert_eq!(lock[&foo][1], "foo");
        assert_eq!(lock[&never_set].len(), 1);
        assert_eq!(lock[&never_set][0], "");
    }

    #[test]
    fn test_change() {
        let mut pm = PropertyMonitor::new();
        let output = PropertyChanges {
            changes: Arc::new(Mutex::new(HashMap::new())),
        };

        let foo = mangle_property_name("debug.property_monitor_test.foo");

        register_callback(&mut pm, &output, foo.clone());
        set_property(&foo, "foo");

        let exit_prop = mangle_property_name("debug.property_monitor_test.exit");
        set_property(&exit_prop, "0");
        pm.add(exit_prop.clone(), |value| value != "1");

        let handle = thread::spawn(move || {
            pm.run();
        });

        thread::sleep(Duration::from_millis(50));

        {
            let lock = output.changes.lock().unwrap();
            assert_eq!(lock.len(), 1);
            assert_eq!(lock[&foo].len(), 2);
            assert_eq!(lock[&foo][0], "");
            assert_eq!(lock[&foo][1], "foo");
        }

        set_property(&foo, "bar");
        thread::sleep(Duration::from_millis(50));

        {
            let lock = output.changes.lock().unwrap();
            assert_eq!(lock[&foo].len(), 3);
            assert_eq!(lock[&foo][0], "");
            assert_eq!(lock[&foo][1], "foo");
            assert_eq!(lock[&foo][2], "bar");
        }

        set_property(&exit_prop, "1");
        handle.join().unwrap();
    }
}
