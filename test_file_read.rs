use std::io::Read;
fn main() {
    let f = std::fs::File::open("Cargo.toml").unwrap();
    let mut rf = &f;
    let mut buf = [0u8; 10];
    rf.read(&mut buf).unwrap();
}
