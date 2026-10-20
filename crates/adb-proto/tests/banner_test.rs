use adb_proto::banner::{
    negotiate_max_payload, negotiate_version, Banner, Feature, A_VERSION, A_VERSION_MIN,
    A_VERSION_SKIP_CHECKSUM, MAX_PAYLOAD, MAX_PAYLOAD_V1,
};

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
fn test_parse_banner_empty_and_no_features() {
    let banner_str = "host::";
    let banner = Banner::parse(banner_str);
    assert_eq!(banner.system_type, "host");
    assert_eq!(banner.serial, "");
    assert!(banner.properties.is_empty());
    assert!(banner.features.is_empty());
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
    assert!(banner.features.is_empty());
    assert_eq!(banner.to_string(), banner_str);
}

#[test]
fn test_parse_banner_features_and_unknown_features() {
    let banner_str = "device:123456:ro.product.name=foo;features=shell_v2,custom_feature_x,cmd";
    let banner = Banner::parse(banner_str);
    assert_eq!(banner.system_type, "device");
    assert_eq!(banner.serial, "123456");
    assert_eq!(banner.product(), Some("foo"));

    assert!(banner.features.contains(&Feature::Shell2));
    assert!(banner.features.contains(&Feature::Cmd));
    assert!(banner
        .features
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
    assert!(banner.features.contains(&Feature::Abb));
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
