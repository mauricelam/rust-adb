/// Specifies how a connection selects a device transport.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum TransportSelector {
    /// Any available device or emulator (`host:tport:any` or default `host:`).
    #[default]
    Any,
    /// Physical USB device (`host:tport:usb`).
    Usb,
    /// Local emulator or network instance (`host:tport:local`).
    Local,
    /// Specific device serial number (`host:tport:serial:<serial>`).
    Serial(String),
    /// Specific transport ID (`host:transport-id:<id>`).
    Id(u64),
}

impl TransportSelector {
    /// Formats a host command with transport prefix according to precedence:
    /// `Id` > `Serial` > `Usb` / `Local` > `Any` (default `host:`).
    pub fn format_host_command(&self, cmd: &str) -> String {
        format_host_command(cmd, self)
    }
}

/// Formats a host command prefix given a service command and transport selector.
///
/// Precedence:
/// - `TransportSelector::Id(id)` -> `host-transport-id:<id>:<cmd>`
/// - `TransportSelector::Serial(serial)` -> `host-serial:<serial>:<cmd>`
/// - `TransportSelector::Usb` -> `host-usb:<cmd>`
/// - `TransportSelector::Local` -> `host-local:<cmd>`
/// - `TransportSelector::Any` -> `host:<cmd>`
pub fn format_host_command(cmd: &str, selector: &TransportSelector) -> String {
    match selector {
        TransportSelector::Id(id) => format!("host-transport-id:{id}:{cmd}"),
        TransportSelector::Serial(s) => format!("host-serial:{s}:{cmd}"),
        TransportSelector::Usb => format!("host-usb:{cmd}"),
        TransportSelector::Local => format!("host-local:{cmd}"),
        TransportSelector::Any => format!("host:{cmd}"),
    }
}
