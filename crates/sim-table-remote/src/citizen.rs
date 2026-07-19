use sim_citizen_derive::Citizen;
use sim_kernel::Symbol;

/// Citizen record describing a [`RemoteDir`](crate::RemoteDir) and its backing
/// site.
#[derive(Clone, Debug, PartialEq, Citizen)]
#[citizen(symbol = "table/RemoteDir", version = 0)]
pub struct RemoteDirDescriptor {
    /// Identifier of the remote site kind backing the directory.
    pub site_kind: String,
    /// Codec for row encoding and decoding over the wire.
    #[citizen(with = "sim_table_core::citizen_fields::symbol")]
    pub codec: Symbol,
    /// Path segments locating the directory within the remote site.
    #[citizen(with = "sim_table_core::citizen_fields::path_segments")]
    pub path: Vec<String>,
}

impl Default for RemoteDirDescriptor {
    fn default() -> Self {
        Self {
            site_kind: String::new(),
            codec: Symbol::qualified("codec", "binary"),
            path: Vec::new(),
        }
    }
}

/// Returns the citizen class symbol for [`RemoteDirDescriptor`].
pub fn remote_dir_class_symbol() -> Symbol {
    Symbol::qualified("table", "RemoteDir")
}
