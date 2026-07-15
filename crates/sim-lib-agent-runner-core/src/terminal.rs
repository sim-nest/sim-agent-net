use sim_kernel::{Error, Expr, Result};

use crate::ModelResponse;

/// Returns the terminal content item from a model response.
pub fn terminal_model_content(response: &ModelResponse) -> Result<&Expr> {
    response
        .content
        .last()
        .ok_or_else(|| Error::Eval("model response had no terminal content".to_owned()))
}

#[cfg(test)]
mod tests {
    use super::terminal_model_content;
    use crate::ModelResponse;
    use sim_kernel::{Expr, Symbol};

    #[test]
    fn terminal_content_is_last_item() {
        let response = ModelResponse::new(
            Symbol::qualified("runner", "fixture"),
            "fixture",
            vec![
                Expr::String("tool progress".to_owned()),
                Expr::String("final answer".to_owned()),
            ],
            Symbol::new("stop"),
        );

        assert_eq!(
            terminal_model_content(&response).unwrap(),
            &Expr::String("final answer".to_owned())
        );
    }

    #[test]
    fn empty_content_is_an_error() {
        let response = ModelResponse::new(
            Symbol::qualified("runner", "fixture"),
            "fixture",
            Vec::new(),
            Symbol::new("stop"),
        );

        assert!(terminal_model_content(&response).is_err());
    }
}
