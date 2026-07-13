use sim_cookbook::{RecipeCard, RecipeRun, RecipeStore, grouped_view, ordered_cards, search, view};
use sim_kernel::Cx;
use sim_lib_cookbook::{LifecycleAction, LoadableLibList, lifecycle_action};

pub(crate) fn render_error_json(message: &str) -> String {
    let mut body = String::from("{\"error\":");
    push_json_str(&mut body, message);
    body.push('}');
    body
}

pub(crate) fn render_index_json(cx: &Cx, store: &RecipeStore, diagnostics: &[String]) -> String {
    // `books` keeps the flat book -> chapter -> recipe tree (back-compat);
    // `families` adds the two-level grouping (family -> domain book -> chapter
    // -> recipe), and `recipes` is the flat ordered list for search-free
    // listing.
    let mut out = String::from("{\"books\":[");
    let cookbook = view(store);
    for (book_index, book) in cookbook.books.iter().enumerate() {
        comma(&mut out, book_index);
        push_book(&mut out, cx, book);
    }
    out.push_str("],\"families\":[");
    for (family_index, family) in grouped_view(store).families.iter().enumerate() {
        comma(&mut out, family_index);
        out.push_str("{\"family\":");
        push_json_str(&mut out, &family.family);
        out.push_str(",\"books\":[");
        for (book_index, book) in family.books.iter().enumerate() {
            comma(&mut out, book_index);
            push_book(&mut out, cx, book);
        }
        out.push_str("]}");
    }
    out.push_str("],\"diagnostics\":");
    push_json_array(&mut out, diagnostics);
    out.push_str(",\"recipes\":[");
    for (index, card) in ordered_cards(store).iter().enumerate() {
        comma(&mut out, index);
        push_recipe_summary(&mut out, cx, card);
    }
    out.push_str("]}");
    out
}

fn push_book(out: &mut String, cx: &Cx, book: &sim_cookbook::BookView) {
    out.push_str("{\"id\":");
    push_json_str(out, &book.id);
    out.push_str(",\"title\":");
    push_json_str(out, &book.title);
    out.push_str(",\"chapters\":[");
    for (chapter_index, chapter) in book.chapters.iter().enumerate() {
        comma(out, chapter_index);
        out.push_str("{\"name\":");
        push_json_str(out, &chapter.name);
        out.push_str(",\"title\":");
        push_json_str(out, &chapter.title);
        out.push_str(",\"recipes\":[");
        for (recipe_index, recipe) in chapter.recipes.iter().enumerate() {
            comma(out, recipe_index);
            push_recipe_summary(out, cx, recipe);
        }
        out.push_str("]}");
    }
    out.push_str("]}");
}

pub(crate) fn render_search_json(cx: &Cx, store: &RecipeStore, query: &str) -> String {
    let mut out = String::from("{\"query\":");
    push_json_str(&mut out, query);
    out.push_str(",\"recipes\":[");
    for (index, card) in search(store, query).iter().enumerate() {
        comma(&mut out, index);
        push_recipe_summary(&mut out, cx, card);
    }
    out.push_str("]}");
    out
}

pub(crate) fn render_recipe_json(cx: &Cx, store: &RecipeStore, card: &RecipeCard) -> String {
    let mut out = String::from("{\"id\":");
    push_json_str(&mut out, &card.id);
    out.push_str(",\"title\":");
    push_json_str(&mut out, &card.title);
    out.push_str(",\"book\":");
    push_json_str(&mut out, &card.book);
    out.push_str(",\"chapter\":");
    push_json_str(&mut out, &card.chapter);
    out.push_str(",\"codec\":");
    push_json_str(&mut out, &card.codec);
    out.push_str(",\"purpose\":");
    push_json_str(&mut out, &card.purpose);
    out.push_str(",\"setup\":");
    push_json_str(&mut out, &String::from_utf8_lossy(&card.setup));
    out.push_str(",\"requires\":");
    push_json_array(&mut out, &card.requires);
    out.push_str(",\"tags\":");
    push_json_array(&mut out, &card.tags);
    out.push_str(",\"next\":");
    if let Some(next) = sim_cookbook::next(store, &card.id) {
        push_json_str(&mut out, &next.id);
    } else {
        out.push_str("null");
    }
    push_lifecycle_fields(&mut out, cx, card);
    out.push('}');
    out
}

pub(crate) fn render_run_json(run: &RecipeRun) -> String {
    let mut out = String::from("{\"recipe\":");
    push_json_str(&mut out, &run.recipe);
    out.push_str(",\"ok\":");
    out.push_str(if run.ok { "true" } else { "false" });
    out.push_str(",\"forms\":");
    out.push_str(&run.forms.to_string());
    out.push_str(",\"results\":");
    push_json_array(&mut out, &run.results);
    out.push_str(",\"checks\":[");
    for (index, check) in run.checks.iter().enumerate() {
        comma(&mut out, index);
        out.push_str("{\"form\":");
        out.push_str(&check.form.to_string());
        out.push_str(",\"expected\":");
        push_json_str(&mut out, &check.expected);
        out.push_str(",\"actual\":");
        push_json_str(&mut out, &check.actual);
        out.push_str(",\"pass\":");
        out.push_str(if check.pass { "true" } else { "false" });
        out.push('}');
    }
    out.push_str("]}");
    out
}

fn push_recipe_summary(out: &mut String, cx: &Cx, card: &RecipeCard) {
    out.push_str("{\"id\":");
    push_json_str(out, &card.id);
    out.push_str(",\"title\":");
    push_json_str(out, &card.title);
    out.push_str(",\"book\":");
    push_json_str(out, &card.book);
    out.push_str(",\"chapter\":");
    push_json_str(out, &card.chapter);
    // A `sandbox-descriptor` recipe documents a descriptor whose live result is
    // not reproducible in the sandbox; every other recipe is runnable.
    out.push_str(",\"runnable\":");
    let runnable = !card.tags.iter().any(|t| t == "sandbox-descriptor");
    out.push_str(if runnable { "true" } else { "false" });
    push_lifecycle_fields(out, cx, card);
    out.push('}');
}

fn push_lifecycle_fields(out: &mut String, cx: &Cx, card: &RecipeCard) {
    let lifecycle = lifecycle_action(card);
    let action = lifecycle.as_ref().map(|(action, _)| *action);
    let lib = lifecycle
        .as_ref()
        .map(|(_, lib)| lib.as_str())
        .unwrap_or(card.book.as_str());
    out.push_str(",\"action\":");
    match action {
        Some(LifecycleAction::Load) => push_json_str(out, "load"),
        Some(LifecycleAction::Unload) => push_json_str(out, "unload"),
        None => out.push_str("null"),
    }
    out.push_str(",\"lib\":");
    push_json_str(out, lib);
    out.push_str(",\"loaded\":");
    out.push_str(if LoadableLibList::is_loaded(cx, lib) {
        "true"
    } else {
        "false"
    });
}

fn push_json_array(out: &mut String, items: &[String]) {
    out.push('[');
    for (index, item) in items.iter().enumerate() {
        comma(out, index);
        push_json_str(out, item);
    }
    out.push(']');
}

fn push_json_str(out: &mut String, text: &str) {
    out.push('"');
    for ch in text.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            ch if ch < ' ' => out.push_str(&format!("\\u{:04x}", ch as u32)),
            ch => out.push(ch),
        }
    }
    out.push('"');
}

fn comma(out: &mut String, index: usize) {
    if index > 0 {
        out.push(',');
    }
}
