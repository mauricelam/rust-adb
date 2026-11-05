//! Device tracker format and serialization for smart-socket `track-devices` subscriptions.
//!
//! Ported from `original/transport.cpp` (`list_transports`, `transportListToText`, `transportListToProto`).

use std::cmp::Ordering;
use std::fmt::Write as _;
use std::sync::Arc;

use crate::registry::DeviceEntry;
use crate::state::ConnectionState;
use crate::transport_type::TransportType;

/// Output formats supported by `host:track-devices*` services.
///
/// Ported from `enum TrackerOutputType` in `original/transport.h`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TrackerFormat {
    /// Short text format: `<serial>\t<state>\n`. (host:track-devices)
    ShortText,
    /// Long text format: `<serial-aligned> <state> product:... transport_id:...`. (host:track-devices-l)
    LongText,
    /// Binary protobuf format of `adb.proto.Devices`. (host:track-devices-proto-binary)
    ProtoBinary,
    /// Text protobuf format of `adb.proto.Devices`. (host:track-devices-proto-text)
    ProtoText,
}

/// Upstream protobuf wire ConnectionState enum values from `adb_host.proto`.
fn proto_connection_state(state: ConnectionState) -> u32 {
    match state {
        ConnectionState::Connecting => 1,
        ConnectionState::Authorizing => 2,
        ConnectionState::Unauthorized => 3,
        ConnectionState::NoPerm => 4,
        ConnectionState::Detached => 5,
        ConnectionState::Offline => 6,
        ConnectionState::Bootloader => 7,
        ConnectionState::Device => 8,
        ConnectionState::Host => 9,
        ConnectionState::Recovery => 10,
        ConnectionState::Sideload => 11,
        ConnectionState::Rescue => 12,
    }
}

/// Upstream protobuf text format connection state string from `adb_host.proto`.
fn proto_connection_state_str(state: ConnectionState) -> &'static str {
    match state {
        ConnectionState::Connecting => "CONNECTING",
        ConnectionState::Authorizing => "AUTHORIZING",
        ConnectionState::Unauthorized => "UNAUTHORIZED",
        ConnectionState::NoPerm => "NOPERMISSION",
        ConnectionState::Detached => "DETACHED",
        ConnectionState::Offline => "OFFLINE",
        ConnectionState::Bootloader => "BOOTLOADER",
        ConnectionState::Device => "DEVICE",
        ConnectionState::Host => "HOST",
        ConnectionState::Recovery => "RECOVERY",
        ConnectionState::Sideload => "SIDELOAD",
        ConnectionState::Rescue => "RESCUE",
    }
}

/// ConnectionType enum values in `adb_host.proto`.
fn proto_connection_type(t: TransportType) -> (u32, &'static str) {
    match t {
        TransportType::Usb => (1, "USB"),
        _ => (2, "SOCKET"),
    }
}

/// Sanitizes property string, replacing newlines with '_' and optionally non-alphanumeric chars.
///
/// Ported from `sanitize` in `original/transport.cpp`:
/// ```cpp
/// static std::string sanitize(std::string str, bool alphanumeric) {
///     auto pred = alphanumeric ? [](const char c) { return !isalnum(c); }
///                              : [](const char c) { return c == '\n'; };
///     std::replace_if(str.begin(), str.end(), pred, '_');
///     return str;
/// }
/// ```
pub fn sanitize(s: &str, alphanumeric: bool) -> String {
    s.chars()
        .map(|c| {
            if alphanumeric {
                if c.is_ascii_alphanumeric() {
                    c
                } else {
                    '_'
                }
            } else if c == '\n' {
                '_'
            } else {
                c
            }
        })
        .collect()
}

/// Sorts devices matching `original/transport.cpp`:
/// ```cpp
/// sorted_transport_list.sort([](atransport*& x, atransport*& y) {
///     if (x->type != y->type) {
///         return x->type < y->type;
///     }
///     return x->serial < y->serial;
/// });
/// ```
/// Note: kTransportUsb = 0, kTransportLocal = 1.
pub fn sort_devices(devices: &mut [Arc<DeviceEntry>]) {
    devices.sort_by(|a, b| {
        let type_rank = |t: TransportType| match t {
            TransportType::Usb => 0,
            TransportType::Local => 1,
            TransportType::Any => 2,
            TransportType::Host => 3,
        };
        match type_rank(a.transport_type).cmp(&type_rank(b.transport_type)) {
            Ordering::Equal => a.serial.cmp(&b.serial),
            other => other,
        }
    });
}

fn append_transport_info(result: &mut String, key: &str, value: Option<&str>, alphanumeric: bool) {
    if let Some(val) = value {
        if !val.is_empty() {
            result.push(' ');
            result.push_str(key);
            result.push_str(&sanitize(val, alphanumeric));
        }
    }
}

fn append_transport(t: &DeviceEntry, result: &mut String, long_listing: bool) {
    let serial = if t.serial.is_empty() {
        "(no serial number)"
    } else {
        &t.serial
    };

    let state_str = if t.state == ConnectionState::NoPerm {
        crate::diagnose::usb_no_permissions_short_help_text()
    } else {
        t.state.as_str().to_string()
    };

    if !long_listing {
        result.push_str(serial);
        result.push('\t');
        result.push_str(&state_str);
    } else {
        // android::base::StringAppendF(result, "%-22s %s", serial.c_str(), to_string(...));
        let _ = write!(result, "{:<22} {}", serial, state_str);
        append_transport_info(result, "", t.devpath.as_deref(), false);
        append_transport_info(result, "product:", t.product.as_deref(), false);
        append_transport_info(result, "model:", t.model.as_deref(), true);
        append_transport_info(result, "device:", t.device.as_deref(), false);

        let _ = write!(result, " transport_id:{}", t.transport_id);
    }
    result.push('\n');
}

/// Encodes a u64 varint into a byte buffer.
fn encode_varint(mut val: u64, buf: &mut Vec<u8>) {
    while val >= 0x80 {
        buf.push(((val & 0x7f) as u8) | 0x80);
        val >>= 7;
    }
    buf.push(val as u8);
}

/// Encodes a protobuf length-delimited field (tag = (field_num << 3) | 2).
fn encode_bytes_field(field_num: u32, data: &[u8], buf: &mut Vec<u8>) {
    encode_varint(((field_num as u64) << 3) | 2, buf);
    encode_varint(data.len() as u64, buf);
    buf.extend_from_slice(data);
}

/// Encodes a protobuf varint field (tag = (field_num << 3) | 0, where wire type 0 = varint).
fn encode_varint_field(field_num: u32, val: u64, buf: &mut Vec<u8>) {
    encode_varint((field_num as u64) << 3, buf);
    encode_varint(val, buf);
}

/// Formats device list according to `TrackerFormat`.
pub fn format_devices_raw(devices: &[Arc<DeviceEntry>], format: TrackerFormat) -> Vec<u8> {
    let mut sorted = devices.to_vec();
    sort_devices(&mut sorted);

    match format {
        TrackerFormat::ShortText | TrackerFormat::LongText => {
            let long = format == TrackerFormat::LongText;
            let mut out = String::new();
            for dev in &sorted {
                append_transport(dev, &mut out, long);
            }
            out.into_bytes()
        }
        TrackerFormat::ProtoText => {
            let mut out = String::new();
            for dev in &sorted {
                out.push_str("device {\n");
                let _ = writeln!(out, "  serial: \"{}\"", dev.serial);
                let _ = writeln!(out, "  state: {}", proto_connection_state_str(dev.state));
                if let Some(ref bp) = dev.devpath {
                    let s = sanitize(bp, false);
                    if !s.is_empty() {
                        let _ = writeln!(out, "  bus_address: \"{}\"", s);
                    }
                }
                if let Some(ref prod) = dev.product {
                    let s = sanitize(prod, false);
                    if !s.is_empty() {
                        let _ = writeln!(out, "  product: \"{}\"", s);
                    }
                }
                if let Some(ref model) = dev.model {
                    let s = sanitize(model, true);
                    if !s.is_empty() {
                        let _ = writeln!(out, "  model: \"{}\"", s);
                    }
                }
                if let Some(ref device) = dev.device {
                    let s = sanitize(device, false);
                    if !s.is_empty() {
                        let _ = writeln!(out, "  device: \"{}\"", s);
                    }
                }
                let (_, conn_type_str) = proto_connection_type(dev.transport_type);
                let _ = writeln!(out, "  connection_type: {}", conn_type_str);
                let _ = writeln!(out, "  transport_id: {}", dev.transport_id);
                out.push_str("}\n");
            }
            out.into_bytes()
        }
        TrackerFormat::ProtoBinary => {
            let mut devices_buf = Vec::new();
            for dev in &sorted {
                let mut dev_buf = Vec::new();

                // 1: serial (string)
                encode_bytes_field(1, dev.serial.as_bytes(), &mut dev_buf);

                // 2: state (ConnectionState varint)
                encode_varint_field(2, proto_connection_state(dev.state) as u64, &mut dev_buf);

                // 3: bus_address (string)
                if let Some(ref devpath) = dev.devpath {
                    let s = sanitize(devpath, false);
                    if !s.is_empty() {
                        encode_bytes_field(3, s.as_bytes(), &mut dev_buf);
                    }
                }

                // 4: product (string)
                if let Some(ref prod) = dev.product {
                    let s = sanitize(prod, false);
                    if !s.is_empty() {
                        encode_bytes_field(4, s.as_bytes(), &mut dev_buf);
                    }
                }

                // 5: model (string)
                if let Some(ref model) = dev.model {
                    let s = sanitize(model, true);
                    if !s.is_empty() {
                        encode_bytes_field(5, s.as_bytes(), &mut dev_buf);
                    }
                }

                // 6: device (string)
                if let Some(ref device) = dev.device {
                    let s = sanitize(device, false);
                    if !s.is_empty() {
                        encode_bytes_field(6, s.as_bytes(), &mut dev_buf);
                    }
                }

                // 7: connection_type (ConnectionType varint)
                let (conn_type_val, _) = proto_connection_type(dev.transport_type);
                encode_varint_field(7, conn_type_val as u64, &mut dev_buf);

                // 10: transport_id (int64 varint)
                encode_varint_field(10, dev.transport_id.get(), &mut dev_buf);

                // Append Device submessage to Devices message (field 1, repeated message)
                encode_bytes_field(1, &dev_buf, &mut devices_buf);
            }
            devices_buf
        }
    }
}

/// Formats device list framed with a 4-hex-digit length prefix (`%04x`), matching upstream ADB smart socket.
///
/// Ported from `device_tracker_send` in `original/transport.cpp`:
/// ```cpp
/// snprintf(buf, sizeof(buf), "%04x", static_cast<int>(string.size()));
/// memcpy(&data[0], buf, 4);
/// memcpy(&data[4], string.data(), string.size());
/// ```
pub fn format_devices_framed(devices: &[Arc<DeviceEntry>], format: TrackerFormat) -> Vec<u8> {
    let payload = format_devices_raw(devices, format);
    let mut framed = Vec::with_capacity(4 + payload.len());
    let len_str = format!("{:04x}", payload.len());
    framed.extend_from_slice(len_str.as_bytes());
    framed.extend_from_slice(&payload);
    framed
}
