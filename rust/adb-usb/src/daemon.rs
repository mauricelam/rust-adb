use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::sync::{Mutex, Weak};

use adb_transport::{ATransport, BlockingConnection};
use adb_types::{Amessage, Apacket, Block};
use rust_adb_crypto::Key;

pub const USB_FFS_ADB_PATH: &str = "/dev/usb-ffs/adb/";
pub const USB_FFS_ADB_EP0: &str = "/dev/usb-ffs/adb/ep0";
pub const USB_FFS_ADB_OUT: &str = "/dev/usb-ffs/adb/ep1";
pub const USB_FFS_ADB_IN: &str = "/dev/usb-ffs/adb/ep2";

const ADB_CLASS: u8 = 0xff;
const ADB_SUBCLASS: u8 = 0x42;
const ADB_PROTOCOL: u8 = 0x01;

const MAX_PACKET_SIZE_FS: u16 = 64;
const MAX_PACKET_SIZE_HS: u16 = 512;
const MAX_PACKET_SIZE_SS: u16 = 1024;

const FUNCTIONFS_DESCRIPTORS_MAGIC_V2: u32 = 2;
const FUNCTIONFS_STRINGS_MAGIC: u32 = 2;

const FUNCTIONFS_HAS_FS_DESC: u32 = 1;
const FUNCTIONFS_HAS_HS_DESC: u32 = 2;
const FUNCTIONFS_HAS_SS_DESC: u32 = 4;
const FUNCTIONFS_HAS_MS_OS_DESC: u32 = 8;

const USB_DT_INTERFACE: u8 = 0x04;
const USB_DT_ENDPOINT: u8 = 0x05;
const USB_DT_SS_ENDPOINT_COMP: u8 = 0x30;

const USB_DIR_OUT: u8 = 0x00;
const USB_DIR_IN: u8 = 0x80;
const USB_ENDPOINT_XFER_BULK: u8 = 0x02;

#[allow(dead_code)]
#[repr(packed)]
#[derive(Clone, Copy)]
struct UsbInterfaceDescriptor {
    b_length: u8,
    b_descriptor_type: u8,
    b_interface_number: u8,
    b_alternate_setting: u8,
    b_num_endpoints: u8,
    b_interface_class: u8,
    b_interface_sub_class: u8,
    b_interface_protocol: u8,
    i_interface: u8,
}

#[allow(dead_code)]
#[repr(packed)]
#[derive(Clone, Copy)]
struct UsbEndpointDescriptorNoAudio {
    b_length: u8,
    b_descriptor_type: u8,
    b_endpoint_address: u8,
    bm_attributes: u8,
    w_max_packet_size: u16,
    b_interval: u8,
}

#[allow(dead_code)]
#[repr(packed)]
#[derive(Clone, Copy)]
struct UsbSsEpCompDescriptor {
    b_length: u8,
    b_descriptor_type: u8,
    b_max_burst: u8,
    bm_attributes: u8,
    w_bytes_per_interval: u16,
}

#[allow(dead_code)]
#[repr(packed)]
#[derive(Clone, Copy)]
struct FuncDesc {
    intf: UsbInterfaceDescriptor,
    source: UsbEndpointDescriptorNoAudio,
    sink: UsbEndpointDescriptorNoAudio,
}

#[allow(dead_code)]
#[repr(packed)]
#[derive(Clone, Copy)]
struct SsFuncDesc {
    intf: UsbInterfaceDescriptor,
    source: UsbEndpointDescriptorNoAudio,
    source_comp: UsbSsEpCompDescriptor,
    sink: UsbEndpointDescriptorNoAudio,
    sink_comp: UsbSsEpCompDescriptor,
}

#[allow(dead_code)]
#[repr(packed)]
#[derive(Clone, Copy)]
struct UsbOsDescHeader {
    interface: u32,
    dw_length: u32,
    bcd_version: u32,
    w_index: u32,
    b_count: u32,
    reserved: u32,
}

#[allow(dead_code)]
#[repr(packed)]
#[derive(Clone, Copy)]
struct UsbExtCompatDesc {
    b_first_interface_number: u8,
    reserved1: u32,
    compatible_id: [u8; 8],
    sub_compatible_id: [u8; 8],
    reserved2: [u8; 6],
}

#[allow(dead_code)]
#[repr(packed)]
#[derive(Clone, Copy)]
struct UsbOsDescPropHeader {
    interface: u32,
    dw_length: u32,
    bcd_version: u32,
    w_index: u32,
    w_count: u16,
}

#[allow(dead_code)]
#[repr(packed)]
#[derive(Clone, Copy)]
struct UsbOsDescExtProp {
    dw_size: u32,
    dw_property_data_type: u32,
    w_property_name_length: u16,
    b_property_name: [u8; 20],
    dw_property_data_length: u32,
    b_property: [u8; 39],
}

#[allow(dead_code)]
#[repr(packed)]
#[derive(Clone, Copy)]
struct DescV2 {
    magic: u32,
    length: u32,
    flags: u32,
    fs_count: u32,
    hs_count: u32,
    ss_count: u32,
    os_count: u32,
    fs_descs: FuncDesc,
    hs_descs: FuncDesc,
    ss_descs: SsFuncDesc,
    os_header: UsbOsDescHeader,
    os_desc: UsbExtCompatDesc,
    os_prop_header: UsbOsDescPropHeader,
    os_prop_values: UsbOsDescExtProp,
}

#[allow(dead_code)]
#[repr(packed)]
struct UsbFunctionfsStringsHead {
    magic: u32,
    length: u32,
    str_count: u32,
    lang_count: u32,
}

#[allow(dead_code)]
#[repr(packed)]
struct UsbStrings {
    header: UsbFunctionfsStringsHead,
    lang_code: u16,
    str_interface: [u8; 14], // "ADB Interface" + \0
}

fn create_fs_hs_desc(max_packet_size: u16) -> FuncDesc {
    FuncDesc {
        intf: UsbInterfaceDescriptor {
            b_length: std::mem::size_of::<UsbInterfaceDescriptor>() as u8,
            b_descriptor_type: USB_DT_INTERFACE,
            b_interface_number: 0,
            b_alternate_setting: 0,
            b_num_endpoints: 2,
            b_interface_class: ADB_CLASS,
            b_interface_sub_class: ADB_SUBCLASS,
            b_interface_protocol: ADB_PROTOCOL,
            i_interface: 1,
        },
        source: UsbEndpointDescriptorNoAudio {
            b_length: std::mem::size_of::<UsbEndpointDescriptorNoAudio>() as u8,
            b_descriptor_type: USB_DT_ENDPOINT,
            b_endpoint_address: 1 | USB_DIR_OUT,
            bm_attributes: USB_ENDPOINT_XFER_BULK,
            w_max_packet_size: max_packet_size.to_le(),
            b_interval: 0,
        },
        sink: UsbEndpointDescriptorNoAudio {
            b_length: std::mem::size_of::<UsbEndpointDescriptorNoAudio>() as u8,
            b_descriptor_type: USB_DT_ENDPOINT,
            b_endpoint_address: 2 | USB_DIR_IN,
            bm_attributes: USB_ENDPOINT_XFER_BULK,
            w_max_packet_size: max_packet_size.to_le(),
            b_interval: 0,
        },
    }
}

pub fn init_functionfs() -> std::io::Result<(File, File, File)> {
    let mut ep0 = OpenOptions::new().read(true).write(true).open(USB_FFS_ADB_EP0)?;

    let desc = DescV2 {
        magic: FUNCTIONFS_DESCRIPTORS_MAGIC_V2.to_le(),
        length: (std::mem::size_of::<DescV2>() as u32).to_le(),
        flags: (FUNCTIONFS_HAS_FS_DESC | FUNCTIONFS_HAS_HS_DESC | FUNCTIONFS_HAS_SS_DESC | FUNCTIONFS_HAS_MS_OS_DESC).to_le(),
        fs_count: 3_u32.to_le(),
        hs_count: 3_u32.to_le(),
        ss_count: 5_u32.to_le(),
        os_count: 2_u32.to_le(),
        fs_descs: create_fs_hs_desc(MAX_PACKET_SIZE_FS),
        hs_descs: create_fs_hs_desc(MAX_PACKET_SIZE_HS),
        ss_descs: SsFuncDesc {
            intf: UsbInterfaceDescriptor {
                b_length: std::mem::size_of::<UsbInterfaceDescriptor>() as u8,
                b_descriptor_type: USB_DT_INTERFACE,
                b_interface_number: 0,
                b_alternate_setting: 0,
                b_num_endpoints: 2,
                b_interface_class: ADB_CLASS,
                b_interface_sub_class: ADB_SUBCLASS,
                b_interface_protocol: ADB_PROTOCOL,
                i_interface: 1,
            },
            source: UsbEndpointDescriptorNoAudio {
                b_length: std::mem::size_of::<UsbEndpointDescriptorNoAudio>() as u8,
                b_descriptor_type: USB_DT_ENDPOINT,
                b_endpoint_address: 1 | USB_DIR_OUT,
                bm_attributes: USB_ENDPOINT_XFER_BULK,
                w_max_packet_size: MAX_PACKET_SIZE_SS.to_le(),
                b_interval: 0,
            },
            source_comp: UsbSsEpCompDescriptor {
                b_length: std::mem::size_of::<UsbSsEpCompDescriptor>() as u8,
                b_descriptor_type: USB_DT_SS_ENDPOINT_COMP,
                b_max_burst: 4,
                bm_attributes: 0,
                w_bytes_per_interval: 0,
            },
            sink: UsbEndpointDescriptorNoAudio {
                b_length: std::mem::size_of::<UsbEndpointDescriptorNoAudio>() as u8,
                b_descriptor_type: USB_DT_ENDPOINT,
                b_endpoint_address: 2 | USB_DIR_IN,
                bm_attributes: USB_ENDPOINT_XFER_BULK,
                w_max_packet_size: MAX_PACKET_SIZE_SS.to_le(),
                b_interval: 0,
            },
            sink_comp: UsbSsEpCompDescriptor {
                b_length: std::mem::size_of::<UsbSsEpCompDescriptor>() as u8,
                b_descriptor_type: USB_DT_SS_ENDPOINT_COMP,
                b_max_burst: 4,
                bm_attributes: 0,
                w_bytes_per_interval: 0,
            },
        },
        os_header: UsbOsDescHeader {
            interface: 0,
            dw_length: (std::mem::size_of::<UsbOsDescHeader>() as u32 + std::mem::size_of::<UsbExtCompatDesc>() as u32).to_le(),
            bcd_version: 1_u32.to_le(),
            w_index: 4_u32.to_le(),
            b_count: 1_u32.to_le(),
            reserved: 0,
        },
        os_desc: UsbExtCompatDesc {
            b_first_interface_number: 0,
            reserved1: 1_u32.to_le(),
            compatible_id: *b"WINUSB\0\0",
            sub_compatible_id: [0; 8],
            reserved2: [0; 6],
        },
        os_prop_header: UsbOsDescPropHeader {
            interface: 0,
            dw_length: (std::mem::size_of::<UsbOsDescPropHeader>() as u32 + std::mem::size_of::<UsbOsDescExtProp>() as u32).to_le(),
            bcd_version: 1_u32.to_le(),
            w_index: 5_u32.to_le(),
            w_count: 1_u16.to_le(),
        },
        os_prop_values: UsbOsDescExtProp {
            dw_size: std::mem::size_of::<UsbOsDescExtProp>() as u32,
            dw_property_data_type: 1_u32.to_le(), // USB_EXT_PROP_UNICODE
            w_property_name_length: 20_u16.to_le(),
            b_property_name: *b"DeviceInterfaceGUID\0",
            dw_property_data_length: 39_u32.to_le(),
            b_property: *b"{F72FE0D4-CBCB-407D-8814-9ED673D0DD6B}\0",
        },
    };

    let desc_bytes = unsafe {
        std::slice::from_raw_parts(&desc as *const DescV2 as *const u8, std::mem::size_of::<DescV2>())
    };
    ep0.write_all(desc_bytes)?;

    let strings = UsbStrings {
        header: UsbFunctionfsStringsHead {
            magic: FUNCTIONFS_STRINGS_MAGIC.to_le(),
            length: (std::mem::size_of::<UsbStrings>() as u32).to_le(),
            str_count: 1_u32.to_le(),
            lang_count: 1_u32.to_le(),
        },
        lang_code: 0x0409_u16.to_le(),
        str_interface: *b"ADB Interface\0",
    };

    let strings_bytes = unsafe {
        std::slice::from_raw_parts(&strings as *const UsbStrings as *const u8, std::mem::size_of::<UsbStrings>())
    };
    ep0.write_all(strings_bytes)?;

    let bulk_out = OpenOptions::new().read(true).open(USB_FFS_ADB_OUT)?;
    let bulk_in = OpenOptions::new().write(true).open(USB_FFS_ADB_IN)?;

    Ok((ep0, bulk_out, bulk_in))
}

pub struct DaemonUsbConnection {
    #[allow(dead_code)]
    ep0: Mutex<File>,
    bulk_out: Mutex<File>,
    bulk_in: Mutex<File>,
    transport: Mutex<Option<Weak<ATransport>>>,
}

impl DaemonUsbConnection {
    pub fn new() -> std::io::Result<Self> {
        let (ep0, bulk_out, bulk_in) = init_functionfs()?;
        Ok(Self {
            ep0: Mutex::new(ep0),
            bulk_out: Mutex::new(bulk_out),
            bulk_in: Mutex::new(bulk_in),
            transport: Mutex::new(None),
        })
    }
}

impl BlockingConnection for DaemonUsbConnection {
    fn read(&self) -> std::io::Result<Apacket> {
        let mut header_buf = [0u8; std::mem::size_of::<Amessage>()];
        let mut bulk_out = self.bulk_out.lock().unwrap();
        bulk_out.read_exact(&mut header_buf)?;

        let msg: Amessage = unsafe { std::ptr::read_unaligned(header_buf.as_ptr() as *const Amessage) };
        let mut payload = Block::new(msg.data_length as usize);
        if msg.data_length > 0 {
            bulk_out.read_exact(payload.get_mut())?;
        }

        Ok(Apacket { msg, payload })
    }

    fn write(&self, packet: &Apacket) -> std::io::Result<()> {
        let header_bytes: [u8; std::mem::size_of::<Amessage>()] = unsafe { std::mem::transmute(packet.msg) };
        let mut bulk_in = self.bulk_in.lock().unwrap();
        bulk_in.write_all(&header_bytes)?;

        if !packet.payload.is_empty() {
            bulk_in.write_all(packet.payload.get_ref())?;
        }

        Ok(())
    }

    fn do_tls_handshake(&self, _key: &Key, _auth_key: Option<&mut String>) -> bool {
        // TLS over USB is not yet supported.
        false
    }

    fn close(&self) {
        // Files are closed on drop.
    }

    fn reset(&self) {
        // Not much to reset for FunctionFS in this way.
    }
}

impl adb_transport::Connection for DaemonUsbConnection {
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
