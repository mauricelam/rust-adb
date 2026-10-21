//! A library for talking to an ADB server.

pub mod server_address;

pub use server_address::{
    resolve_server_address, AddressError, ServerAddress, ServerAddressEnv, ServerAddressOptions,
    DEFAULT_SERVER_PORT,
};

#[cfg(test)]
mod tests {
    #[test]
    fn smoke_test() {
        assert_eq!(2 + 2, 4);
    }
}
