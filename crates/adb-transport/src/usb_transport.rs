//! USB transport implementation providing ADB packet framing over USB bulk endpoints.
//!
//! Ported and adapted from upstream C++ ADB USB transport:
//! - `original/client/transport_usb.cpp`
//! - `original/client/usb_libusb_device.cpp`
//! - `original/client/usb_linux.cpp`

use crate::transport::{calculate_checksum, Packet, Transport, TransportError};
use crate::usb::{UsbDeviceHandle, UsbError};
use adb_proto::{Header, MAX_PAYLOAD};
use std::io::{Error as IoError, ErrorKind};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// Transports ADB packets over bulk IN and OUT endpoints using an underlying [`UsbDeviceHandle`].
#[derive(Debug)]
pub struct UsbTransport<H = Box<dyn UsbDeviceHandle>> {
    handle: H,
    bulk_out_address: u8,
    bulk_out_max_packet_size: u16,
    bulk_in_address: u8,
    bulk_in_max_packet_size: u16,
    max_payload: usize,
    skip_checksum: bool,
    failed: Arc<AtomicBool>,
}

impl<H> UsbTransport<H> {
    /// Constructs a new [`UsbTransport`].
    pub fn new(
        handle: H,
        bulk_out_address: u8,
        bulk_out_max_packet_size: u16,
        bulk_in_address: u8,
        bulk_in_max_packet_size: u16,
    ) -> Self {
        Self {
            handle,
            bulk_out_address,
            bulk_out_max_packet_size,
            bulk_in_address,
            bulk_in_max_packet_size,
            max_payload: MAX_PAYLOAD,
            skip_checksum: false,
            failed: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Returns the bulk OUT endpoint address.
    pub fn bulk_out_address(&self) -> u8 {
        self.bulk_out_address
    }

    /// Returns the bulk OUT endpoint maximum packet size.
    pub fn bulk_out_max_packet_size(&self) -> u16 {
        self.bulk_out_max_packet_size
    }

    /// Returns the bulk IN endpoint address.
    pub fn bulk_in_address(&self) -> u8 {
        self.bulk_in_address
    }

    /// Returns the bulk IN endpoint maximum packet size.
    pub fn bulk_in_max_packet_size(&self) -> u16 {
        self.bulk_in_max_packet_size
    }

    /// Sets negotiated maximum payload limit.
    pub fn set_max_payload(&mut self, max_payload: usize) {
        self.max_payload = max_payload;
    }

    /// Configures whether payload checksum calculation and verification should be skipped.
    pub fn set_skip_checksum(&mut self, skip_checksum: bool) {
        self.skip_checksum = skip_checksum;
    }

    /// Returns a reference to the underlying handle.
    pub fn handle(&self) -> &H {
        &self.handle
    }

    /// Returns a mutable reference to the underlying handle.
    pub fn handle_mut(&mut self) -> &mut H {
        &mut self.handle
    }

    /// Unwraps and returns the underlying handle.
    pub fn into_inner(self) -> H {
        self.handle
    }
}

fn usb_err_to_transport_err(err: UsbError) -> TransportError {
    match err {
        UsbError::Disconnected => {
            TransportError::Io(IoError::new(ErrorKind::BrokenPipe, "device disconnected"))
        }
        UsbError::Timeout => {
            TransportError::Io(IoError::new(ErrorKind::TimedOut, "USB transfer timed out"))
        }
        UsbError::Stall => {
            TransportError::Io(IoError::new(ErrorKind::BrokenPipe, "endpoint stalled"))
        }
        UsbError::PermissionDenied => TransportError::Io(IoError::new(
            ErrorKind::PermissionDenied,
            "permission denied on USB device",
        )),
        UsbError::Io(msg) => TransportError::Io(IoError::other(msg)),
        UsbError::Other(msg) => TransportError::Io(IoError::other(msg)),
    }
}

impl<H: UsbDeviceHandle + Send + Sync> Transport for UsbTransport<H> {
    async fn read_packet(&mut self) -> Result<Packet, TransportError> {
        if self.failed.load(Ordering::Relaxed) {
            return Err(TransportError::Io(IoError::new(
                ErrorKind::BrokenPipe,
                "USB transport already disconnected or failed",
            )));
        }

        let res: Result<Packet, TransportError> = async {
            // Read 24-byte packet header
            let mut header_buf = [0u8; Header::SIZE];
            let mut header_read = 0;
            while header_read < Header::SIZE {
                let n = self
                    .handle
                    .bulk_read(self.bulk_in_address, &mut header_buf[header_read..])
                    .await
                    .map_err(usb_err_to_transport_err)?;
                if n == 0 {
                    return Err(TransportError::Io(IoError::new(
                        ErrorKind::UnexpectedEof,
                        "unexpected EOF reading USB packet header",
                    )));
                }
                header_read += n;
            }

            let header = Header::decode(&header_buf)?;

            let data_length = header.data_length as usize;
            if data_length > self.max_payload {
                return Err(TransportError::PayloadTooLarge {
                    max: self.max_payload,
                    actual: data_length,
                });
            }

            let mut payload = vec![0u8; data_length];
            let mut payload_read = 0;
            while payload_read < data_length {
                let n = self
                    .handle
                    .bulk_read(self.bulk_in_address, &mut payload[payload_read..])
                    .await
                    .map_err(usb_err_to_transport_err)?;
                if n == 0 {
                    return Err(TransportError::Io(IoError::new(
                        ErrorKind::UnexpectedEof,
                        "unexpected EOF reading USB packet payload",
                    )));
                }
                payload_read += n;
            }

            if !self.skip_checksum && header.data_crc32 != 0 {
                let actual_checksum = calculate_checksum(&payload);
                if actual_checksum != header.data_crc32 {
                    return Err(TransportError::ChecksumMismatch {
                        expected: header.data_crc32,
                        actual: actual_checksum,
                    });
                }
            }

            Ok(Packet { header, payload })
        }
        .await;

        if res.is_err() {
            self.failed.store(true, Ordering::Relaxed);
        }

        res
    }

    async fn write_packet(&mut self, packet: &Packet) -> Result<(), TransportError> {
        if self.failed.load(Ordering::Relaxed) {
            return Err(TransportError::Io(IoError::new(
                ErrorKind::BrokenPipe,
                "USB transport already disconnected or failed",
            )));
        }

        let res: Result<(), TransportError> = async {
            // Write 24-byte packet header
            let header_buf = packet.header.encode();
            let mut written = 0;
            while written < header_buf.len() {
                let n = self
                    .handle
                    .bulk_write(self.bulk_out_address, &header_buf[written..])
                    .await
                    .map_err(usb_err_to_transport_err)?;
                if n == 0 {
                    return Err(TransportError::Io(IoError::new(
                        ErrorKind::WriteZero,
                        "zero bytes written for USB packet header",
                    )));
                }
                written += n;
            }

            // Write payload in a separate transfer if payload is non-empty
            let payload_len = packet.payload.len();
            if payload_len > 0 {
                let mut p_written = 0;
                while p_written < payload_len {
                    let n = self
                        .handle
                        .bulk_write(self.bulk_out_address, &packet.payload[p_written..])
                        .await
                        .map_err(usb_err_to_transport_err)?;
                    if n == 0 {
                        return Err(TransportError::Io(IoError::new(
                            ErrorKind::WriteZero,
                            "zero bytes written for USB packet payload",
                        )));
                    }
                    p_written += n;
                }

                // If payload length is a non-zero multiple of endpoint's max packet size,
                // send a zero-length packet (ZLP) to terminate the USB bulk transfer.
                // Upstream: if (zero_mask && !(len & zero_mask)) usb_bulk_write(h, _data, 0);
                #[allow(clippy::manual_is_multiple_of)]
                if self.bulk_out_max_packet_size > 0
                    && (payload_len % (self.bulk_out_max_packet_size as usize) == 0)
                {
                    self.handle
                        .bulk_write(self.bulk_out_address, &[])
                        .await
                        .map_err(usb_err_to_transport_err)?;
                }
            }

            Ok(())
        }
        .await;

        if res.is_err() {
            self.failed.store(true, Ordering::Relaxed);
        }

        res
    }
}
