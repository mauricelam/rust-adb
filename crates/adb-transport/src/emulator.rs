use crate::tcp::TcpTransport;
use std::time::Duration;
use tokio::net::TcpStream;

/// Default starting local transport ADB port for Android emulators (5555).
pub const DEFAULT_ADB_LOCAL_TRANSPORT_PORT: u16 = 5555;

/// Default maximum local transport ADB port for Android emulators (5585).
/// Upstream C++ ADB scans 16 emulators: `5555 + 16 * 2 - 1 = 5585`.
pub const DEFAULT_ADB_LOCAL_TRANSPORT_MAX_PORT: u16 = 5585;

/// Calculates the console port for a given ADB transport port.
///
/// In standard Android emulator configuration, the console port is `adb_port - 1`.
#[inline]
pub fn console_port_for_adb_port(adb_port: u16) -> u16 {
    adb_port.saturating_sub(1)
}

/// Returns the standard ADB serial string for an emulator from its console port.
///
/// E.g. `emulator-5554` for console port 5554 (ADB port 5555).
#[inline]
pub fn emulator_serial(console_port: u16) -> String {
    format!("emulator-{console_port}")
}

/// Configuration for scanning local or remote ports to discover running emulators.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmulatorScanConfig {
    /// Host to connect to (defaults to `127.0.0.1` or `$ADBHOST` if set).
    pub host: String,
    /// Minimum ADB port to scan (default 5555).
    pub min_port: u16,
    /// Maximum ADB port to scan (default 5585 or `$ADB_LOCAL_TRANSPORT_MAX_PORT`).
    pub max_port: u16,
    /// Port stride/step (default 2, since ports are allocated in pairs: console + adb).
    pub step: u16,
    /// Connection timeout per port attempt.
    pub connect_timeout: Duration,
}

impl Default for EmulatorScanConfig {
    fn default() -> Self {
        let host = std::env::var("ADBHOST")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| "127.0.0.1".to_string());

        let max_port = std::env::var("ADB_LOCAL_TRANSPORT_MAX_PORT")
            .ok()
            .and_then(|val| val.trim().parse::<u16>().ok())
            .unwrap_or(DEFAULT_ADB_LOCAL_TRANSPORT_MAX_PORT);

        Self {
            host,
            min_port: DEFAULT_ADB_LOCAL_TRANSPORT_PORT,
            max_port,
            step: 2,
            connect_timeout: Duration::from_millis(250),
        }
    }
}

/// Represents an active emulator discovered via port scanning.
#[derive(Debug)]
pub struct DiscoveredEmulator {
    /// Emulator serial string (e.g. `emulator-5554`).
    pub serial: String,
    /// The ADB wire protocol TCP port (e.g. 5555).
    pub adb_port: u16,
    /// The emulator console TCP port (e.g. 5554).
    pub console_port: u16,
    /// Connected TCP transport ready for ADB wire packet exchange.
    pub transport: TcpTransport,
}

/// Scans for running emulators using the default configuration and environment variables.
pub async fn scan_for_emulators() -> Vec<DiscoveredEmulator> {
    scan_for_emulators_with_config(&EmulatorScanConfig::default()).await
}

/// Scans for running emulators using the specified scan configuration.
///
/// Port probes are conducted concurrently without blocking other connections or the runtime loop.
pub async fn scan_for_emulators_with_config(
    config: &EmulatorScanConfig,
) -> Vec<DiscoveredEmulator> {
    if config.min_port > config.max_port || config.step == 0 {
        return Vec::new();
    }

    let ports: Vec<u16> = (config.min_port..=config.max_port)
        .step_by(config.step as usize)
        .collect();

    let mut tasks = Vec::with_capacity(ports.len());

    for port in ports {
        let host = config.host.clone();
        let timeout = config.connect_timeout;
        tasks.push(tokio::spawn(async move {
            let addr = format!("{host}:{port}");
            match tokio::time::timeout(timeout, TcpStream::connect(&addr)).await {
                Ok(Ok(stream)) => {
                    let console_port = console_port_for_adb_port(port);
                    let serial = emulator_serial(console_port);
                    let transport = TcpTransport::from_stream(stream);
                    Some(DiscoveredEmulator {
                        serial,
                        adb_port: port,
                        console_port,
                        transport,
                    })
                }
                _ => None,
            }
        }));
    }

    let mut results = Vec::new();
    for task in tasks {
        if let Ok(Some(discovered)) = task.await {
            results.push(discovered);
        }
    }

    // Sort stably by adb_port in ascending order
    results.sort_by_key(|e| e.adb_port);
    results
}
