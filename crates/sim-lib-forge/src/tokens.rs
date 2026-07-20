//! Prompt-token estimation shared by FORGE authoring paths.

/// Splits prose into the semantic token stream used by FORGE prompt budgets.
pub fn semantic_tokens(prose: &str) -> Vec<String> {
    prose
        .split(|ch: char| !ch.is_ascii_alphanumeric() && ch != '-' && ch != '_' && ch != '/')
        .filter_map(|raw| {
            let token = raw.trim().to_ascii_lowercase();
            (!token.is_empty() && !is_stop_word(&token)).then_some(token)
        })
        .collect()
}

/// Estimates prompt tokens with the FORGE baseline semantic-token counter.
pub fn estimate_prompt_tokens(prose: &str) -> usize {
    semantic_tokens(prose).len()
}

fn is_stop_word(token: &str) -> bool {
    matches!(
        token,
        "a" | "an" | "and" | "for" | "in" | "of" | "on" | "please" | "the" | "to" | "with"
    )
}
