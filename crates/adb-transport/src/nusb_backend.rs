//! Real USB backend implementation using `nusb`.
//!
//! Enabled when the `usb` feature is active on `adb-transport`.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::Mutex;

use nusb::descriptors::TransferType as NusbTransferType;
use nusb::transfer::Direction as NusbDirection;
use nusb::MaybeFuture;

use crate::usb::{
    find_adb_interface, EndpointDirection, EndpointType, UsbBackend, UsbDeviceHandle,
    UsbDeviceInfo, UsbEndpointInfo, UsbError, UsbInterfaceInfo,
};

/// Translates `nusb::Error` to `UsbError`.
fn map_nusb_error(err: nusb::Error) -> UsbError {
    match err.kind() {
        nusb::ErrorKind::PermissionDenied => UsbError::PermissionDenied,
        nusb::ErrorKind::Disconnected | nusb::ErrorKind::NotFound => UsbError::Disconnected,
        _ => UsbError::Io(err.to_string()),
    }
}

/// Translates `std::io::Error` to `UsbError`.
fn map_io_error(err: std::io::Error) -> UsbError {
    match err.kind() {
        std::io::ErrorKind::PermissionDenied => UsbError::PermissionDenied,
        std::io::ErrorKind::NotFound | std::io::ErrorKind::UnexpectedEof => UsbError::Disconnected,
        std::io::ErrorKind::TimedOut => UsbError::Timeout,
        _ => UsbError::Io(err.to_string()),
    }
}

/// Real USB backend driven by `nusb`.
#[derive(Debug, Default, Clone)]
pub struct NusbBackend;

impl NusbBackend {
    /// Creates a new `NusbBackend`.
    pub fn new() -> Self {
        Self
    }
}

/// Device handle wrapping an active `nusb::Interface`.
#[derive(Debug)]
pub struct NusbDeviceHandle {
    interface: Arc<Mutex<nusb::Interface>>,
}

impl UsbDeviceHandle for NusbDeviceHandle {
    fn bulk_read<'a>(
        &'a self,
        endpoint: u8,
        buf: &'a mut [u8],
    ) -> Pin<Box<dyn Future<Output = Result<usize, UsbError>> + Send + 'a>> {
        Box::pin(async move {
            let iface = self.interface.lock().await;
            let mut reader = iface
                .endpoint::<nusb::transfer::Bulk, nusb::transfer::In>(endpoint)
                .map_err(map_nusb_error)?
                .reader(buf.len());

            use std::io::Read;
            let len = buf.len();
            let mut temp = vec![0u8; len];
            let n = tokio::task::spawn_blocking(move || {
                let n = reader.read(&mut temp).map_err(map_io_error)?;
                temp.truncate(n);
                Ok::<_, UsbError>(temp)
            })
            .await
            .map_err(|e| UsbError::Io(e.to_string()))??;
            buf[..n.len()].copy_from_slice(&n);
            Ok(n.len())
        })
    }

    fn bulk_write<'a>(
        &'a self,
        endpoint: u8,
        data: &'a [u8],
    ) -> Pin<Box<dyn Future<Output = Result<usize, UsbError>> + Send + 'a>> {
        Box::pin(async move {
            let iface = self.interface.lock().await;
            let mut writer = iface
                .endpoint::<nusb::transfer::Bulk, nusb::transfer::Out>(endpoint)
                .map_err(map_nusb_error)?
                .writer(data.len());

            use std::io::Write;
            let data_vec = data.to_vec();
            tokio::task::spawn_blocking(move || {
                writer.write_all(&data_vec).map_err(map_io_error)?;
                writer.flush().map_err(map_io_error)?;
                Ok(data_vec.len())
            })
            .await
            .map_err(|e| UsbError::Io(e.to_string()))?
        })
    }

    fn clear_halt<'a>(
        &'a self,
        endpoint: u8,
    ) -> Pin<Box<dyn Future<Output = Result<(), UsbError>> + Send + 'a>> {
        Box::pin(async move {
            let iface = self.interface.lock().await;
            use nusb::transfer::{ControlOut, ControlType, Recipient};
            let control = ControlOut {
                control_type: ControlType::Standard,
                recipient: Recipient::Endpoint,
                request: 0x01, // CLEAR_FEATURE
                value: 0x00,   // ENDPOINT_HALT
                index: endpoint as u16,
                data: &[],
            };
            let _ = iface
                .control_out(control, std::time::Duration::from_millis(1000))
                .wait();
            Ok(())
        })
    }

    fn close<'a>(&'a self) -> Pin<Box<dyn Future<Output = Result<(), UsbError>> + Send + 'a>> {
        Box::pin(async move { Ok(()) })
    }
}

impl UsbBackend for NusbBackend {
    fn list_devices<'a>(
        &'a self,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<UsbDeviceInfo>, UsbError>> + Send + 'a>> {
        Box::pin(async move {
            let devs: Vec<nusb::DeviceInfo> = nusb::list_devices()
                .wait()
                .map_err(map_nusb_error)?
                .collect();

            let mut result = Vec::new();

            for dev in devs {
                let bus_number = dev.busnum();
                let device_address = dev.device_address();

                let port_chain = dev.port_chain();
                let device_path = if !port_chain.is_empty() {
                    let mut path = format!("usb:{bus_number}-{}", port_chain[0]);
                    for p in &port_chain[1..] {
                        path.push_str(&format!(".{p}"));
                    }
                    path
                } else {
                    format!("usb:{bus_number}-{device_address}")
                };

                let serial = dev.serial_number().map(|s| s.to_string());

                let mut interfaces = Vec::new();
                if let Ok(opened) = dev.open().wait() {
                    if let Ok(config) = opened.active_configuration() {
                        for alt in config.interface_alt_settings() {
                            let mut endpoints = Vec::new();
                            for ep in alt.endpoints() {
                                let dir = match ep.direction() {
                                    NusbDirection::In => EndpointDirection::In,
                                    NusbDirection::Out => EndpointDirection::Out,
                                };
                                let transfer_type = match ep.transfer_type() {
                                    NusbTransferType::Bulk => EndpointType::Bulk,
                                    NusbTransferType::Control => EndpointType::Control,
                                    NusbTransferType::Interrupt => EndpointType::Interrupt,
                                    NusbTransferType::Isochronous => EndpointType::Isochronous,
                                };
                                endpoints.push(UsbEndpointInfo::new(
                                    ep.address(),
                                    dir,
                                    transfer_type,
                                    ep.max_packet_size() as u16,
                                ));
                            }

                            interfaces.push(UsbInterfaceInfo {
                                interface_number: alt.interface_number(),
                                class: alt.class(),
                                subclass: alt.subclass(),
                                protocol: alt.protocol(),
                                endpoints,
                            });
                        }
                    }
                }

                result.push(UsbDeviceInfo {
                    bus_number,
                    device_address,
                    device_path,
                    vendor_id: dev.vendor_id(),
                    product_id: dev.product_id(),
                    serial,
                    interfaces,
                });
            }

            Ok(result)
        })
    }

    fn open<'a>(
        &'a self,
        device: &'a UsbDeviceInfo,
    ) -> Pin<Box<dyn Future<Output = Result<Box<dyn UsbDeviceHandle>, UsbError>> + Send + 'a>> {
        Box::pin(async move {
            let match_res = find_adb_interface(&device.interfaces)
                .ok_or_else(|| UsbError::Other("device has no ADB interface".to_string()))?;

            let devs: Vec<nusb::DeviceInfo> = nusb::list_devices()
                .wait()
                .map_err(map_nusb_error)?
                .collect();

            let dev_info = devs
                .into_iter()
                .find(|d| {
                    d.busnum() == device.bus_number && d.device_address() == device.device_address
                })
                .ok_or(UsbError::Disconnected)?;

            let opened = dev_info.open().wait().map_err(map_nusb_error)?;

            #[cfg(target_os = "linux")]
            let iface = opened
                .detach_and_claim_interface(match_res.interface_number)
                .wait()
                .map_err(map_nusb_error)?;

            #[cfg(not(target_os = "linux"))]
            let iface = opened
                .claim_interface(match_res.interface_number)
                .wait()
                .map_err(map_nusb_error)?;

            Ok(Box::new(NusbDeviceHandle {
                interface: Arc::new(Mutex::new(iface)),
            }) as Box<dyn UsbDeviceHandle>)
        })
    }
}
