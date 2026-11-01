use adb_proto::{Command, FeatureOptions, A_VERSION, MAX_PAYLOAD};
use adb_transport::{
    console_port_for_adb_port, emulator_serial, scan_for_emulators, scan_for_emulators_with_config,
    DiscoveredEmulator, EmulatorScanConfig, Packet, TcpTransport, Transport,
    DEFAULT_ADB_LOCAL_TRANSPORT_MAX_PORT, DEFAULT_ADB_LOCAL_TRANSPORT_PORT,
};
use std::time::Duration;
use tokio::net::TcpListener;

#[tokio::test]
async fn test_tcp_transport_basic_send_receive() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let server_task = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut transport = TcpTransport::from_stream(stream);
        let packet = transport.read_packet().await.unwrap();
        assert_eq!(packet.header.command, Command::Cnxn);

        let reply = Packet::new(
            Command::Cnxn,
            A_VERSION,
            MAX_PAYLOAD as u32,
            b"device:test:features=shell_v2;".to_vec(),
            true,
        );
        transport.write_packet(&reply).await.unwrap();
    });

    let mut client_transport = TcpTransport::connect(addr).await.unwrap();
    assert!(client_transport.local_addr().is_ok());
    assert!(client_transport.peer_addr().is_ok());

    let handshake = perform_cnxn_handshake(&mut client_transport, &FeatureOptions::default())
        .await
        .unwrap();

    assert_eq!(handshake.negotiated_version, A_VERSION);
    server_task.await.unwrap();
}

#[tokio::test]
async fn test_emulator_discovery_mock_server() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();

    let server_task = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut transport = TcpTransport::from_stream(stream);
        let req = transport.read_packet().await.unwrap();
        assert_eq!(req.header.command, Command::Cnxn);

        let reply = Packet::new(
            Command::Cnxn,
            A_VERSION,
            MAX_PAYLOAD as u32,
            b"device:emulator-test:features=shell_v2;".to_vec(),
            true,
        );
        transport.write_packet(&reply).await.unwrap();
    });

    let config = EmulatorScanConfig {
        host: "127.0.0.1".to_string(),
        min_port: port,
        max_port: port,
        step: 2,
        connect_timeout: Duration::from_millis(500),
    };

    let discovered = scan_for_emulators_with_config(&config).await;
    assert_eq!(discovered.len(), 1);

    let DiscoveredEmulator {
        serial,
        adb_port,
        console_port,
        mut transport,
    } = discovered.into_iter().next().unwrap();

    assert_eq!(adb_port, port);
    assert_eq!(console_port, port - 1);
    assert_eq!(serial, format!("emulator-{}", port - 1));

    let handshake = perform_cnxn_handshake(&mut transport, &FeatureOptions::default())
        .await
        .unwrap();
    assert_eq!(handshake.negotiated_version, A_VERSION);

    server_task.await.unwrap();
}

#[tokio::test]
async fn test_unresponsive_port_scan_times_out_gracefully() {
    let config = EmulatorScanConfig {
        host: "127.0.0.1".to_string(),
        min_port: 59991,
        max_port: 59995,
        step: 2,
        connect_timeout: Duration::from_millis(100),
    };

    let start = std::time::Instant::now();
    let discovered = scan_for_emulators_with_config(&config).await;
    let elapsed = start.elapsed();

    assert!(discovered.is_empty());
    assert!(elapsed < Duration::from_secs(2));
}

#[tokio::test]
async fn test_scan_constants_and_helpers() {
    assert_eq!(DEFAULT_ADB_LOCAL_TRANSPORT_PORT, 5555);
    assert_eq!(DEFAULT_ADB_LOCAL_TRANSPORT_MAX_PORT, 5585);
    assert_eq!(console_port_for_adb_port(5555), 5554);
    assert_eq!(emulator_serial(5554), "emulator-5554");

    // Test default scan config
    let default_config = EmulatorScanConfig::default();
    assert_eq!(default_config.host, "127.0.0.1");
    assert_eq!(default_config.min_port, DEFAULT_ADB_LOCAL_TRANSPORT_PORT);
    assert_eq!(
        default_config.max_port,
        DEFAULT_ADB_LOCAL_TRANSPORT_MAX_PORT
    );
    assert_eq!(default_config.step, 2);

    // Empty scan when min_port > max_port
    let empty_config = EmulatorScanConfig {
        min_port: 5560,
        max_port: 5550,
        ..default_config
    };
    assert!(scan_for_emulators_with_config(&empty_config)
        .await
        .is_empty());
}

#[tokio::test]
async fn test_scan_for_emulators_default_call() {
    // Calling scan_for_emulators() should run without panic
    let _ = scan_for_emulators().await;
}

fn perform_cnxn_handshake<'a, T: Transport + ?Sized>(
    transport: &'a mut T,
    options: &'a FeatureOptions,
) -> impl std::future::Future<
    Output = Result<adb_transport::HandshakeResult, adb_transport::TransportError>,
> + 'a {
    adb_transport::perform_cnxn_handshake(transport, options)
}
