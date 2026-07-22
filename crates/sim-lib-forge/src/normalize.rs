use sim_kernel::{ContentId, Datum, Error, Expr, Result};

/// Normalizes prose into the byte-level source identity used by FORGE.
///
/// The normal form trims leading and trailing ASCII whitespace and collapses
/// each internal ASCII whitespace run to one space. It preserves case,
/// spelling, punctuation, and non-ASCII bytes exactly; this is a deterministic
/// byte-level key, not a semantic lookup.
pub fn normalize_prose(prose: &str) -> Result<(String, ContentId)> {
    let normalized = collapse_ascii_whitespace(prose);
    if normalized.is_empty() {
        return Err(Error::Eval("forge lift prose must not be empty".to_owned()));
    }
    Ok((normalized.clone(), source_content_id(&normalized)?))
}

fn collapse_ascii_whitespace(prose: &str) -> String {
    let mut out = String::with_capacity(prose.len());
    let mut pending_space = false;

    for ch in prose.chars() {
        if ch.is_ascii_whitespace() {
            if !out.is_empty() {
                pending_space = true;
            }
            continue;
        }
        if pending_space {
            out.push(' ');
            pending_space = false;
        }
        out.push(ch);
    }

    out
}

fn source_content_id(normalized: &str) -> Result<ContentId> {
    Datum::try_from(Expr::String(normalized.to_owned()))?.content_id()
}
