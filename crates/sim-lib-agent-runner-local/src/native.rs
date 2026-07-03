#![allow(missing_docs)]

#[sim::sim_lib(id = "model/local", version = "0.1.0", native_export = true)]
mod local_model_native {
    #[allow(unused_imports)]
    use sim::{
        kernel::{Expr, Result},
        sim_site,
    };

    #[sim_site(symbol = "model-site:local", realize = "realize_local_model")]
    pub fn local_model_site() {}

    pub fn realize_local_model(args: Vec<Expr>) -> Result<Expr> {
        crate::realize_site_args(args)
    }
}
