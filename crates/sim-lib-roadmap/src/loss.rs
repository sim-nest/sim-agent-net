#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RenderLoss {
    pub path: String,
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LossReport {
    pub losses: Vec<RenderLoss>,
}

impl std::fmt::Display for LossReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "flat v3 rendering would lose {} field(s)",
            self.losses.len()
        )
    }
}
impl std::error::Error for LossReport {}
