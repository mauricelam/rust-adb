//! A library for talking to an ADB server.

pub mod client;
pub mod server_address;
pub mod session;
pub mod transport;

pub use client::{
    check_server_version, kill_server, AdbClient, ClientError, ClientEvent, DefaultSessionFactory,
    ServerLauncher, SessionFactory, ADB_SERVER_VERSION,
};
pub use server_address::{
    resolve_server_address, AddressError, ServerAddress, ServerAddressEnv, ServerAddressOptions,
    DEFAULT_SERVER_PORT,
};
pub use session::{Session, SessionError};
pub use transport::{format_host_command, TransportSelector};

#[cfg(test)]
mod tests {
    #[test]
    fn smoke_test() {
        assert_eq!(2 + 2, 4);
    }
}
