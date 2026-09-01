use crate::ImportError;
use sim_codec_config::ConfigDecoder;
use sim_kernel::Expr;

#[derive(Clone, Debug, PartialEq)]
pub struct Catalogs {
    pub proof: Expr,
    pub repositories: Expr,
    pub resources: Expr,
    pub tractability: Expr,
}

pub fn import_catalogs(
    proof: &str,
    repositories: &str,
    resources: &str,
    tractability: &str,
) -> Result<Catalogs, ImportError> {
    fn one(name: &str, s: &str) -> Result<Expr, ImportError> {
        let value = ConfigDecoder::table()
            .decode_text(s)
            .map_err(|e| ImportError::Invalid(format!("{name} catalog: {e}")))?;
        reject_shell(name, &value)?;
        Ok(value)
    }
    Ok(Catalogs {
        proof: one("proof", proof)?,
        repositories: one("repository", repositories)?,
        resources: one("resource", resources)?,
        tractability: one("tractability", tractability)?,
    })
}
fn reject_shell(path: &str, e: &Expr) -> Result<(), ImportError> {
    match e {
        Expr::Map(xs) => {
            for (k, v) in xs {
                let key = match k {
                    Expr::Symbol(s) => s.name.as_ref(),
                    Expr::String(s) => s.as_str(),
                    _ => "",
                };
                if matches!(key, "command" | "run" | "check") && matches!(v, Expr::String(_)) {
                    return Err(ImportError::Invalid(format!(
                        "{path}.{key} must be argv, not opaque shell text"
                    )));
                }
                reject_shell(&format!("{path}.{key}"), v)?
            }
        }
        Expr::List(xs) | Expr::Vector(xs) => {
            for x in xs {
                reject_shell(path, x)?
            }
        }
        _ => {}
    }
    Ok(())
}
