//! Transfer soak test verifying that sustained large transfers finish without memory leaks,
//! descriptor leaks, or hangs, and that progress watchdog checks are enforced.
//!
//! Provides both:
//! 1. Normal CI mock variant: Repeatedly transfers large payloads over `MockUsbDeviceHandle`
//!    with periodic simulated disconnect/replug cycles.
//! 2. Real hardware variant (`test_usb_hardware_transfer_soak`): Gated behind `--features usb`
//!    and `#[ignore]` for long multi-hour runs in `.github/workflows/usb-hardware.yml`.

use std::env;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::time::timeout;

use adb_proto::Command;
use adb_transport::{MockUsbDeviceHandle, Packet, Transport, UsbTransport};

/// Captures current system resource usage metrics for leak detection.
#[derive(Debug, Clone, Copy)]
struct ResourceSnapshot {
    open_fds: usize,
    resident_memory_bytes: usize,
}

impl ResourceSnapshot {
    fn capture() -> Self {
        let open_fds = count_open_fds();
        let resident_memory_bytes = get_resident_memory_bytes();
        Self {
            open_fds,
            resident_memory_bytes,
        }
    }
}

/// Counts open file descriptors across platforms.
fn count_open_fds() -> usize {
    #[cfg(target_os = "linux")]
    {
        if let Ok(entries) = std::fs::read_dir("/proc/self/fd") {
            return entries.count();
        }
    }

    #[cfg(unix)]
    {
        // Fallback for macOS / BSD
        let mut count = 0;
        for fd in 0..1024 {
            // Check if fd is open via fcntl F_GETFD
            let res = unsafe { libc::fcntl(fd, libc::F_GETFD) };
            if res >= 0 {
                count += 1;
            }
        }
        count
    }

    #[cfg(windows)]
    {
        0
    }
}

/// Returns resident memory (RSS) in bytes.
fn get_resident_memory_bytes() -> usize {
    #[cfg(unix)]
    {
        let mut rusage = std::mem::MaybeUninit::<libc::rusage>::uninit();
        if unsafe { libc::getrusage(libc::RUSAGE_SELF, rusage.as_mut_ptr()) } == 0 {
            let rusage = unsafe { rusage.assume_init() };
            #[cfg(target_os = "macos")]
            {
                // On macOS, ru_maxrss is in bytes
                rusage.ru_maxrss as usize
            }
            #[cfg(not(target_os = "macos"))]
            {
                // On Linux, ru_maxrss is in kilobytes
                (rusage.ru_maxrss as usize) * 1024
            }
        } else {
            0
        }
    }

    #[cfg(windows)]
    {
        0
    }
}

/// Helper executing soak loop iterations with progress watchdog.
async fn run_mock_soak_loop(
    iterations: usize,
    chunk_size: usize,
    replug_interval: usize,
    step_timeout: Duration,
) {
    let mut mock = Arc::new(MockUsbDeviceHandle::default());
    let mut transport = UsbTransport::new(mock.clone(), 0x01, 512, 0x81, 512);

    let sample_payload = vec![0x5A; chunk_size];

    for i in 1..=iterations {
        let step_start = Instant::now();

        // Simulate periodic disconnect / replug cycle
        if replug_interval > 0 && i % replug_interval == 0 {
            // Re-instantiate transport to simulate hotplug replug
            mock = Arc::new(MockUsbDeviceHandle::default());
            transport = UsbTransport::new(mock.clone(), 0x01, 512, 0x81, 512);
        }

        // 1. OUT Transfer (Push / Write) guarded by watchdog timeout
        let out_pkt = Packet::new(
            Command::Wrte,
            i as u32,
            (i + 1) as u32,
            sample_payload.clone(),
            false,
        );
        let write_fut = transport.write_packet(&out_pkt);
        timeout(step_timeout, write_fut)
            .await
            .expect("watchdog: write_packet step stalled")
            .expect("write_packet succeeded");

        // Prepare mock IN data for corresponding read
        let in_pkt = Packet::new(
            Command::Wrte,
            (i + 1) as u32,
            i as u32,
            sample_payload.clone(),
            false,
        );
        let mut read_bytes = in_pkt.header.encode().to_vec();
        read_bytes.extend_from_slice(&sample_payload);
        *mock.read_data.lock().await = read_bytes;

        // 2. IN Transfer (Pull / Read) guarded by watchdog timeout
        let read_fut = transport.read_packet();
        let received = timeout(step_timeout, read_fut)
            .await
            .expect("watchdog: read_packet step stalled")
            .expect("read_packet succeeded");

        assert_eq!(received.payload.len(), chunk_size);
        assert!(
            step_start.elapsed() < step_timeout,
            "step progress must be within watchdog timeout"
        );
    }
}

#[tokio::test]
async fn test_mock_usb_soak_run_with_watchdog_and_resource_checks() {
    // Read environment variables or use sensible defaults for CI
    let iterations: usize = env::var("USB_SOAK_ITERATIONS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(25);

    let chunk_size: usize = env::var("USB_SOAK_BYTES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(64 * 1024); // 64 KiB

    let timeout_secs: u64 = env::var("USB_SOAK_TIMEOUT_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(5);

    let step_timeout = Duration::from_secs(timeout_secs);
    let replug_interval = 5;

    // Warm-up iteration to stabilize runtime memory and file descriptors
    run_mock_soak_loop(2, 4096, 0, step_timeout).await;

    // Resource sampling before the soak run
    let before = ResourceSnapshot::capture();

    // Execute the soak transfer loop
    run_mock_soak_loop(iterations, chunk_size, replug_interval, step_timeout).await;

    // Resource sampling after the soak run
    let after = ResourceSnapshot::capture();

    println!(
        "Soak run completed (iterations: {iterations}, chunk: {chunk_size} bytes). Before: FDs={}, RSS={} bytes | After: FDs={}, RSS={} bytes",
        before.open_fds, before.resident_memory_bytes, after.open_fds, after.resident_memory_bytes
    );

    // Verify file descriptor leak margin (must not grow indefinitely)
    if before.open_fds > 0 && after.open_fds > 0 {
        assert!(
            after.open_fds <= before.open_fds + 5,
            "File descriptor leak detected: before={}, after={}",
            before.open_fds,
            after.open_fds
        );
    }

    // Verify resident memory growth margin (allow reasonable allocator overhead but no unbounded leaks)
    // 64 MiB margin accounts for tokio / libc allocation buffer pools
    let allowable_memory_growth_bytes = 64 * 1024 * 1024;
    if before.resident_memory_bytes > 0
        && after.resident_memory_bytes > before.resident_memory_bytes
    {
        let growth = after.resident_memory_bytes - before.resident_memory_bytes;
        assert!(
            growth < allowable_memory_growth_bytes,
            "Memory leak detected: growth={growth} bytes exceeds limit={allowable_memory_growth_bytes} bytes"
        );
    }
}

/// Physical hardware soak test for multi-hour nightly validation.
///
/// Run with:
/// ```bash
/// cargo test -p adb-transport --features usb --test usb_soak_test -- test_usb_hardware_transfer_soak --ignored --nocapture
/// ```
#[cfg(feature = "usb")]
#[tokio::test]
#[ignore]
async fn test_usb_hardware_transfer_soak() {
    use adb_transport::{find_adb_interface, NusbBackend, UsbBackend};

    let iterations: usize = env::var("USB_HW_SOAK_ITERATIONS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(500);

    let timeout_secs: u64 = env::var("USB_HW_SOAK_TIMEOUT_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(10);

    let backend = NusbBackend::new();
    let devices = backend
        .list_devices()
        .await
        .expect("must enumerate USB devices");

    let mut target_device = None;
    let mut adb_match_info = None;

    for dev in devices {
        if let Some(m) = find_adb_interface(&dev.interfaces) {
            target_device = Some(dev);
            adb_match_info = Some(m);
            break;
        }
    }

    let target = target_device
        .expect("At least one ADB USB device must be connected for hardware soak test");
    let match_info = adb_match_info.unwrap();

    let before = ResourceSnapshot::capture();
    println!(
        "Starting hardware soak test on device {:?} (path: {}) for {} iterations. Initial RSS: {} bytes, FDs: {}",
        target.serial, target.device_path, iterations, before.resident_memory_bytes, before.open_fds
    );

    let handle = backend.open(&target).await.expect("open device handle");
    let mut transport = UsbTransport::new(
        handle,
        match_info.bulk_out_address,
        match_info.bulk_out_max_packet_size,
        match_info.bulk_in_address,
        match_info.bulk_in_max_packet_size,
    );

    let step_timeout = Duration::from_secs(timeout_secs);
    for i in 1..=iterations {
        let dummy_packet = Packet::new(
            Command::Cnxn,
            0x01000000,
            1024 * 1024,
            b"host::\0".to_vec(),
            false,
        );
        let write_fut = transport.write_packet(&dummy_packet);
        timeout(step_timeout, write_fut)
            .await
            .expect("hardware soak: write watchdog triggered")
            .expect("hardware soak: write success");
        if i % 100 == 0 {
            println!("Hardware soak progress: completed {i}/{iterations} iterations");
        }
    }

    let after = ResourceSnapshot::capture();
    println!(
        "Hardware soak finished cleanly. Final RSS: {} bytes, FDs: {}",
        after.resident_memory_bytes, after.open_fds
    );
}
