use adb_client::{forward_dest_is_featured, forward_targets_are_valid};
use adb_proto::{Feature, Features};

#[test]
fn test_forward_targets_are_valid_success() {
    assert_eq!(forward_targets_are_valid("tcp:1234", "tcp:5678"), Ok(()));
    assert_eq!(forward_targets_are_valid("tcp:0", "tcp:5678"), Ok(()));
    assert_eq!(
        forward_targets_are_valid("localabstract:foo", "localreserved:bar"),
        Ok(())
    );
}

#[test]
fn test_forward_targets_are_valid_invalid_source_port() {
    assert_eq!(
        forward_targets_are_valid("tcp:-1", "tcp:5678"),
        Err("Invalid source port: '-1'".to_string())
    );
    assert_eq!(
        forward_targets_are_valid("tcp:abc", "tcp:5678"),
        Err("Invalid source port: 'abc'".to_string())
    );
}

#[test]
fn test_forward_targets_are_valid_invalid_destination_port() {
    assert_eq!(
        forward_targets_are_valid("tcp:1234", "tcp:0"),
        Err("Invalid destination port: '0'".to_string())
    );
    assert_eq!(
        forward_targets_are_valid("tcp:1234", "tcp:-5"),
        Err("Invalid destination port: '-5'".to_string())
    );
    assert_eq!(
        forward_targets_are_valid("tcp:1234", "tcp:xyz"),
        Err("Invalid destination port: 'xyz'".to_string())
    );
}

#[test]
fn test_forward_dest_is_featured() {
    let empty_features = Features::new();
    let mut dev_raw_features = Features::new();
    dev_raw_features.push(Feature::DevRaw);

    assert_eq!(
        forward_dest_is_featured("dev-raw:foo", &empty_features),
        Err("dev-raw is not supported by the device".to_string())
    );
    assert_eq!(
        forward_dest_is_featured("dev-raw:foo", &dev_raw_features),
        Ok(())
    );
    assert_eq!(
        forward_dest_is_featured("tcp:1234", &empty_features),
        Ok(())
    );
}
