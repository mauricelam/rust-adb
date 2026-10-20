#![no_main]

use adb_proto::{parse_protocol_string, parse_reply};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = parse_reply(data);
    let _ = parse_protocol_string(data);
});
