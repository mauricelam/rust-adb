use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use rusb::{Device, DeviceDescriptor, DeviceHandle, GlobalContext};
use adb_transport::{ATransport, BlockingConnection};
use adb_types::{Amessage, Apacket, Block};
use rust_adb_crypto::Key;

pub const ADB_CLASS: u8 = 0xff;
pub const ADB_SUBCLASS: u8 = 0x42;
pub const ADB_PROTOCOL: u8 = 0x01;

pub const ADB_DBC_CLASS: u8 = 0xdc;
pub const ADB_DBC_SUBCLASS: u8 = 0x02;

pub fn is_adb_interface(class: u8, subclass: u8, protocol: u8) -> bool {
    (protocol == ADB_PROTOCOL && class == ADB_CLASS && subclass == ADB_SUBCLASS) ||
    (protocol == ADB_PROTOCOL && class == ADB_DBC_CLASS && subclass == ADB_DBC_SUBCLASS)
}

pub struct AdbDeviceInfo {
    pub device: Device<GlobalContext>,
    pub desc: DeviceDescriptor,
    pub interface_num: u8,
    pub read_endpoint: u8,
    pub write_endpoint: u8,
    pub max_packet_size: u16,
}

pub fn find_adb_devices() -> rusb::Result<Vec<AdbDeviceInfo>> {
    let mut devices = Vec::new();
    for device in rusb::devices()?.iter() {
        let device_desc = device.device_descriptor()?;

        let config_desc = match device.active_config_descriptor() {
            Ok(d) => d,
            Err(_) => continue,
        };

        'interfaces: for interface in config_desc.interfaces() {
            for interface_desc in interface.descriptors() {
                if is_adb_interface(
                    interface_desc.class_code(),
                    interface_desc.sub_class_code(),
                    interface_desc.protocol_code(),
                ) {
                    let mut read_endpoint = None;
                    let mut write_endpoint = None;
                    let mut max_packet_size = 0;

                    for endpoint_desc in interface_desc.endpoint_descriptors() {
                        use rusb::TransferType;
                        if endpoint_desc.transfer_type() == TransferType::Bulk {
                            use rusb::Direction;
                            match endpoint_desc.direction() {
                                Direction::In => {
                                    read_endpoint = Some(endpoint_desc.address());
                                }
                                Direction::Out => {
                                    write_endpoint = Some(endpoint_desc.address());
                                    max_packet_size = endpoint_desc.max_packet_size();
                                }
                            }
                        }
                    }

                    if let (Some(read_ep), Some(write_ep)) = (read_endpoint, write_endpoint) {
                        devices.push(AdbDeviceInfo {
                            device: device.clone(),
                            desc: device_desc,
                            interface_num: interface_desc.interface_number(),
                            read_endpoint: read_ep,
                            write_endpoint: write_ep,
                            max_packet_size,
                        });
                        // Found an ADB interface on this device, move to next device.
                        break 'interfaces;
                    }
                }
            }
        }
    }
    Ok(devices)
}

pub struct HostUsbConnection {
    handle: Arc<Mutex<DeviceHandle<GlobalContext>>>,
    info: AdbDeviceInfo,
    transport: Mutex<Option<Weak<ATransport>>>,
}

impl HostUsbConnection {
    pub fn new(info: AdbDeviceInfo) -> rusb::Result<Self> {
        let handle = info.device.open()?;
        handle.claim_interface(info.interface_num)?;

        // Clear halt on endpoints
        let _ = handle.clear_halt(info.read_endpoint);
        let _ = handle.clear_halt(info.write_endpoint);

        Ok(Self {
            handle: Arc::new(Mutex::new(handle)),
            info,
            transport: Mutex::new(None),
        })
    }

    fn usb_read(&self, buf: &mut [u8]) -> std::io::Result<usize> {
        let handle = self.handle.lock().unwrap();
        match handle.read_bulk(self.info.read_endpoint, buf, Duration::from_secs(5)) {
            Ok(n) => Ok(n),
            Err(rusb::Error::Timeout) => Err(std::io::Error::new(std::io::ErrorKind::TimedOut, "USB read timeout")),
            Err(e) => Err(std::io::Error::new(std::io::ErrorKind::Other, format!("USB read error: {}", e))),
        }
    }

    fn usb_write(&self, buf: &[u8]) -> std::io::Result<usize> {
        let handle = self.handle.lock().unwrap();
        match handle.write_bulk(self.info.write_endpoint, buf, Duration::from_secs(5)) {
            Ok(n) => Ok(n),
            Err(rusb::Error::Timeout) => Err(std::io::Error::new(std::io::ErrorKind::TimedOut, "USB write timeout")),
            Err(e) => Err(std::io::Error::new(std::io::ErrorKind::Other, format!("USB write error: {}", e))),
        }
    }
}

impl BlockingConnection for HostUsbConnection {
    fn read(&self) -> std::io::Result<Apacket> {
        let mut header_buf = [0u8; std::mem::size_of::<Amessage>()];
        let n = self.usb_read(&mut header_buf)?;
        if n != header_buf.len() {
            return Err(std::io::Error::new(std::io::ErrorKind::Other, "USB read incomplete header"));
        }

        let msg: Amessage = unsafe { std::ptr::read_unaligned(header_buf.as_ptr() as *const Amessage) };
        let mut payload = Block::new(msg.data_length as usize);
        if msg.data_length > 0 {
            // libusb/rusb might require reading in multiples of max_packet_size to avoid overflow,
            // but usually for bulk it's fine if we provide enough space.
            // However, ADB protocol sends header and payload separately.
            let n = self.usb_read(payload.get_mut())?;
            if n != msg.data_length as usize {
                return Err(std::io::Error::new(std::io::ErrorKind::Other, "USB read incomplete payload"));
            }
        }

        Ok(Apacket { msg, payload })
    }

    fn write(&self, packet: &Apacket) -> std::io::Result<()> {
        let header_bytes: [u8; std::mem::size_of::<Amessage>()] = unsafe { std::mem::transmute(packet.msg) };
        let n = self.usb_write(&header_bytes)?;
        if n != header_bytes.len() {
            return Err(std::io::Error::new(std::io::ErrorKind::Other, "USB write incomplete header"));
        }

        if !packet.payload.is_empty() {
            let data = packet.payload.get_ref();
            let n = self.usb_write(data)?;
            if n != data.len() {
                return Err(std::io::Error::new(std::io::ErrorKind::Other, "USB write incomplete payload"));
            }

            // ZLP logic
            if self.info.max_packet_size > 0 && (data.len() % self.info.max_packet_size as usize == 0) {
                self.usb_write(&[])?;
            }
        }

        Ok(())
    }

    fn do_tls_handshake(&self, _key: &Key, _auth_key: Option<&mut String>) -> bool {
        // TLS over USB is not yet supported in the original C++ either.
        false
    }

    fn close(&self) {
        let handle = self.handle.lock().unwrap();
        let _ = handle.release_interface(self.info.interface_num);
    }

    fn reset(&self) {
        let handle = self.handle.lock().unwrap();
        let _ = handle.reset();
    }
}

impl adb_transport::Connection for HostUsbConnection {
    fn set_transport(&self, transport: Weak<ATransport>) {
        *self.transport.lock().unwrap() = Some(transport);
    }

    fn write(&self, packet: Apacket) -> bool {
        BlockingConnection::write(self, &packet).is_ok()
    }

    fn start(&self, transport: Weak<ATransport>) -> bool {
        self.set_transport(transport);
        true
    }

    fn stop(&self) {
        self.close();
    }

    fn do_tls_handshake(&self, key: &Key, auth_key: Option<&mut String>) -> bool {
        BlockingConnection::do_tls_handshake(self, key, auth_key)
    }

    fn reset(&self) {
        BlockingConnection::reset(self);
    }
}
