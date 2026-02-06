pub mod host;

#[cfg(target_os = "linux")]
pub mod daemon;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_module_structure() {
        // Just verify it compiles and we can access constants
        assert_eq!(host::ADB_CLASS, 0xff);
    }
}
