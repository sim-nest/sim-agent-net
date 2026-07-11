use sim_cookbook::{RecipeCard, RecipeStore, next, ordered_cards, view};
use sim_kernel::{Cx, Result};
use sim_lib_cookbook::{SeededLibCatalog, run_recipe_with_catalog, seeded_recipe_store};

use crate::cookbook_web_json::{
    render_error_json, render_index_json, render_recipe_json, render_run_json, render_search_json,
};

const JSON: &str = "application/json; charset=utf-8";
const HTML: &str = "text/html; charset=utf-8";

/// Cookbook state exposed by the WebUI route and JSON API.
#[derive(Clone, Debug)]
pub struct CookbookWebState {
    store: RecipeStore,
}

impl CookbookWebState {
    /// Build a WebUI state from the crate-shipped seeded recipe books.
    pub fn seeded() -> Result<Self> {
        Ok(Self {
            store: seeded_recipe_store()?,
        })
    }

    /// Build a WebUI state from an existing recipe store.
    pub fn from_store(store: RecipeStore) -> Self {
        Self { store }
    }

    /// Build an empty WebUI state for tests and no-recipes deployments.
    pub fn empty() -> Self {
        Self::from_store(RecipeStore::new())
    }

    /// Access the backing cookbook store.
    pub fn store(&self) -> &RecipeStore {
        &self.store
    }

    /// Route one cookbook WebUI request.
    ///
    /// `target` is an HTTP request target such as `/api/cookbook?q=x`. The run
    /// endpoint requires a runtime context because it executes the same
    /// `sim-lib-cookbook` recipe runner used by the runtime ops and CLI.
    pub fn handle_request(
        &self,
        method: &str,
        target: &str,
        cx: Option<&mut Cx>,
    ) -> CookbookWebResponse {
        let (path, query) = split_target(target);
        match (method, path) {
            ("GET", "/cookbook") => {
                let selected = query.and_then(|q| query_value(q, "recipe"));
                CookbookWebResponse::html(render_page(&self.store, selected.as_deref()))
            }
            ("GET", "/api/cookbook") => CookbookWebResponse::json(render_index_json(&self.store)),
            ("GET", "/api/cookbook/search") => {
                let q = query
                    .and_then(|query| query_value(query, "q"))
                    .unwrap_or_default();
                CookbookWebResponse::json(render_search_json(&self.store, &q))
            }
            ("POST", path)
                if path.starts_with("/api/cookbook/recipe/") && path.ends_with("/run") =>
            {
                let start = "/api/cookbook/recipe/".len();
                let end = path.len() - "/run".len();
                let raw_id = &path[start..end];
                let Some(cx) = cx else {
                    return CookbookWebResponse::server_error(
                        "cookbook run endpoint requires a runtime context",
                    );
                };
                let card = match decode_component(raw_id, false)
                    .and_then(|id| resolve_recipe(&self.store, &id))
                {
                    Ok(card) => card.clone(),
                    Err(err) => return response_for_resolve_error(err),
                };
                // COOKBOOK_7: the run loads the recipe's `requires` from the
                // catalog before eval, so a domain absent from the boot Cx is
                // loaded on demand rather than failing as "not loaded".
                let catalog = SeededLibCatalog::standard();
                match run_recipe_with_catalog(cx, &catalog, &card) {
                    Ok(run) => CookbookWebResponse::json(render_run_json(&run)),
                    Err(err) => CookbookWebResponse::server_error(err.to_string()),
                }
            }
            (_, path) if path.starts_with("/api/cookbook/recipe/") && path.ends_with("/run") => {
                CookbookWebResponse::method_not_allowed()
            }
            ("GET", path) if path.starts_with("/api/cookbook/recipe/") => {
                let raw_id = &path["/api/cookbook/recipe/".len()..];
                match decode_component(raw_id, false)
                    .and_then(|id| resolve_recipe(&self.store, &id))
                {
                    Ok(card) => CookbookWebResponse::json(render_recipe_json(&self.store, card)),
                    Err(err) => response_for_resolve_error(err),
                }
            }
            (_, "/api/cookbook") | (_, "/api/cookbook/search") | (_, "/cookbook") => {
                CookbookWebResponse::method_not_allowed()
            }
            (_, path) if path.starts_with("/api/cookbook/recipe/") => {
                CookbookWebResponse::method_not_allowed()
            }
            _ => CookbookWebResponse::not_found("unknown cookbook route"),
        }
    }
}

/// Minimal HTTP response model for cookbook route adapters.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CookbookWebResponse {
    /// HTTP status code.
    pub status: u16,
    /// Response `Content-Type`.
    pub content_type: &'static str,
    /// UTF-8 response body.
    pub body: String,
}

impl CookbookWebResponse {
    fn html(body: String) -> Self {
        Self {
            status: 200,
            content_type: HTML,
            body,
        }
    }

    fn json(body: String) -> Self {
        Self {
            status: 200,
            content_type: JSON,
            body,
        }
    }

    fn not_found(message: impl AsRef<str>) -> Self {
        Self::error(404, message)
    }

    fn method_not_allowed() -> Self {
        Self::error(405, "method not allowed")
    }

    fn server_error(message: impl AsRef<str>) -> Self {
        Self::error(500, message)
    }

    fn error(status: u16, message: impl AsRef<str>) -> Self {
        Self {
            status,
            content_type: JSON,
            body: render_error_json(message.as_ref()),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum ResolveError {
    Unknown(String),
    Ambiguous(String),
    BadRequest(String),
}

fn response_for_resolve_error(err: ResolveError) -> CookbookWebResponse {
    match err {
        ResolveError::Unknown(message) => CookbookWebResponse::not_found(message),
        ResolveError::Ambiguous(message) | ResolveError::BadRequest(message) => {
            CookbookWebResponse::error(400, message)
        }
    }
}

fn resolve_recipe<'a>(
    store: &'a RecipeStore,
    id: &str,
) -> std::result::Result<&'a RecipeCard, ResolveError> {
    if id.trim().is_empty() {
        return Err(ResolveError::BadRequest(
            "cookbook recipe id must not be empty".to_owned(),
        ));
    }
    if let Some(card) = store.card(id) {
        return Ok(card);
    }
    let suffix = format!("/{id}");
    let candidates: Vec<&RecipeCard> = ordered_cards(store)
        .into_iter()
        .filter(|card| card.id.ends_with(&suffix))
        .collect();
    match candidates.as_slice() {
        [card] => Ok(*card),
        [] => Err(ResolveError::Unknown(format!("unknown recipe {id}"))),
        many => Err(ResolveError::Ambiguous(format!(
            "ambiguous recipe {id}: {}",
            many.iter()
                .map(|card| card.id.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ))),
    }
}

fn render_page(store: &RecipeStore, selected: Option<&str>) -> String {
    let selected_card = selected
        .and_then(|id| resolve_recipe(store, id).ok())
        .or_else(|| ordered_cards(store).first().copied());
    let selected_id = selected_card.map(|card| card.id.as_str());
    let mut out = String::from(
        r#"<!DOCTYPE html><html lang="en"><head><meta charset="utf-8" /><meta name="viewport" content="width=device-width, initial-scale=1" /><title>SIM Cookbook</title><link rel="stylesheet" href="/styles/theme.css" /></head><body><nav class="top-nav"><a href="/">Shell</a><a href="/cookbook" aria-current="page">Cookbook</a></nav><main class="cookbook-shell">"#,
    );
    out.push_str(r#"<aside class="cookbook-rail"><input type="search" aria-label="Search recipes" placeholder="Search" /><nav class="cookbook-tree">"#);
    if store.is_empty() {
        out.push_str(r#"<p class="empty-state">No recipes loaded.</p>"#);
    } else {
        render_tree_html(&mut out, store, selected_id);
    }
    out.push_str(r#"</nav></aside><section class="cookbook-main">"#);
    match selected_card {
        Some(card) => render_recipe_html(&mut out, store, card),
        None => out.push_str(r#"<h1>Cookbook</h1><p class="empty-state">No recipes loaded.</p>"#),
    }
    out.push_str("</section></main></body></html>");
    out
}

fn render_tree_html(out: &mut String, store: &RecipeStore, selected: Option<&str>) {
    for book in view(store).books {
        out.push_str("<section><h2>");
        push_html(out, &book.title);
        out.push_str("</h2>");
        for chapter in book.chapters {
            out.push_str("<div><h3>");
            push_html(out, &chapter.title);
            out.push_str("</h3><ul>");
            for recipe in chapter.recipes {
                let active = selected == Some(recipe.id.as_str());
                out.push_str("<li><a href=\"/cookbook?recipe=");
                push_attr(out, &recipe.id);
                out.push('"');
                if active {
                    out.push_str(" aria-current=\"page\" class=\"selected\"");
                }
                out.push('>');
                push_html(out, &recipe.title);
                out.push_str("</a></li>");
            }
            out.push_str("</ul></div>");
        }
        out.push_str("</section>");
    }
}

fn render_recipe_html(out: &mut String, store: &RecipeStore, card: &RecipeCard) {
    out.push_str("<h1>");
    push_html(out, &card.title);
    out.push_str("</h1><div class=\"purpose\">");
    push_purpose_html(out, &card.purpose);
    out.push_str("</div><div class=\"recipe-actions\"><button type=\"button\">Copy</button><button type=\"button\">Run</button></div><pre><code>");
    push_html(out, &String::from_utf8_lossy(&card.setup));
    out.push_str("</code></pre><section class=\"results-panel\" aria-live=\"polite\">Run this recipe to see pass/fail data.</section><footer>");
    if let Some(next) = next(store, &card.id) {
        out.push_str("<a href=\"/cookbook?recipe=");
        push_attr(out, &next.id);
        out.push_str("\">Next recipe -&gt;</a>");
    } else {
        out.push_str("<span>No next recipe.</span>");
    }
    out.push_str("</footer>");
}

fn split_target(target: &str) -> (&str, Option<&str>) {
    match target.split_once('?') {
        Some((path, query)) => (path, Some(query)),
        None => (target, None),
    }
}

fn query_value(query: &str, key: &str) -> Option<String> {
    query.split('&').find_map(|pair| {
        let (name, value) = pair.split_once('=').unwrap_or((pair, ""));
        (name == key).then(|| decode_component(value, true).unwrap_or_default())
    })
}

fn decode_component(raw: &str, plus_is_space: bool) -> std::result::Result<String, ResolveError> {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                let hi = hex(bytes[i + 1]);
                let lo = hex(bytes[i + 2]);
                match (hi, lo) {
                    (Some(hi), Some(lo)) => out.push((hi << 4) | lo),
                    _ => {
                        return Err(ResolveError::BadRequest(
                            "invalid percent escape in cookbook route".to_owned(),
                        ));
                    }
                }
                i += 3;
            }
            b'%' => {
                return Err(ResolveError::BadRequest(
                    "truncated percent escape in cookbook route".to_owned(),
                ));
            }
            b'+' if plus_is_space => {
                out.push(b' ');
                i += 1;
            }
            byte => {
                out.push(byte);
                i += 1;
            }
        }
    }
    String::from_utf8(out).map_err(|err| ResolveError::BadRequest(err.to_string()))
}

fn hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn push_purpose_html(out: &mut String, purpose: &str) {
    let mut wrote = false;
    let mut open_p = false;
    for raw in purpose.lines() {
        let line = raw.trim();
        if line.is_empty() {
            if open_p {
                out.push_str("</p>");
                open_p = false;
            }
            continue;
        }
        if let Some(title) = line.strip_prefix("# ") {
            if open_p {
                out.push_str("</p>");
                open_p = false;
            }
            out.push_str("<h2>");
            push_html(out, title);
            out.push_str("</h2>");
        } else {
            if !open_p {
                out.push_str("<p>");
                open_p = true;
            } else {
                out.push(' ');
            }
            push_html(out, line);
        }
        wrote = true;
    }
    if open_p {
        out.push_str("</p>");
    }
    if !wrote {
        out.push_str("<p>No purpose provided.</p>");
    }
}

fn push_html(out: &mut String, text: &str) {
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            ch => out.push(ch),
        }
    }
}

fn push_attr(out: &mut String, text: &str) {
    push_html(out, text);
}
