use adb_proto::{Command, Feature, FeatureOptions, Header, A_VERSION, MAX_PAYLOAD};
use adb_transport::{
    perform_cnxn_handshake, AsyncStreamTransport, Packet, Transport, TransportError,
};
use tokio::io::duplex;

#[tokio::test]
async fn test_cnxn_handshake_android10_12_14_15() {
    let android_versions = [
        (
            "Android 10",
            "device:12345678:ro.product.name=aosp_arm64;ro.product.model=Pixel_3;ro.product.device=blueline;features=shell_v2,cmd,stat_v2,ls_v2,fixed_push_mkdir,apex;",
            vec![
                Feature::Shell2,
                Feature::Cmd,
                Feature::Stat2,
                Feature::Ls2,
                Feature::FixedPushMkdir,
                Feature::Apex,
            ],
        ),
        (
            "Android 12",
            "device:12345678:ro.product.name=sdk_gphone64_x86_64;ro.product.model=Pixel_5;ro.product.device=redfin;features=shell_v2,cmd,stat_v2,ls_v2,fixed_push_mkdir,apex,abb,abb_exec;",
            vec![
                Feature::Shell2,
                Feature::Cmd,
                Feature::Stat2,
                Feature::Ls2,
                Feature::FixedPushMkdir,
                Feature::Apex,
                Feature::Abb,
                Feature::AbbExec,
            ],
        ),
        (
            "Android 14",
            "device:12345678:ro.product.name=sdk_gphone64_arm64;ro.product.model=Pixel_7;ro.product.device=panther;features=shell_v2,cmd,stat_v2,ls_v2,fixed_push_mkdir,apex,abb,abb_exec,sendrecv_v2,sendrecv_v2_brotli;",
            vec![
                Feature::Shell2,
                Feature::Cmd,
                Feature::Stat2,
                Feature::Ls2,
                Feature::FixedPushMkdir,
                Feature::Apex,
                Feature::Abb,
                Feature::AbbExec,
                Feature::SendRecv2,
                Feature::SendRecv2Brotli,
            ],
        ),
        (
            "Android 15",
            "device:12345678:ro.product.name=sdk_gphone64_x86_64;ro.product.model=Pixel_8;ro.product.device=shiba;features=shell_v2,cmd,stat_v2,ls_v2,fixed_push_mkdir,apex,abb,abb_exec,sendrecv_v2,sendrecv_v2_brotli,sendrecv_v2_lz4,sendrecv_v2_zstd,devicetracker_proto_format;",
            vec![
                Feature::Shell2,
                Feature::Cmd,
                Feature::Stat2,
                Feature::Ls2,
                Feature::FixedPushMkdir,
                Feature::Apex,
                Feature::Abb,
                Feature::AbbExec,
                Feature::SendRecv2,
                Feature::SendRecv2Brotli,
                Feature::SendRecv2LZ4,
                Feature::SendRecv2Zstd,
                Feature::DeviceTrackerProtoFormat,
            ],
        ),
    ];

    for (name, banner_str, expected_features) in android_versions {
        let (client_stream, server_stream) = duplex(1024);
        let mut client_transport = AsyncStreamTransport::new(client_stream);
        let mut server_transport = AsyncStreamTransport::new(server_stream);

        let banner_payload = banner_str.as_bytes().to_vec();

        let server_task = tokio::spawn(async move {
            let initial_packet = server_transport.read_packet().await.unwrap();
            assert_eq!(initial_packet.header.command, Command::Cnxn);
            assert_eq!(initial_packet.header.arg0, A_VERSION);

            let reply = Packet::new(
                Command::Cnxn,
                A_VERSION,
                MAX_PAYLOAD as u32,
                banner_payload,
                true,
            );
            server_transport.write_packet(&reply).await.unwrap();
        });

        let result = perform_cnxn_handshake(&mut client_transport, &FeatureOptions::default())
            .await
            .unwrap_or_else(|e| panic!("Handshake failed for {name}: {e}"));

        server_task.await.unwrap();

        assert_eq!(
            result.negotiated_version, A_VERSION,
            "Version check for {name}"
        );
        assert_eq!(
            result.negotiated_max_payload, MAX_PAYLOAD,
            "Max payload check for {name}"
        );

        for expected_feat in &expected_features {
            assert!(
                result.remote_features.contains(expected_feat),
                "Missing feature {expected_feat:?} in {name}"
            );
        }
    }
}

#[tokio::test]
async fn test_max_payload_capping() {
    let (client_stream, server_stream) = duplex(1024);
    let mut client_transport = AsyncStreamTransport::new(client_stream);
    let mut server_transport = AsyncStreamTransport::new(server_stream);

    let lower_max_payload = 4096;
    let banner_str = "device:12345678:features=shell_v2;";

    let server_task = tokio::spawn(async move {
        let req = server_transport.read_packet().await.unwrap();
        assert_eq!(req.header.command, Command::Cnxn);

        let reply = Packet::new(
            Command::Cnxn,
            A_VERSION,
            lower_max_payload as u32,
            banner_str.as_bytes().to_vec(),
            true,
        );
        server_transport.write_packet(&reply).await.unwrap();
    });

    let result = perform_cnxn_handshake(&mut client_transport, &FeatureOptions::default())
        .await
        .unwrap();

    server_task.await.unwrap();

    assert_eq!(result.negotiated_max_payload, lower_max_payload);
}

#[tokio::test]
async fn test_malformed_banner_and_invalid_magic() {
    // 1. Invalid Magic
    {
        let (client_stream, mut server_stream) = duplex(1024);
        let mut client_transport = AsyncStreamTransport::new(client_stream);

        let server_task = tokio::spawn(async move {
            let mut buf = [0u8; Header::SIZE];
            use tokio::io::AsyncReadExt;
            server_stream.read_exact(&mut buf).await.unwrap();

            // Send header with corrupted magic bytes
            let mut bad_header = Header::new(Command::Cnxn, A_VERSION, 4096, 0, 0);
            bad_header.magic = 0xdeadbeef;
            let encoded = bad_header.encode();

            use tokio::io::AsyncWriteExt;
            server_stream.write_all(&encoded).await.unwrap();
        });

        let err = perform_cnxn_handshake(&mut client_transport, &FeatureOptions::default()).await;

        server_task.await.unwrap();

        assert!(matches!(err, Err(TransportError::Decode(_))));
    }

    // 2. Corrupt or non-standard banner string (should still parse without panicking)
    {
        let (client_stream, server_stream) = duplex(1024);
        let mut client_transport = AsyncStreamTransport::new(client_stream);
        let mut server_transport = AsyncStreamTransport::new(server_stream);

        let corrupt_banner = "::corrupt=data;;;features=invalid_feat1,shell_v2;missing_equals;";

        let server_task = tokio::spawn(async move {
            let _req = server_transport.read_packet().await.unwrap();

            let reply = Packet::new(
                Command::Cnxn,
                A_VERSION,
                MAX_PAYLOAD as u32,
                corrupt_banner.as_bytes().to_vec(),
                true,
            );
            server_transport.write_packet(&reply).await.unwrap();
        });

        let result = perform_cnxn_handshake(&mut client_transport, &FeatureOptions::default())
            .await
            .unwrap();

        server_task.await.unwrap();

        assert!(result.remote_features.contains(&Feature::Shell2));
    }
}
