use sim_kernel::Expr;
/// Current stable continuity schema.
pub const CURRENT_SCHEMA_VERSION: u64 = 1;
/// Explicit migration refusal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MigrationError {
    /// No migration is defined from this version.
    UnsupportedVersion(u64),
}
/// Migrates an encoded read-construct payload. Version one is identity; no implicit legacy form exists.
pub fn migrate(version: u64, value: Expr) -> Result<Expr, MigrationError> {
    if version == CURRENT_SCHEMA_VERSION {
        Ok(value)
    } else {
        Err(MigrationError::UnsupportedVersion(version))
    }
}
