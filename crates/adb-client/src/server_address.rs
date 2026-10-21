use std::fmt;

/// Default ADB server port (5037).
pub const DEFAULT_SERVER_PORT: u16 = 5037;

/// Options passed via command-line flags (-H, -P, -L).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ServerAddressOptions {
    /// Host set via -H <host> or -Hhost
    pub host: Option<String>,
    /// Port set via -P <port> or -Pport
    pub port: Option<String>,
    /// Socket spec set via -L <socket>
    pub socket: Option<String>,
}

/// Environment variables relevant to server address resolution.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ServerAddressEnv {
    /// ADB_SERVER_SOCKET environment variable
    pub adb_server_socket: Option<String>,
    /// ANDROID_ADB_SERVER_ADDRESS environment variable
    pub android_adb_server_address: Option<String>,
    /// ANDROID_ADB_SERVER_PORT environment variable
    pub android_adb_server_port: Option<String>,
}

/// Errors that can occur during server address resolution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AddressError {
    /// -L is incompatible with -H or -P
    IncompatibleOptions,
    /// Invalid port specified in -P or ANDROID_ADB_SERVER_PORT
    InvalidPort(String),
    /// -H requires an argument (if needed during parsing, though options struct handles parsed state)
    MissingOptionArgument(String),
    /// Socket spec string is unsupported (e.g. non-tcp spec like local:, vsock:)
    UnsupportedSocketSpec(String),
    /// Bad port number in socket spec string (e.g. out of u16 range or negative)
    BadPortNumber(String),
    /// Malformed socket spec string
    InvalidSocketSpec(String),
}

impl fmt::Display for AddressError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AddressError::IncompatibleOptions => write!(f, "-L is incompatible with -H or -P"),
            AddressError::InvalidPort(val) => write!(
                f,
                "$ANDROID_ADB_SERVER_PORT must be a positive number less than 65535: got \"{}\"",
                val
            ),
            AddressError::MissingOptionArgument(opt) => write!(f, "{} requires an argument", opt),
            AddressError::UnsupportedSocketSpec(spec) => {
                write!(f, "specification is not tcp: {}", spec)
            }
            AddressError::BadPortNumber(port) => write!(f, "bad port number '{}'", port),
            AddressError::InvalidSocketSpec(spec) => write!(f, "invalid socket spec: {}", spec),
        }
    }
}

impl std::error::Error for AddressError {}

/// Represents a resolved ADB server address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerAddress {
    pub host: Option<String>,
    pub port: u16,
}

impl ServerAddress {
    /// Formats the address as a `tcp:` socket specification.
    ///
    /// The resulting spec is `tcp:<port>` when there is no host,
    /// or `tcp:<host>:<port>` when there is one.
    pub fn to_spec(&self) -> String {
        match &self.host {
            Some(h) => format!("tcp:{}:{}", h, self.port),
            None => format!("tcp:{}", self.port),
        }
    }

    /// Checks if this server address is considered local.
    ///
    /// Following C++ `is_local_socket_spec` / `tcp_host_is_local`:
    /// Returns true if host is None or "localhost".
    pub fn is_local(&self) -> bool {
        match &self.host {
            None => true,
            Some(h) => h.is_empty() || h == "localhost",
        }
    }
}

/// Helper function to parse a `tcp:` socket spec string into `ServerAddress`.
fn parse_tcp_socket_spec(spec: &str) -> Result<ServerAddress, AddressError> {
    if !spec.starts_with("tcp:") {
        return Err(AddressError::UnsupportedSocketSpec(spec.to_string()));
    }

    let rest = &spec[4..];

    // If rest parses directly as integer: tcp:<port>
    if let Ok(port_val) = rest.parse::<i64>() {
        if !(0..=65535).contains(&port_val) {
            return Err(AddressError::BadPortNumber(port_val.to_string()));
        }
        return Ok(ServerAddress {
            host: None,
            port: port_val as u16,
        });
    }

    // Parse host and optional port: tcp:<host>:<port> or tcp:<host>
    // Note: C++ ParseNetAddress splits on last ':' if port is present.
    if let Some((host_part, port_part)) = rest.rsplit_once(':') {
        if let Ok(port_val) = port_part.parse::<i64>() {
            if !(0..=65535).contains(&port_val) {
                return Err(AddressError::BadPortNumber(port_val.to_string()));
            }
            return Ok(ServerAddress {
                host: Some(host_part.to_string()),
                port: port_val as u16,
            });
        } else {
            return Err(AddressError::InvalidSocketSpec(spec.to_string()));
        }
    }

    // Default port 5037 if no port specified in host spec: tcp:<host>
    Ok(ServerAddress {
        host: Some(rest.to_string()),
        port: DEFAULT_SERVER_PORT,
    })
}

/// Resolves the ADB server address based on command-line options and environment variables.
pub fn resolve_server_address(
    options: &ServerAddressOptions,
    env: &ServerAddressEnv,
) -> Result<ServerAddress, AddressError> {
    let server_host_option = options.host.as_deref();
    let server_port_option = options.port.as_deref();
    let mut server_socket_str = options.socket.as_deref();

    // Check incompatibility: -L together with -H or -P is an error
    if (server_host_option.is_some() || server_port_option.is_some())
        && server_socket_str.is_some()
    {
        return Err(AddressError::IncompatibleOptions);
    }

    // If none of -L, -H, -P is given, ADB_SERVER_SOCKET environment variable wins over everything else.
    if server_host_option.is_none()
        && server_port_option.is_none()
        && server_socket_str.is_none()
    {
        server_socket_str = env.adb_server_socket.as_deref();
    }

    if let Some(socket_spec) = server_socket_str {
        return parse_tcp_socket_spec(socket_spec);
    }

    // Otherwise, host is -H or ANDROID_ADB_SERVER_ADDRESS
    let host = server_host_option
        .or(env.android_adb_server_address.as_deref())
        .map(|s| s.to_string());

    // Port is -P or ANDROID_ADB_SERVER_PORT, defaulting to 5037.
    let port_str = server_port_option.or(env.android_adb_server_port.as_deref());

    let port = if let Some(p) = port_str {
        if p.is_empty() {
            DEFAULT_SERVER_PORT
        } else {
            // Parse as integer 1..=65535
            match p.parse::<u32>() {
                Ok(val) if (1..=65535).contains(&val) => val as u16,
                _ => return Err(AddressError::InvalidPort(p.to_string())),
            }
        }
    } else {
        DEFAULT_SERVER_PORT
    };

    Ok(ServerAddress { host, port })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_server_address_table() {
        struct TestCase {
            name: &'static str,
            options: ServerAddressOptions,
            env: ServerAddressEnv,
            expected: Result<ServerAddress, AddressError>,
            expected_spec: Option<&'static str>,
            expected_is_local: Option<bool>,
        }

        let test_cases = vec![
            TestCase {
                name: "defaults (no options, no env)",
                options: ServerAddressOptions::default(),
                env: ServerAddressEnv::default(),
                expected: Ok(ServerAddress {
                    host: None,
                    port: 5037,
                }),
                expected_spec: Some("tcp:5037"),
                expected_is_local: Some(true),
            },
            TestCase {
                name: "-H localhost",
                options: ServerAddressOptions {
                    host: Some("localhost".into()),
                    ..Default::default()
                },
                env: ServerAddressEnv::default(),
                expected: Ok(ServerAddress {
                    host: Some("localhost".into()),
                    port: 5037,
                }),
                expected_spec: Some("tcp:localhost:5037"),
                expected_is_local: Some(true),
            },
            TestCase {
                name: "-H remote_host",
                options: ServerAddressOptions {
                    host: Some("192.168.1.1".into()),
                    ..Default::default()
                },
                env: ServerAddressEnv::default(),
                expected: Ok(ServerAddress {
                    host: Some("192.168.1.1".into()),
                    port: 5037,
                }),
                expected_spec: Some("tcp:192.168.1.1:5037"),
                expected_is_local: Some(false),
            },
            TestCase {
                name: "-P 1234",
                options: ServerAddressOptions {
                    port: Some("1234".into()),
                    ..Default::default()
                },
                env: ServerAddressEnv::default(),
                expected: Ok(ServerAddress {
                    host: None,
                    port: 1234,
                }),
                expected_spec: Some("tcp:1234"),
                expected_is_local: Some(true),
            },
            TestCase {
                name: "-H example.com -P 8080",
                options: ServerAddressOptions {
                    host: Some("example.com".into()),
                    port: Some("8080".into()),
                    ..Default::default()
                },
                env: ServerAddressEnv::default(),
                expected: Ok(ServerAddress {
                    host: Some("example.com".into()),
                    port: 8080,
                }),
                expected_spec: Some("tcp:example.com:8080"),
                expected_is_local: Some(false),
            },
            TestCase {
                name: "-P empty string (counts as unset)",
                options: ServerAddressOptions {
                    port: Some("".into()),
                    ..Default::default()
                },
                env: ServerAddressEnv::default(),
                expected: Ok(ServerAddress {
                    host: None,
                    port: 5037,
                }),
                expected_spec: Some("tcp:5037"),
                expected_is_local: Some(true),
            },
            TestCase {
                name: "-P 0 (invalid, must be 1..=65535)",
                options: ServerAddressOptions {
                    port: Some("0".into()),
                    ..Default::default()
                },
                env: ServerAddressEnv::default(),
                expected: Err(AddressError::InvalidPort("0".into())),
                expected_spec: None,
                expected_is_local: None,
            },
            TestCase {
                name: "-P 65535 (upper valid boundary)",
                options: ServerAddressOptions {
                    port: Some("65535".into()),
                    ..Default::default()
                },
                env: ServerAddressEnv::default(),
                expected: Ok(ServerAddress {
                    host: None,
                    port: 65535,
                }),
                expected_spec: Some("tcp:65535"),
                expected_is_local: Some(true),
            },
            TestCase {
                name: "-P 65536 (invalid, out of range)",
                options: ServerAddressOptions {
                    port: Some("65536".into()),
                    ..Default::default()
                },
                env: ServerAddressEnv::default(),
                expected: Err(AddressError::InvalidPort("65536".into())),
                expected_spec: None,
                expected_is_local: None,
            },
            TestCase {
                name: "-P non-number",
                options: ServerAddressOptions {
                    port: Some("abc".into()),
                    ..Default::default()
                },
                env: ServerAddressEnv::default(),
                expected: Err(AddressError::InvalidPort("abc".into())),
                expected_spec: None,
                expected_is_local: None,
            },
            TestCase {
                name: "-L tcp:1234",
                options: ServerAddressOptions {
                    socket: Some("tcp:1234".into()),
                    ..Default::default()
                },
                env: ServerAddressEnv::default(),
                expected: Ok(ServerAddress {
                    host: None,
                    port: 1234,
                }),
                expected_spec: Some("tcp:1234"),
                expected_is_local: Some(true),
            },
            TestCase {
                name: "-L tcp:localhost:1234",
                options: ServerAddressOptions {
                    socket: Some("tcp:localhost:1234".into()),
                    ..Default::default()
                },
                env: ServerAddressEnv::default(),
                expected: Ok(ServerAddress {
                    host: Some("localhost".into()),
                    port: 1234,
                }),
                expected_spec: Some("tcp:localhost:1234"),
                expected_is_local: Some(true),
            },
            TestCase {
                name: "-L and -H combined error",
                options: ServerAddressOptions {
                    socket: Some("tcp:1".into()),
                    host: Some("x".into()),
                    ..Default::default()
                },
                env: ServerAddressEnv::default(),
                expected: Err(AddressError::IncompatibleOptions),
                expected_spec: None,
                expected_is_local: None,
            },
            TestCase {
                name: "-L and -P combined error",
                options: ServerAddressOptions {
                    socket: Some("tcp:1".into()),
                    port: Some("1234".into()),
                    ..Default::default()
                },
                env: ServerAddressEnv::default(),
                expected: Err(AddressError::IncompatibleOptions),
                expected_spec: None,
                expected_is_local: None,
            },
            TestCase {
                name: "-L non-tcp spec (localreserved:foo)",
                options: ServerAddressOptions {
                    socket: Some("localreserved:foo".into()),
                    ..Default::default()
                },
                env: ServerAddressEnv::default(),
                expected: Err(AddressError::UnsupportedSocketSpec(
                    "localreserved:foo".into(),
                )),
                expected_spec: None,
                expected_is_local: None,
            },
            TestCase {
                name: "-L tcp:65536 (bad port in tcp spec)",
                options: ServerAddressOptions {
                    socket: Some("tcp:65536".into()),
                    ..Default::default()
                },
                env: ServerAddressEnv::default(),
                expected: Err(AddressError::BadPortNumber("65536".into())),
                expected_spec: None,
                expected_is_local: None,
            },
            TestCase {
                name: "ADB_SERVER_SOCKET wins when no options given",
                options: ServerAddressOptions::default(),
                env: ServerAddressEnv {
                    adb_server_socket: Some("tcp:9999".into()),
                    android_adb_server_address: Some("10.0.0.1".into()),
                    android_adb_server_port: Some("5555".into()),
                },
                expected: Ok(ServerAddress {
                    host: None,
                    port: 9999,
                }),
                expected_spec: Some("tcp:9999"),
                expected_is_local: Some(true),
            },
            TestCase {
                name: "ANDROID_ADB_SERVER_ADDRESS and PORT used when no options or ADB_SERVER_SOCKET",
                options: ServerAddressOptions::default(),
                env: ServerAddressEnv {
                    android_adb_server_address: Some("10.0.0.1".into()),
                    android_adb_server_port: Some("5555".into()),
                    ..Default::default()
                },
                expected: Ok(ServerAddress {
                    host: Some("10.0.0.1".into()),
                    port: 5555,
                }),
                expected_spec: Some("tcp:10.0.0.1:5555"),
                expected_is_local: Some(false),
            },
            TestCase {
                name: "-H option ignores ADB_SERVER_SOCKET",
                options: ServerAddressOptions {
                    host: Some("host1".into()),
                    ..Default::default()
                },
                env: ServerAddressEnv {
                    adb_server_socket: Some("tcp:8888".into()),
                    ..Default::default()
                },
                expected: Ok(ServerAddress {
                    host: Some("host1".into()),
                    port: 5037,
                }),
                expected_spec: Some("tcp:host1:5037"),
                expected_is_local: Some(false),
            },
            TestCase {
                name: "-P option ignores ADB_SERVER_SOCKET",
                options: ServerAddressOptions {
                    port: Some("1234".into()),
                    ..Default::default()
                },
                env: ServerAddressEnv {
                    adb_server_socket: Some("tcp:8888".into()),
                    ..Default::default()
                },
                expected: Ok(ServerAddress {
                    host: None,
                    port: 1234,
                }),
                expected_spec: Some("tcp:1234"),
                expected_is_local: Some(true),
            },
            TestCase {
                name: "-L option ignores ADB_SERVER_SOCKET",
                options: ServerAddressOptions {
                    socket: Some("tcp:1111".into()),
                    ..Default::default()
                },
                env: ServerAddressEnv {
                    adb_server_socket: Some("tcp:8888".into()),
                    ..Default::default()
                },
                expected: Ok(ServerAddress {
                    host: None,
                    port: 1111,
                }),
                expected_spec: Some("tcp:1111"),
                expected_is_local: Some(true),
            },
            TestCase {
                name: "-H option overrides ANDROID_ADB_SERVER_ADDRESS",
                options: ServerAddressOptions {
                    host: Some("host1".into()),
                    ..Default::default()
                },
                env: ServerAddressEnv {
                    android_adb_server_address: Some("host2".into()),
                    ..Default::default()
                },
                expected: Ok(ServerAddress {
                    host: Some("host1".into()),
                    port: 5037,
                }),
                expected_spec: Some("tcp:host1:5037"),
                expected_is_local: Some(false),
            },
            TestCase {
                name: "-P option overrides ANDROID_ADB_SERVER_PORT",
                options: ServerAddressOptions {
                    port: Some("1234".into()),
                    ..Default::default()
                },
                env: ServerAddressEnv {
                    android_adb_server_port: Some("5678".into()),
                    ..Default::default()
                },
                expected: Ok(ServerAddress {
                    host: None,
                    port: 1234,
                }),
                expected_spec: Some("tcp:1234"),
                expected_is_local: Some(true),
            },
            TestCase {
                name: "ANDROID_ADB_SERVER_PORT=0 error",
                options: ServerAddressOptions::default(),
                env: ServerAddressEnv {
                    android_adb_server_port: Some("0".into()),
                    ..Default::default()
                },
                expected: Err(AddressError::InvalidPort("0".into())),
                expected_spec: None,
                expected_is_local: None,
            },
            TestCase {
                name: "ANDROID_ADB_SERVER_PORT empty string counts as unset",
                options: ServerAddressOptions::default(),
                env: ServerAddressEnv {
                    android_adb_server_port: Some("".into()),
                    ..Default::default()
                },
                expected: Ok(ServerAddress {
                    host: None,
                    port: 5037,
                }),
                expected_spec: Some("tcp:5037"),
                expected_is_local: Some(true),
            },
        ];

        for tc in test_cases {
            let res = resolve_server_address(&tc.options, &tc.env);
            assert_eq!(res, tc.expected, "Failed test case: {}", tc.name);
            if let Ok(addr) = &res {
                if let Some(expected_spec) = tc.expected_spec {
                    assert_eq!(
                        addr.to_spec(),
                        expected_spec,
                        "Failed spec for test case: {}",
                        tc.name
                    );
                }
                if let Some(expected_is_local) = tc.expected_is_local {
                    assert_eq!(
                        addr.is_local(),
                        expected_is_local,
                        "Failed is_local for test case: {}",
                        tc.name
                    );
                }
            }
        }
    }
}
