#![no_main]

use adb_proto::Header;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = Header::decode(data);
});
