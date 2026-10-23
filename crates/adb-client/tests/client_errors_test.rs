use adb_client::{
    ClientError, ClientErrorKind, ClientEvent, LaunchError, SessionError,
};
use std::io;

#[test]
fn test_client_error_display_bare() {
    // Session errors wrapped in ClientError display bare without "error:" prefix
    let fail_err = ClientError::Session(SessionError::Fail("no devices/emulators found".to_string()));
    assert_eq!(fail_err.to_string(), "no devices/emulators found");

    let cannot_conn_session = ClientError::Session(SessionError::CannotConnect {
        spec: "tcp:5037".to_string(),
        cause: "Connection refused".to_string(),
    });
    assert_eq!(
        cannot_conn_session.to_string(),
        "cannot connect to daemon at tcp:5037: Connection refused"
    );

    let status_failed = ClientError::Session(SessionError::ReadStatusFailed(io::Error::new(
        io::ErrorKind::ConnectionReset,
        "Connection reset by peer",
    )));
    assert_eq!(
        status_failed.to_string(),
        "protocol fault (couldn't read status): Connection reset by peer"
    );

    let bad_len = ClientError::Session(SessionError::BadServiceLength(100000));
    assert_eq!(bad_len.to_string(), "bad service name length (100000)");

    // Direct ClientError variants
    let cannot_conn = ClientError::CannotConnect {
        spec: "tcp:5037".to_string(),
        cause: "Connection refused".to_string(),
    };
    assert_eq!(
        cannot_conn.to_string(),
        "cannot connect to daemon at tcp:5037: Connection refused"
    );

    let cannot_conn_daemon = ClientError::CannotConnectDaemon { source: None };
    assert_eq!(cannot_conn_daemon.to_string(), "cannot connect to daemon");

    let cannot_conn_daemon_with_source = ClientError::CannotConnectDaemon {
        source: Some(LaunchError::Unsupported("Windows server launching is not supported yet".to_string())),
    };
    assert_eq!(
        cannot_conn_daemon_with_source.to_string(),
        "cannot connect to daemon"
    );

    let parse_ver = ClientError::CannotParseVersionString("invalid".to_string());
    assert_eq!(parse_ver.to_string(), "cannot parse version string: invalid");

    let kill_failed = ClientError::KillFailed("cannot kill".to_string());
    assert_eq!(kill_failed.to_string(), "cannot kill");

    let check_ver = ClientError::FailedToCheckVersion("timeout".to_string());
    assert_eq!(check_ver.to_string(), "failed to check server version: timeout");

    let query_failed = ClientError::QueryFailed("no devices/emulators found".to_string());
    assert_eq!(query_failed.to_string(), "no devices/emulators found");

    let invalid_usage = ClientError::InvalidUsage("usage: adb connect HOST[:PORT]".to_string());
    assert_eq!(invalid_usage.to_string(), "usage: adb connect HOST[:PORT]");
}

#[test]
fn test_client_error_kind() {
    assert_eq!(
        ClientError::Session(SessionError::Fail("err".to_string())).kind(),
        ClientErrorKind::Server
    );
    assert_eq!(
        ClientError::Session(SessionError::CannotConnect {
            spec: "tcp:5037".to_string(),
            cause: "refused".to_string()
        })
        .kind(),
        ClientErrorKind::Connection
    );
    assert_eq!(
        ClientError::Session(SessionError::ReadStatusFailed(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "eof"
        )))
        .kind(),
        ClientErrorKind::Protocol
    );

    assert_eq!(
        ClientError::CannotConnect {
            spec: "tcp:5037".to_string(),
            cause: "refused".to_string()
        }
        .kind(),
        ClientErrorKind::Connection
    );

    assert_eq!(
        ClientError::CannotConnectDaemon { source: None }.kind(),
        ClientErrorKind::Connection
    );
    assert_eq!(
        ClientError::CannotConnectDaemon {
            source: Some(LaunchError::Unsupported("unsupported".to_string()))
        }
        .kind(),
        ClientErrorKind::Launch
    );

    assert_eq!(
        ClientError::CannotParseVersionString("abc".to_string()).kind(),
        ClientErrorKind::Version
    );
    assert_eq!(
        ClientError::KillFailed("failed".to_string()).kind(),
        ClientErrorKind::Server
    );
    assert_eq!(
        ClientError::FailedToCheckVersion("failed".to_string()).kind(),
        ClientErrorKind::Server
    );
    assert_eq!(
        ClientError::QueryFailed("failed".to_string()).kind(),
        ClientErrorKind::Server
    );
    assert_eq!(
        ClientError::InvalidUsage("usage".to_string()).kind(),
        ClientErrorKind::Usage
    );
}

#[test]
fn test_client_error_source() {
    use std::error::Error;

    let launch_err = LaunchError::Unsupported("unsupported".to_string());
    let err = ClientError::CannotConnectDaemon {
        source: Some(launch_err),
    };
    assert!(err.source().is_some());
    assert_eq!(
        err.source().unwrap().to_string(),
        "unsupported"
    );

    let err_no_source = ClientError::CannotConnectDaemon { source: None };
    assert!(err_no_source.source().is_none());
}

#[test]
fn test_client_event_display() {
    let ver_mismatch = ClientEvent::ServerVersionMismatch {
        server: 40,
        client: 41,
    };
    assert_eq!(
        ver_mismatch.to_string(),
        "adb server version (40) doesn't match this client (41); killing..."
    );

    let daemon_not_running = ClientEvent::DaemonNotRunning {
        spec: "tcp:5037".to_string(),
    };
    assert_eq!(
        daemon_not_running.to_string(),
        "* daemon not running; starting now at tcp:5037"
    );

    let cannot_start_remote = ClientEvent::CannotStartRemote;
    assert_eq!(
        cannot_start_remote.to_string(),
        "* cannot start server on remote host"
    );

    let cannot_conn_daemon = ClientEvent::CannotConnectDaemon {
        spec: "tcp:5037".to_string(),
        cause: "Connection refused".to_string(),
    };
    assert_eq!(
        cannot_conn_daemon.to_string(),
        "cannot connect to daemon at tcp:5037: Connection refused"
    );

    let error_line = ClientEvent::ErrorLine("error: write failure during connection: Broken pipe".to_string());
    assert_eq!(
        error_line.to_string(),
        "error: write failure during connection: Broken pipe"
    );

    let failed_to_start = ClientEvent::FailedToStartDaemon;
    assert_eq!(failed_to_start.to_string(), "* failed to start daemon");

    let daemon_started = ClientEvent::DaemonStarted;
    assert_eq!(daemon_started.to_string(), "* daemon started successfully");
}
