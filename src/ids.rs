use rand::RngExt;
use std::time::{SystemTime, UNIX_EPOCH};

pub(crate) fn unix_timestamp() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or_default()
}

pub(crate) fn prefixed_id(prefix: &str) -> String {
    format!("{}{}", prefix, random_uuid_like())
}

pub(crate) fn prefixed_compact_id(prefix: &str) -> String {
    format!("{}{:032x}", prefix, rand::rng().random::<u128>())
}

fn random_uuid_like() -> String {
    let hex = format!("{:032x}", rand::rng().random::<u128>());
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}
