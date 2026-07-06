//! Shared path-id extraction for the OpenAI gateway routes.
//!
//! Every retrieval/action route parsed its `{id}` with the same
//! `strip_prefix(..).filter(single segment)` shape; these two helpers are the
//! one home for it so the routes only supply their own prefix (and suffix).

/// Extract a single-segment id from `path` by stripping `prefix`.
///
/// Returns `None` when the prefix is absent or the remainder is empty or
/// contains `/` (i.e. it is not a single path segment).
pub(crate) fn id_from_path<'a>(path: &'a str, prefix: &str) -> Option<&'a str> {
    path.strip_prefix(prefix)
        .filter(|id| !id.is_empty() && !id.contains('/'))
}

/// Extract a single-segment id bracketed by `prefix` and `suffix` (e.g. the
/// `{id}` in `/v1/batches/{id}/cancel`).
pub(crate) fn id_from_path_with_suffix<'a>(
    path: &'a str,
    prefix: &str,
    suffix: &str,
) -> Option<&'a str> {
    path.strip_prefix(prefix)
        .and_then(|rest| rest.strip_suffix(suffix))
        .filter(|id| !id.is_empty() && !id.contains('/'))
}

#[cfg(test)]
mod tests {
    use super::{id_from_path, id_from_path_with_suffix};

    #[test]
    fn id_from_path_requires_a_single_segment() {
        assert_eq!(id_from_path("/v1/x/abc", "/v1/x/"), Some("abc"));
        assert_eq!(id_from_path("/v1/x/", "/v1/x/"), None);
        assert_eq!(id_from_path("/v1/x/a/b", "/v1/x/"), None);
        assert_eq!(id_from_path("/other/abc", "/v1/x/"), None);
    }

    #[test]
    fn id_from_path_with_suffix_brackets_the_id() {
        assert_eq!(
            id_from_path_with_suffix("/v1/x/abc/cancel", "/v1/x/", "/cancel"),
            Some("abc")
        );
        assert_eq!(
            id_from_path_with_suffix("/v1/x/abc", "/v1/x/", "/cancel"),
            None
        );
        assert_eq!(
            id_from_path_with_suffix("/v1/x//cancel", "/v1/x/", "/cancel"),
            None
        );
    }
}
