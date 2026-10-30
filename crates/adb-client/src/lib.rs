//! adb-client: Client-side library for connecting to and interacting with the ADB server.

pub mod client;
pub mod device_selection;
pub mod server_address;
pub mod session;
pub mod shell_interactive;
pub mod sync_session;
pub mod trace;
pub mod transport;

pub use client::{
    check_server_version, check_server_version_with_options, escape_arg, forward_dest_is_featured,
    forward_targets_are_valid, kill_server, start_server, start_server_with_one_device,
    start_server_with_options, AdbClient, AdbClientBuilder, ClientError, ClientErrorKind,
    ClientEvent, DefaultSessionFactory, ExternalBinaryLauncher, LaunchConfig, LaunchError, Outcome,
    SelfExecLauncher, ServerLauncher, SessionFactory, ShellOptions, TrackDevicesMode,
    ADB_SERVER_VERSION,
};
pub use device_selection::{DeviceSelection, SelectionError};
pub use server_address::{
    resolve_server_address, AddressError, ServerAddress, ServerAddressEnv, ServerAddressOptions,
    DEFAULT_SERVER_PORT,
};
pub use session::{Session, SessionConfig, SessionError};
pub use shell_interactive::{
    determine_pty_allocation, format_window_size, parse_escape_char_arg, EscapeResult, EscapeState,
    EscapeStateMachine, PtyDecision, PtyMode,
};
pub use sync_session::{
    errno_from_wire, errno_to_string, CompressionType, DirEntry, PullOptions, PullResult,
    PullSourceOutcome, PushOptions, PushResult, PushSourceOutcome, Stat, SyncError, SyncSession,
};
pub use transport::{format_host_command, TransportSelector};

#[cfg(test)]
mod tests {
    #[test]
    fn smoke_test() {
        assert_eq!(2 + 2, 4);
    }
}
