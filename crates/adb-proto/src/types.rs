//! Strongly-typed domain wrappers for IDs, ports, protocol versions, and payload sizes.
//!
//! Replaces primitive integers and manual invariant checks with newtypes that enforce
//! non-zero guarantees and provide wire serialization and negotiation methods.

use std::fmt;
use std::num::{NonZeroU32, NonZeroU64, ParseIntError};
use std::ops::Deref;
use std::str::FromStr;

/// Delayed acknowledgment byte count in `A_OKAY` packets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AckBytes(pub u32);

impl AckBytes {
    /// Creates a new `AckBytes`.
    pub const fn new(val: u32) -> Self {
        Self(val)
    }

    /// Returns the raw `u32` byte count.
    pub const fn get(&self) -> u32 {
        self.0
    }

    /// Encodes `AckBytes` to 4 little-endian bytes.
    pub fn to_le_bytes(&self) -> [u8; 4] {
        self.0.to_le_bytes()
    }

    /// Decodes `AckBytes` from 4 little-endian bytes.
    pub fn from_le_bytes(bytes: [u8; 4]) -> Self {
        Self(u32::from_le_bytes(bytes))
    }
}

impl fmt::Display for AckBytes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<u32> for AckBytes {
    fn from(val: u32) -> Self {
        Self(val)
    }
}

impl From<AckBytes> for u32 {
    fn from(val: AckBytes) -> Self {
        val.0
    }
}

impl Deref for AckBytes {
    type Target = u32;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl PartialEq<u32> for AckBytes {
    fn eq(&self, other: &u32) -> bool {
        self.0 == *other
    }
}

impl PartialEq<AckBytes> for u32 {
    fn eq(&self, other: &AckBytes) -> bool {
        *self == other.0
    }
}

/// Expected ADB server version matching C++ `ADB_SERVER_VERSION` in `original/adb.h`.
pub const ADB_SERVER_VERSION: u32 = 41;

/// Newtype representing an ADB server version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ServerVersion(pub u32);

/// Global constant for the canonical server version.
pub const SERVER_VERSION: ServerVersion = ServerVersion(ADB_SERVER_VERSION);

impl ServerVersion {
    /// Creates a new `ServerVersion`.
    pub const fn new(val: u32) -> Self {
        Self(val)
    }

    /// Returns the raw u32 version number.
    pub const fn get(&self) -> u32 {
        self.0
    }
}

impl fmt::Display for ServerVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for ServerVersion {
    type Err = ParseIntError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        s.parse::<u32>().map(Self)
    }
}

impl From<u32> for ServerVersion {
    fn from(val: u32) -> Self {
        Self(val)
    }
}

impl From<ServerVersion> for u32 {
    fn from(val: ServerVersion) -> Self {
        val.0
    }
}

impl Deref for ServerVersion {
    type Target = u32;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl PartialEq<u32> for ServerVersion {
    fn eq(&self, other: &u32) -> bool {
        self.0 == *other
    }
}

impl PartialEq<ServerVersion> for u32 {
    fn eq(&self, other: &ServerVersion) -> bool {
        *self == other.0
    }
}

/// Error returned when parsing or converting an ID that violates the non-zero invariant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IdError {
    /// The ID was 0.
    Zero,
    /// An integer parsing error occurred.
    Parse(ParseIntError),
}

impl fmt::Display for IdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Zero => write!(f, "id cannot be zero"),
            Self::Parse(e) => write!(f, "failed to parse id: {e}"),
        }
    }
}

impl std::error::Error for IdError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Parse(e) => Some(e),
            _ => None,
        }
    }
}

impl From<ParseIntError> for IdError {
    fn from(e: ParseIntError) -> Self {
        Self::Parse(e)
    }
}

/// Strongly-typed non-zero local stream identifier.
///
/// Ported from `local_id` in `original/sockets.cpp`. Local stream IDs are strictly non-zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LocalId(NonZeroU32);

impl LocalId {
    /// Creates a `LocalId` from a `NonZeroU32`.
    pub const fn new(val: NonZeroU32) -> Self {
        Self(val)
    }

    /// Converts a raw `u32` into a `LocalId`, returning `None` if zero.
    pub fn from_u32(val: u32) -> Option<Self> {
        NonZeroU32::new(val).map(Self)
    }

    /// Returns the raw non-zero `u32` value.
    pub const fn get(&self) -> u32 {
        self.0.get()
    }

    /// Returns the inner `NonZeroU32`.
    pub const fn into_inner(self) -> NonZeroU32 {
        self.0
    }
}

impl fmt::Display for LocalId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for LocalId {
    type Err = IdError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let val = s.parse::<NonZeroU32>().map_err(|e| {
            if s.trim() == "0" {
                IdError::Zero
            } else {
                IdError::Parse(e)
            }
        })?;
        Ok(Self(val))
    }
}

impl From<NonZeroU32> for LocalId {
    fn from(val: NonZeroU32) -> Self {
        Self(val)
    }
}

impl From<LocalId> for NonZeroU32 {
    fn from(id: LocalId) -> Self {
        id.0
    }
}

impl From<LocalId> for u32 {
    fn from(id: LocalId) -> Self {
        id.0.get()
    }
}

impl Deref for LocalId {
    type Target = NonZeroU32;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl PartialEq<u32> for LocalId {
    fn eq(&self, other: &u32) -> bool {
        self.0.get() == *other
    }
}

impl PartialEq<LocalId> for u32 {
    fn eq(&self, other: &LocalId) -> bool {
        *self == other.0.get()
    }
}

impl TryFrom<u32> for LocalId {
    type Error = IdError;

    fn try_from(val: u32) -> Result<Self, Self::Error> {
        Self::from_u32(val).ok_or(IdError::Zero)
    }
}

/// Strongly-typed non-zero remote peer stream identifier.
///
/// Ported from `remote_id` in `original/sockets.cpp`. Peer stream IDs are strictly non-zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RemoteId(NonZeroU32);

impl RemoteId {
    /// Creates a `RemoteId` from a `NonZeroU32`.
    pub const fn new(val: NonZeroU32) -> Self {
        Self(val)
    }

    /// Converts a raw `u32` into a `RemoteId`, returning `None` if zero.
    pub fn from_u32(val: u32) -> Option<Self> {
        NonZeroU32::new(val).map(Self)
    }

    /// Returns the raw non-zero `u32` value.
    pub const fn get(&self) -> u32 {
        self.0.get()
    }

    /// Returns the inner `NonZeroU32`.
    pub const fn into_inner(self) -> NonZeroU32 {
        self.0
    }
}

impl fmt::Display for RemoteId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for RemoteId {
    type Err = IdError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let val = s.parse::<NonZeroU32>().map_err(|e| {
            if s.trim() == "0" {
                IdError::Zero
            } else {
                IdError::Parse(e)
            }
        })?;
        Ok(Self(val))
    }
}

impl From<NonZeroU32> for RemoteId {
    fn from(val: NonZeroU32) -> Self {
        Self(val)
    }
}

impl From<RemoteId> for NonZeroU32 {
    fn from(id: RemoteId) -> Self {
        id.0
    }
}

impl From<RemoteId> for u32 {
    fn from(id: RemoteId) -> Self {
        id.0.get()
    }
}

impl Deref for RemoteId {
    type Target = NonZeroU32;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl PartialEq<u32> for RemoteId {
    fn eq(&self, other: &u32) -> bool {
        self.0.get() == *other
    }
}

impl PartialEq<RemoteId> for u32 {
    fn eq(&self, other: &RemoteId) -> bool {
        *self == other.0.get()
    }
}

impl TryFrom<u32> for RemoteId {
    type Error = IdError;

    fn try_from(val: u32) -> Result<Self, Self::Error> {
        Self::from_u32(val).ok_or(IdError::Zero)
    }
}

/// Strongly-typed non-zero monotonic transport identifier.
///
/// Ported from `TransportId` in `original/transport.h` and `original/transport.cpp`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TransportId(NonZeroU64);

impl TransportId {
    /// Creates a `TransportId` from a `NonZeroU64`.
    pub const fn new(val: NonZeroU64) -> Self {
        Self(val)
    }

    /// Converts a raw `u64` into a `TransportId`, returning `None` if zero.
    pub fn from_u64(val: u64) -> Option<Self> {
        NonZeroU64::new(val).map(Self)
    }

    /// Returns the raw non-zero `u64` value.
    pub const fn get(&self) -> u64 {
        self.0.get()
    }

    /// Returns the inner `NonZeroU64`.
    pub const fn into_inner(self) -> NonZeroU64 {
        self.0
    }

    /// Encodes this transport ID into 8 little-endian bytes for wire transmission.
    pub fn to_le_bytes(&self) -> [u8; 8] {
        self.0.get().to_le_bytes()
    }

    /// Decodes an 8-byte little-endian array into a `TransportId`, returning `None` if zero.
    pub fn from_le_bytes(bytes: [u8; 8]) -> Option<Self> {
        Self::from_u64(u64::from_le_bytes(bytes))
    }

    /// Decodes an 8-byte native-endian array into a `TransportId`, returning `None` if zero.
    pub fn from_ne_bytes(bytes: [u8; 8]) -> Option<Self> {
        Self::from_u64(u64::from_ne_bytes(bytes))
    }
}

impl fmt::Display for TransportId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for TransportId {
    type Err = IdError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let val = s.parse::<NonZeroU64>().map_err(|e| {
            if s.trim() == "0" {
                IdError::Zero
            } else {
                IdError::Parse(e)
            }
        })?;
        Ok(Self(val))
    }
}

impl From<NonZeroU64> for TransportId {
    fn from(val: NonZeroU64) -> Self {
        Self(val)
    }
}

impl From<TransportId> for NonZeroU64 {
    fn from(id: TransportId) -> Self {
        id.0
    }
}

impl From<TransportId> for u64 {
    fn from(id: TransportId) -> Self {
        id.0.get()
    }
}

impl Deref for TransportId {
    type Target = NonZeroU64;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl PartialEq<u64> for TransportId {
    fn eq(&self, other: &u64) -> bool {
        self.0.get() == *other
    }
}

impl PartialEq<TransportId> for u64 {
    fn eq(&self, other: &TransportId) -> bool {
        *self == other.0.get()
    }
}

impl TryFrom<u64> for TransportId {
    type Error = IdError;

    fn try_from(val: u64) -> Result<Self, Self::Error> {
        Self::from_u64(val).ok_or(IdError::Zero)
    }
}

/// Strongly-typed transport wire protocol version.
///
/// Ported from `A_VERSION` in `original/adb.h`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProtocolVersion(pub u32);

impl ProtocolVersion {
    /// Creates a new `ProtocolVersion`.
    pub const fn new(val: u32) -> Self {
        Self(val)
    }

    /// Returns the raw u32 version.
    pub const fn get(&self) -> u32 {
        self.0
    }

    /// Negotiates protocol version as the minimum of self and other.
    ///
    /// Ported from `atransport::set_protocol_version` in `original/transport.cpp`.
    pub fn negotiate(self, other: ProtocolVersion) -> ProtocolVersion {
        ProtocolVersion(std::cmp::min(self.0, other.0))
    }
}

impl fmt::Display for ProtocolVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:#010x}", self.0)
    }
}

impl FromStr for ProtocolVersion {
    type Err = ParseIntError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
            u32::from_str_radix(hex, 16).map(Self)
        } else {
            s.parse::<u32>().map(Self)
        }
    }
}

impl From<u32> for ProtocolVersion {
    fn from(val: u32) -> Self {
        Self(val)
    }
}

impl From<ProtocolVersion> for u32 {
    fn from(val: ProtocolVersion) -> Self {
        val.0
    }
}

impl Deref for ProtocolVersion {
    type Target = u32;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl PartialEq<u32> for ProtocolVersion {
    fn eq(&self, other: &u32) -> bool {
        self.0 == *other
    }
}

impl PartialEq<ProtocolVersion> for u32 {
    fn eq(&self, other: &ProtocolVersion) -> bool {
        *self == other.0
    }
}

/// Strongly-typed maximum payload size limit in bytes.
///
/// Ported from `MAX_PAYLOAD` in `original/adb.h`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MaxPayload(pub u32);

impl MaxPayload {
    /// Creates a new `MaxPayload`.
    pub const fn new(val: u32) -> Self {
        Self(val)
    }

    /// Returns the raw u32 payload size.
    pub const fn get(&self) -> u32 {
        self.0
    }

    /// Returns the payload size as `usize`.
    pub const fn as_usize(&self) -> usize {
        self.0 as usize
    }

    /// Negotiates maximum payload as the minimum of self and other.
    ///
    /// Ported from `atransport::set_max_payload` in `original/transport.cpp`.
    pub fn negotiate(self, other: MaxPayload) -> MaxPayload {
        MaxPayload(std::cmp::min(self.0, other.0))
    }
}

impl fmt::Display for MaxPayload {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for MaxPayload {
    type Err = ParseIntError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        s.parse::<u32>().map(Self)
    }
}

impl From<u32> for MaxPayload {
    fn from(val: u32) -> Self {
        Self(val)
    }
}

impl From<MaxPayload> for u32 {
    fn from(val: MaxPayload) -> Self {
        val.0
    }
}

impl From<MaxPayload> for usize {
    fn from(val: MaxPayload) -> Self {
        val.0 as usize
    }
}

impl Deref for MaxPayload {
    type Target = u32;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl PartialEq<u32> for MaxPayload {
    fn eq(&self, other: &u32) -> bool {
        self.0 == *other
    }
}

impl PartialEq<MaxPayload> for u32 {
    fn eq(&self, other: &MaxPayload) -> bool {
        *self == other.0
    }
}

impl PartialEq<usize> for MaxPayload {
    fn eq(&self, other: &usize) -> bool {
        self.0 as usize == *other
    }
}

impl PartialEq<MaxPayload> for usize {
    fn eq(&self, other: &MaxPayload) -> bool {
        *self == other.0 as usize
    }
}

impl PartialEq<i32> for MaxPayload {
    fn eq(&self, other: &i32) -> bool {
        if *other < 0 {
            false
        } else {
            self.0 == *other as u32
        }
    }
}

impl PartialEq<MaxPayload> for i32 {
    fn eq(&self, other: &MaxPayload) -> bool {
        other == self
    }
}

/// Strongly-typed ADB wire protocol TCP port.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AdbPort(pub u16);

impl AdbPort {
    /// Creates a new `AdbPort`.
    pub const fn new(port: u16) -> Self {
        Self(port)
    }

    /// Returns the raw u16 port.
    pub const fn get(&self) -> u16 {
        self.0
    }
}

impl fmt::Display for AdbPort {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for AdbPort {
    type Err = ParseIntError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        s.parse::<u16>().map(Self)
    }
}

impl From<u16> for AdbPort {
    fn from(val: u16) -> Self {
        Self(val)
    }
}

impl From<AdbPort> for u16 {
    fn from(val: AdbPort) -> Self {
        val.0
    }
}

impl Deref for AdbPort {
    type Target = u16;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl PartialEq<u16> for AdbPort {
    fn eq(&self, other: &u16) -> bool {
        self.0 == *other
    }
}

impl PartialEq<AdbPort> for u16 {
    fn eq(&self, other: &AdbPort) -> bool {
        *self == other.0
    }
}

/// Strongly-typed Android emulator console TCP port.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ConsolePort(pub u16);

impl ConsolePort {
    /// Creates a new `ConsolePort`.
    pub const fn new(port: u16) -> Self {
        Self(port)
    }

    /// Returns the raw u16 port.
    pub const fn get(&self) -> u16 {
        self.0
    }

    /// Derives the console port from an ADB transport port (`adb_port - 1`).
    ///
    /// In standard Android emulator configuration, `adb_port` is an odd number and
    /// `console_port` is `adb_port - 1`. If `adb_port` is 0, returns `None`.
    pub fn from_adb_port(adb_port: AdbPort) -> Option<Self> {
        adb_port.0.checked_sub(1).map(Self)
    }

    /// Returns the canonical serial number string for this console port (e.g. `emulator-5554`).
    pub fn to_emulator_serial(&self) -> String {
        format!("emulator-{}", self.0)
    }
}

impl fmt::Display for ConsolePort {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for ConsolePort {
    type Err = ParseIntError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        s.parse::<u16>().map(Self)
    }
}

impl From<u16> for ConsolePort {
    fn from(val: u16) -> Self {
        Self(val)
    }
}

impl From<ConsolePort> for u16 {
    fn from(val: ConsolePort) -> Self {
        val.0
    }
}

impl Deref for ConsolePort {
    type Target = u16;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl PartialEq<u16> for ConsolePort {
    fn eq(&self, other: &u16) -> bool {
        self.0 == *other
    }
}

impl PartialEq<ConsolePort> for u16 {
    fn eq(&self, other: &ConsolePort) -> bool {
        *self == other.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_local_id_invariants() {
        assert!(LocalId::from_u32(0).is_none());
        assert!("0".parse::<LocalId>().is_err());
        assert!("abc".parse::<LocalId>().is_err());

        let id = LocalId::from_u32(42).unwrap();
        assert_eq!(id.get(), 42);
        assert_eq!(u32::from(id), 42);
        assert_eq!(id, 42);
        assert_eq!(42, id);
        assert_eq!(id.to_string(), "42");
        assert_eq!("42".parse::<LocalId>().unwrap(), id);
        assert_eq!(LocalId::try_from(42).unwrap(), id);
        assert!(LocalId::try_from(0).is_err());
    }

    #[test]
    fn test_remote_id_invariants() {
        assert!(RemoteId::from_u32(0).is_none());
        assert!("0".parse::<RemoteId>().is_err());

        let id = RemoteId::from_u32(100).unwrap();
        assert_eq!(id.get(), 100);
        assert_eq!(u32::from(id), 100);
        assert_eq!(id, 100);
        assert_eq!(100, id);
        assert_eq!(id.to_string(), "100");
        assert_eq!("100".parse::<RemoteId>().unwrap(), id);
        assert_eq!(RemoteId::try_from(100).unwrap(), id);
        assert!(RemoteId::try_from(0).is_err());
    }

    #[test]
    fn test_transport_id_invariants_and_wire() {
        assert!(TransportId::from_u64(0).is_none());
        assert!("0".parse::<TransportId>().is_err());

        let id = TransportId::from_u64(12345).unwrap();
        assert_eq!(id.get(), 12345);
        assert_eq!(u64::from(id), 12345);
        assert_eq!(id, 12345);
        assert_eq!(12345, id);
        assert_eq!(id.to_string(), "12345");
        assert_eq!("12345".parse::<TransportId>().unwrap(), id);

        let le = id.to_le_bytes();
        assert_eq!(TransportId::from_le_bytes(le).unwrap(), id);
        assert!(TransportId::from_le_bytes([0u8; 8]).is_none());
    }

    #[test]
    fn test_protocol_version_negotiate() {
        let v1 = ProtocolVersion::new(0x0100_0000);
        let v2 = ProtocolVersion::new(0x0100_0001);
        assert_eq!(v1.negotiate(v2), v1);
        assert_eq!(v2.negotiate(v1), v1);
        assert_eq!(v1, 0x0100_0000);
        assert_eq!(0x0100_0000, v1);
    }

    #[test]
    fn test_max_payload_negotiate() {
        let p1 = MaxPayload::new(4096);
        let p2 = MaxPayload::new(1024 * 1024);
        assert_eq!(p1.negotiate(p2), p1);
        assert_eq!(p2.negotiate(p1), p1);
        assert_eq!(p1, 4096u32);
        assert_eq!(p1, 4096usize);
        assert_eq!(p1.as_usize(), 4096);
    }

    #[test]
    fn test_adb_port_and_console_port() {
        let adb_port = AdbPort::new(5555);
        assert_eq!(adb_port, 5555);
        let console_port = ConsolePort::from_adb_port(adb_port).unwrap();
        assert_eq!(console_port, 5554);
        assert_eq!(console_port.to_emulator_serial(), "emulator-5554");

        let zero_adb = AdbPort::new(0);
        assert!(ConsolePort::from_adb_port(zero_adb).is_none());
    }

    #[test]
    fn test_server_version() {
        assert_eq!(SERVER_VERSION, ADB_SERVER_VERSION);
        assert_eq!(SERVER_VERSION, 41);
        assert_eq!(41, SERVER_VERSION);
        assert_eq!(SERVER_VERSION.to_string(), "41");
    }
}
