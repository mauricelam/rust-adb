use adb_proto::banner::{
    client_supported_features, negotiate_max_payload, negotiate_version,
    server_advertised_features, Banner, Feature, FeatureOptions, A_VERSION, A_VERSION_MIN,
    A_VERSION_SKIP_CHECKSUM, MAX_PAYLOAD, MAX_PAYLOAD_V1,
};
use proptest::prelude::*;

#[test]
fn test_constants_and_negotiation() {
    assert_eq!(A_VERSION_MIN, 0x0100_0000);
    assert_eq!(A_VERSION_SKIP_CHECKSUM, 0x0100_0001);
    assert_eq!(A_VERSION, 0x0100_0001);

    assert_eq!(MAX_PAYLOAD_V1, 4 * 1024);
    assert_eq!(MAX_PAYLOAD, 1024 * 1024);

    assert_eq!(negotiate_version(A_VERSION, 0x0100_0000), 0x0100_0000);
    assert_eq!(negotiate_version(0x0100_0000, A_VERSION), 0x0100_0000);
    assert_eq!(negotiate_version(A_VERSION, A_VERSION), A_VERSION);

    assert_eq!(negotiate_max_payload(MAX_PAYLOAD, 4096), 4096);
    assert_eq!(negotiate_max_payload(4096, MAX_PAYLOAD), 4096);
    assert_eq!(negotiate_max_payload(MAX_PAYLOAD, MAX_PAYLOAD), MAX_PAYLOAD);
}

#[test]
fn test_banner_fixed_round_trip_cases() {
    let fixed_cases = [
        "device::ro.product.name=x;features=a;ro.product.model=y",
        "device::features=",
        "device::ro.product.name=mock_device;ro.product.model=mock_model;ro.product.device=mock_device;features=shell_v2;",
    ];

    for case in fixed_cases {
        let banner = Banner::parse(case);
        assert_eq!(banner.to_string(), case);
    }
}

proptest! {
    #[test]
    fn prop_banner_round_trip(
        system_type in "[a-z0-9_-]{1,10}",
        serial in "[a-zA-Z0-9_-]{0,10}",
        props in prop::collection::vec(
            ("[a-z0-9_.]+", "[a-zA-Z0-9_.]+"),
            0..5
        ),
        features_opt in prop::option::of(
            prop::collection::vec("[a-z0-9_]+", 0..5)
        ),
        features_first in any::<bool>(),
        ends_with_semicolon in any::<bool>(),
    ) {
        let mut segments = Vec::new();

        let prop_strings: Vec<String> = props
            .iter()
            .filter(|(k, _)| k != "features")
            .map(|(k, v)| format!("{k}={v}"))
            .collect();

        let features_string = features_opt.map(|f_list| format!("features={}", f_list.join(",")));

        if let Some(f_str) = features_string {
            if features_first {
                segments.push(f_str);
                segments.extend(prop_strings);
            } else {
                segments.extend(prop_strings);
                segments.push(f_str);
            }
        } else {
            segments.extend(prop_strings);
        }

        let mut banner_str = format!("{system_type}:{serial}:{}", segments.join(";"));
        if ends_with_semicolon && !segments.is_empty() {
            banner_str.push(';');
        }

        let parsed = Banner::parse(&banner_str);
        prop_assert_eq!(parsed.to_string(), banner_str);
    }
}

#[test]
fn test_pure_supported_features_environment_immunity() {
    unsafe {
        std::env::set_var("ADB_BURST_MODE", "1");
    }

    let opts_no_burst = FeatureOptions { delayed_ack: false };
    let client_feats_no_burst = client_supported_features(&opts_no_burst);
    assert!(!client_feats_no_burst.contains(&Feature::DelayedAck));

    let server_feats_burst = server_advertised_features(&FeatureOptions { delayed_ack: true });
    assert!(!server_feats_burst.contains(&Feature::DelayedAck));
    assert!(server_feats_burst.is_empty());

    unsafe {
        std::env::remove_var("ADB_BURST_MODE");
    }

    let client_feats_no_burst_again = client_supported_features(&opts_no_burst);
    assert_eq!(client_feats_no_burst, client_feats_no_burst_again);

    let opts_burst = FeatureOptions { delayed_ack: true };
    let client_feats_burst = client_supported_features(&opts_burst);
    assert!(client_feats_burst.contains(&Feature::DelayedAck));
}

#[test]
fn test_parse_banner_empty_and_no_features() {
    let banner_str = "host::";
    let banner = Banner::parse(banner_str);
    assert_eq!(banner.system_type, "host");
    assert_eq!(banner.serial, "");
    assert!(banner.properties().is_empty());
    assert!(banner.features().is_empty());
    assert_eq!(banner.to_string(), banner_str);
    assert_eq!(banner.to_bytes(), banner_str.as_bytes());
}

#[test]
fn test_parse_banner_product_properties() {
    let banner_str = "host::ro.product.name=foo;ro.product.model=bar;ro.product.device=baz;";
    let banner = Banner::parse(banner_str);
    assert_eq!(banner.system_type, "host");
    assert_eq!(banner.serial, "");
    assert_eq!(banner.get_property("ro.product.name"), Some("foo"));
    assert_eq!(banner.get_property("ro.product.model"), Some("bar"));
    assert_eq!(banner.get_property("ro.product.device"), Some("baz"));
    assert_eq!(banner.product(), Some("foo"));
    assert_eq!(banner.model(), Some("bar"));
    assert_eq!(banner.device(), Some("baz"));
    assert!(banner.features().is_empty());
    assert_eq!(banner.to_string(), banner_str);
}

#[test]
fn test_parse_banner_features_and_unknown_features() {
    let banner_str = "device:123456:ro.product.name=foo;features=shell_v2,custom_feature_x,cmd";
    let banner = Banner::parse(banner_str);
    assert_eq!(banner.system_type, "device");
    assert_eq!(banner.serial, "123456");
    assert_eq!(banner.product(), Some("foo"));

    assert!(banner.features().contains(&Feature::Shell2));
    assert!(banner.features().contains(&Feature::Cmd));
    assert!(banner
        .features()
        .contains(&Feature::Unknown("custom_feature_x".to_string())));

    // Check round-trip preservation
    assert_eq!(banner.to_string(), banner_str);
    assert_eq!(banner.to_bytes(), banner_str.as_bytes());
}

#[test]
fn test_parse_banner_extra_properties_preserved() {
    let banner_str = "device::ro.product.name=test;custom.key=custom_val;features=abb;";
    let banner = Banner::parse(banner_str);
    assert_eq!(banner.get_property("custom.key"), Some("custom_val"));
    assert!(banner.features().contains(&Feature::Abb));
    assert_eq!(banner.to_string(), banner_str);
}

#[test]
fn test_parse_banner_malformed_no_panic() {
    let inputs = vec![
        "",
        ":",
        "::",
        ":::",
        "device",
        "device:",
        "device::===;;;",
        "device::key_without_val;=val_without_key;key=val=extra",
        ":::extra:colons:",
    ];

    for input in inputs {
        let banner = Banner::parse(input);
        // Parsing malformed input should never panic, and serializing should not panic
        let _s = banner.to_string();
        let _b = banner.to_bytes();
    }
}

#[test]
fn test_feature_enum_str_conversions() {
    let known_features = vec![
        ("shell_v2", Feature::Shell2),
        ("cmd", Feature::Cmd),
        ("stat_v2", Feature::Stat2),
        ("ls_v2", Feature::Ls2),
        ("libusb", Feature::Libusb),
        ("push_sync", Feature::PushSync),
        ("apex", Feature::Apex),
        ("fixed_push_mkdir", Feature::FixedPushMkdir),
        ("abb", Feature::Abb),
        ("abb_exec", Feature::AbbExec),
        (
            "fixed_push_symlink_timestamp",
            Feature::FixedPushSymlinkTimestamp,
        ),
        ("remount_shell", Feature::RemountShell),
        ("track_app", Feature::TrackApp),
        ("sendrecv_v2", Feature::SendRecv2),
        ("sendrecv_v2_brotli", Feature::SendRecv2Brotli),
        ("sendrecv_v2_lz4", Feature::SendRecv2LZ4),
        ("sendrecv_v2_zstd", Feature::SendRecv2Zstd),
        ("sendrecv_v2_dry_run_send", Feature::SendRecv2DryRunSend),
        ("delayed_ack", Feature::DelayedAck),
        ("openscreen_mdns", Feature::OpenscreenMdns),
        (
            "devicetracker_proto_format",
            Feature::DeviceTrackerProtoFormat,
        ),
        ("devraw", Feature::DevRaw),
        ("app_info", Feature::AppInfo),
        ("server_status", Feature::ServerStatus),
    ];

    for (str_repr, feature_variant) in known_features {
        let parsed: Feature = str_repr.parse().unwrap();
        assert_eq!(parsed, feature_variant);
        assert_eq!(parsed.as_str(), str_repr);
    }

    let unknown_str = "something_new";
    let parsed_unknown: Feature = unknown_str.parse().unwrap();
    assert_eq!(
        parsed_unknown,
        Feature::Unknown("something_new".to_string())
    );
    assert_eq!(parsed_unknown.as_str(), "something_new");
}

#[test]
fn test_supported_features() {
    let features = client_supported_features(&FeatureOptions::default());
    assert!(features.contains(&Feature::Shell2));
    assert!(features.contains(&Feature::Cmd));
    assert!(features.contains(&Feature::Stat2));
    assert!(features.contains(&Feature::Ls2));
    assert!(features.contains(&Feature::FixedPushMkdir));
    assert!(features.contains(&Feature::Apex));
    assert!(features.contains(&Feature::Abb));
    assert!(features.contains(&Feature::FixedPushSymlinkTimestamp));
    assert!(features.contains(&Feature::AbbExec));
    assert!(features.contains(&Feature::RemountShell));
    assert!(features.contains(&Feature::TrackApp));
    assert!(features.contains(&Feature::SendRecv2));
    assert!(features.contains(&Feature::SendRecv2Brotli));
    assert!(features.contains(&Feature::SendRecv2LZ4));
    assert!(features.contains(&Feature::SendRecv2Zstd));
    assert!(features.contains(&Feature::SendRecv2DryRunSend));
    assert!(features.contains(&Feature::OpenscreenMdns));
    assert!(features.contains(&Feature::DeviceTrackerProtoFormat));
    assert!(features.contains(&Feature::DevRaw));
    assert!(features.contains(&Feature::AppInfo));
    assert!(features.contains(&Feature::ServerStatus));
}

#[test]
fn test_single_definition_of_limit_constants() {
    // Verifies adb_proto re-exports limit constants from one source (limits.rs)
    assert_eq!(adb_proto::MAX_PAYLOAD, 1024 * 1024);
    assert_eq!(adb_proto::MAX_PAYLOAD_V1, 4096);
    assert_eq!(adb_proto::A_VERSION, 0x0100_0001);
    assert_eq!(adb_proto::A_VERSION_MIN, 0x0100_0000);
    assert_eq!(adb_proto::A_VERSION_SKIP_CHECKSUM, 0x0100_0001);
}
