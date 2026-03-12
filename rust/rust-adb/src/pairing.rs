use crate::adb_client::{adb_query};

/// Pairs with a device for secure TCP/IP communication.
pub fn adb_pair(host: &str, password: &str) -> anyhow::Result<()> {
    let query = format!("host:pair:{}:{}", password, host);
    let result = adb_query(&query)?;
    println!("{}", result);
    Ok(())
}
