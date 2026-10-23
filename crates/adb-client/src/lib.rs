//! A library for talking to an ADB server.

pub mod client;
pub mod device_selection;
pub mod server_address;
pub mod session;
pub mod transport;

pub use client::{
    check_server_version, escape_arg, kill_server, start_server, start_server_with_one_device,
    AdbClient, AdbClientBuilder, ClientError, ClientEvent, DefaultSessionFactory,
    ExternalBinaryLauncher, LaunchError, Outcome, SelfExecLauncher, ServerLauncher, SessionFactory,
    ShellOptions, TrackDevicesMode, ADB_SERVER_VERSION,
};
pub use device_selection::{DeviceSelection, SelectionError};
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
