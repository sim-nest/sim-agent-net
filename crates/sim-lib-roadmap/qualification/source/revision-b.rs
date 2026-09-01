// The public anchor moved and its signature changed.
pub mod moved { pub fn public_api(value: u64) -> u64 { value + 1 } }
fn private_helper(value: u64) -> u64 { value * 2 }
