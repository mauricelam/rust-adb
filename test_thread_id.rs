use std::thread;
fn main() {
    println!("{:?}", thread::current().id());
}
