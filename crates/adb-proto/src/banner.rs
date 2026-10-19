use std::collections::BTreeMap;

/// Builds an ADB connection banner string.
/// Format: `<system_type>::<prop_1>=<val_1>;<prop_2>=<val_2>;...;features=<features>`
pub fn build_banner(system_type: &str, properties: &[(&str, &str)], features: &[&str]) -> String {
    let mut parts = Vec::new();
    for (k, v) in properties {
        parts.push(format!("{k}={v}"));
    }
    let features_str = features.join(",");
    parts.push(format!("features={features_str}"));
    format!("{system_type}::{}", parts.join(";"))
}

/// Parsed banner information.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Banner {
    pub system_type: String,
    pub serial: String,
    pub properties: BTreeMap<String, String>,
    pub features: Vec<String>,
}

impl Banner {
    pub fn parse(banner_str: &str) -> Self {
        let pieces: Vec<&str> = banner_str.split(':').collect();
        let mut banner = Banner::default();

        if let Some(&system_type) = pieces.first() {
            banner.system_type = system_type.to_string();
        }
        if pieces.len() > 1 {
            banner.serial = pieces[1].to_string();
        }
        if pieces.len() > 2 {
            let props_str = pieces[2];
            for prop in props_str.split(';') {
                if prop.is_empty() {
                    continue;
                }
                if let Some((k, v)) = prop.split_once('=') {
                    if k == "features" {
                        banner.features = v
                            .split(',')
                            .filter(|s| !s.is_empty())
                            .map(String::from)
                            .collect();
                    } else {
                        banner.properties.insert(k.to_string(), v.to_string());
                    }
                }
            }
        }

        banner
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_and_parse_banner() {
        let banner_str = build_banner(
            "device",
            &[
                ("ro.product.name", "mock"),
                ("ro.product.model", "mock_model"),
                ("ro.product.device", "mock_device"),
            ],
            &["shell_v2", "cmd"],
        );
        assert_eq!(
            banner_str,
            "device::ro.product.name=mock;ro.product.model=mock_model;ro.product.device=mock_device;features=shell_v2,cmd"
        );

        let parsed = Banner::parse(&banner_str);
        assert_eq!(parsed.system_type, "device");
        assert_eq!(parsed.properties.get("ro.product.name").unwrap(), "mock");
        assert_eq!(
            parsed.properties.get("ro.product.model").unwrap(),
            "mock_model"
        );
        assert_eq!(
            parsed.properties.get("ro.product.device").unwrap(),
            "mock_device"
        );
        assert_eq!(parsed.features, vec!["shell_v2", "cmd"]);
    }
}
