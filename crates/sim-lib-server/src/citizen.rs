use sim_citizen_derive::Citizen;
use sim_kernel::{Expr, Result, Symbol};

/// Citizen descriptor for a `server/Address`, wrapping a validated address [`Expr`].
#[derive(Clone, Debug, PartialEq, Citizen)]
#[citizen(symbol = "server/Address", version = 1)]
pub struct ServerAddressDescriptor {
    #[citizen(with = "address_expr")]
    address: Expr,
}

/// Citizen descriptor for a `server/Frame`, capturing the wire metadata of one transport frame.
#[derive(Clone, Debug, PartialEq, Citizen)]
#[citizen(symbol = "server/Frame", version = 1)]
pub struct ServerFrameDescriptor {
    /// Frame format version.
    pub version: u64,
    /// Codec symbol identifying how the payload is encoded.
    pub codec: Symbol,
    /// Frame kind symbol (for example `request` or `reply`).
    pub kind: Symbol,
    /// Message id, present when the frame participates in correlation.
    pub msg_id: Option<u64>,
    /// Id of the message this frame correlates with, when it is a reply.
    pub correlate: Option<u64>,
    /// Raw encoded payload bytes.
    pub payload: Vec<u8>,
}

impl ServerAddressDescriptor {
    /// Builds a descriptor from `address`, validating that it is a well-formed server address.
    pub fn from_expr(address: Expr) -> Result<Self> {
        address_expr::decode(&address)?;
        Ok(Self { address })
    }

    /// Returns the wrapped address expression.
    pub fn as_expr(&self) -> &Expr {
        &self.address
    }
}

impl Default for ServerAddressDescriptor {
    fn default() -> Self {
        Self::from_expr(Expr::Symbol(Symbol::new("local")))
            .expect("default server address descriptor should be valid")
    }
}

impl Default for ServerFrameDescriptor {
    fn default() -> Self {
        Self {
            version: 1,
            codec: Symbol::qualified("codec", "binary"),
            kind: Symbol::new("request"),
            msg_id: Some(1),
            correlate: None,
            payload: b"citizen-frame".to_vec(),
        }
    }
}

/// Returns the class symbol `server/Address` for the address descriptor citizen.
pub fn server_address_class_symbol() -> Symbol {
    Symbol::qualified("server", "Address")
}

/// Returns the class symbol `server/Frame` for the frame descriptor citizen.
pub fn server_frame_class_symbol() -> Symbol {
    Symbol::qualified("server", "Frame")
}

pub(crate) mod address_expr {
    use sim_kernel::{Expr, Result};

    use crate::ServerAddress;

    pub fn encode(expr: &Expr) -> Expr {
        expr.clone()
    }

    pub fn decode(expr: &Expr) -> Result<Expr> {
        ServerAddress::from_expr(expr)?;
        Ok(expr.clone())
    }
}
