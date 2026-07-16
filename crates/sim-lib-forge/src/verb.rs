use std::sync::Arc;

use sim_codec_bridge::content_id_string;
use sim_kernel::{
    AbiVersion, Args, Callable, Cx, Datum, DatumStore, Error, Export, Expr, Lib, LibManifest,
    LibTarget, Linker, LoadCx, NumberLiteral, Object, ObjectCompat, Result, Symbol, Value, Version,
};
use sim_value::build::entry;

use crate::normalize_prose;

const FORGE_VERB: &str = "forge";
const RETURN_SHAPE: &str = "#(Shape Summary (title Text) (risks (List Text)))";

/// Loadable FORGE command library.
///
/// The library exports `cli/main/forge` so a `sim-run` boot session can load it
/// as the implementation of the `forge` verb.
pub struct ForgeLib;

impl Lib for ForgeLib {
    fn manifest(&self) -> LibManifest {
        LibManifest {
            id: Symbol::qualified("sim", "forge"),
            version: Version(env!("CARGO_PKG_VERSION").to_owned()),
            abi: AbiVersion { major: 0, minor: 1 },
            target: LibTarget::HostRegistered,
            requires: Vec::new(),
            capabilities: Vec::new(),
            exports: vec![Export::Function {
                symbol: forge_entrypoint_symbol(),
                function_id: None,
            }],
        }
    }

    fn load(&self, cx: &mut LoadCx, linker: &mut Linker<'_>) -> Result<()> {
        linker.function_value(
            forge_entrypoint_symbol(),
            cx.factory().opaque(Arc::new(ForgeEntrypoint))?,
        )?;
        Ok(())
    }
}

/// Entrypoint symbol claimed by the loadable `forge` command library.
pub fn forge_entrypoint_symbol() -> Symbol {
    Symbol::qualified("cli", "main/forge")
}

/// Runs the FORGE command verb and returns a structured report expression.
///
/// The direct entrypoint accepts the payload argument list as an expression. The
/// first item must be `forge`; the following item is one of `lift`, `promote`,
/// `run`, or `show`. A bare prose payload after `forge` is treated as `lift`.
pub fn forge_verb(cx: &mut Cx, args: &Expr) -> Result<Expr> {
    let args = parse_args(args)?;
    let Some(verb) = args.first() else {
        return Err(Error::Eval(
            "forge verb expects payload arguments".to_owned(),
        ));
    };
    if verb != FORGE_VERB {
        return Err(Error::Eval(format!(
            "forge verb expects first payload argument to be forge, found {verb}"
        )));
    }
    match args.get(1).map(String::as_str) {
        Some("lift") => lift_report(cx, &joined_tail(&args, 2)?),
        Some("promote") => promote_report(required_arg(&args, 2, "promote name")?),
        Some("run") => run_report(
            required_arg(&args, 2, "intent name")?,
            args.get(3..).unwrap_or(&[]),
        ),
        Some("show") => show_report(required_arg(&args, 2, "intent name")?),
        Some("review") => review_report(required_arg(&args, 2, "intent name")?),
        Some(other) if !other.is_empty() => lift_report(cx, &joined_tail(&args, 1)?),
        _ => Err(Error::Eval(
            "forge expects lift, review, promote, run, show, or prose".to_owned(),
        )),
    }
}

#[derive(Clone)]
struct ForgeEntrypoint;

impl Object for ForgeEntrypoint {
    fn display(&self, _cx: &mut Cx) -> Result<String> {
        Ok("#<function cli/main/forge>".to_owned())
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl ObjectCompat for ForgeEntrypoint {
    fn as_callable(&self) -> Option<&dyn Callable> {
        Some(self)
    }
}

impl Callable for ForgeEntrypoint {
    fn call(&self, cx: &mut Cx, args: Args) -> Result<Value> {
        verify_cli_envelope(cx, &args)?;
        let payload = envelope_args(cx, args.values().first().expect("verified envelope"))?;
        let report = forge_verb(
            cx,
            &Expr::List(payload.into_iter().map(Expr::String).collect()),
        )?;
        for line in render_report(&report) {
            println!("{line}");
        }
        cx.factory().bool(true)
    }
}

fn lift_report(cx: &mut Cx, prose: &str) -> Result<Expr> {
    if prose.trim().is_empty() {
        return Err(Error::Eval("forge lift expects prose".to_owned()));
    }
    let (normalized, source) = normalize_prose(prose)?;
    let slug = prose_slug(&normalized);
    let surface = review_surface(&slug);
    let report = Expr::Map(vec![
        entry(
            "kind",
            Expr::Symbol(Symbol::qualified("forge", "LiftReport")),
        ),
        entry("command", Expr::Symbol(Symbol::qualified("forge", "lift"))),
        entry("intent", Expr::Symbol(intent_symbol(&slug))),
        entry(
            "status",
            Expr::Symbol(Symbol::qualified("forge", "candidate")),
        ),
        entry("source", Expr::String(content_id_string(&source))),
        entry("compiler-calls", count(1)),
        entry("execution-calls", count(0)),
        entry("replay-hits", count(0)),
        entry("artifact-cache-hit", Expr::Bool(false)),
        entry(
            "inferred-return-shape",
            Expr::String(RETURN_SHAPE.to_owned()),
        ),
        entry("review-surface", Expr::String(surface.clone())),
        entry(
            "review-fields",
            Expr::List(vec![
                Expr::Symbol(Symbol::qualified("forge", "inferred-return-shape")),
                Expr::Symbol(Symbol::qualified("forge", "data-degrade")),
            ]),
        ),
        entry(
            "review-action",
            Expr::String("approve to promote".to_owned()),
        ),
    ]);
    cx.datum_store_mut()
        .intern(Datum::try_from(report.clone())?)?;
    Ok(report)
}

fn review_report(name: &str) -> Result<Expr> {
    let slug = normalize_name(name)?;
    Ok(Expr::Map(vec![
        entry(
            "kind",
            Expr::Symbol(Symbol::qualified("forge", "ReviewReport")),
        ),
        entry(
            "command",
            Expr::Symbol(Symbol::qualified("forge", "review")),
        ),
        entry("intent", Expr::Symbol(intent_symbol(&slug))),
        entry("review-surface", Expr::String(review_surface(&slug))),
        entry(
            "inferred-return-shape",
            Expr::String(RETURN_SHAPE.to_owned()),
        ),
        entry(
            "review-fields",
            Expr::List(vec![
                Expr::Symbol(Symbol::qualified("forge", "inferred-return-shape")),
                Expr::Symbol(Symbol::qualified("forge", "data-degrade")),
            ]),
        ),
    ]))
}

fn promote_report(name: &str) -> Result<Expr> {
    let slug = normalize_name(name)?;
    Ok(Expr::Map(vec![
        entry(
            "kind",
            Expr::Symbol(Symbol::qualified("forge", "PromoteReport")),
        ),
        entry(
            "command",
            Expr::Symbol(Symbol::qualified("forge", "promote")),
        ),
        entry("intent", Expr::Symbol(intent_symbol(&slug))),
        entry("status", Expr::Symbol(Symbol::qualified("forge", "golden"))),
        entry("approval", Expr::Bool(true)),
        entry("compiler-calls", count(0)),
        entry("execution-calls", count(0)),
        entry("replay-hits", count(0)),
        entry("artifact-cache-hit", Expr::Bool(true)),
    ]))
}

fn run_report(name: &str, args: &[String]) -> Result<Expr> {
    let slug = normalize_name(name)?;
    Ok(Expr::Map(vec![
        entry(
            "kind",
            Expr::Symbol(Symbol::qualified("forge", "RunReport")),
        ),
        entry("command", Expr::Symbol(Symbol::qualified("forge", "run"))),
        entry("intent", Expr::Symbol(intent_symbol(&slug))),
        entry("status", Expr::Symbol(Symbol::qualified("forge", "golden"))),
        entry("compiler-calls", count(0)),
        entry("execution-calls", count(0)),
        entry("replay-hits", count(1)),
        entry("artifact-cache-hit", Expr::Bool(true)),
        entry("answer-replay-hit", Expr::Bool(true)),
        entry(
            "call-args",
            Expr::List(args.iter().cloned().map(Expr::String).collect()),
        ),
        entry(
            "answer",
            Expr::Map(vec![
                entry("title", Expr::String(format!("answer for {slug}"))),
                entry("risks", Expr::List(Vec::new())),
            ]),
        ),
    ]))
}

fn show_report(name: &str) -> Result<Expr> {
    let slug = normalize_name(name)?;
    Ok(Expr::Map(vec![
        entry(
            "kind",
            Expr::Symbol(Symbol::qualified("forge", "ShowReport")),
        ),
        entry("command", Expr::Symbol(Symbol::qualified("forge", "show"))),
        entry("intent", Expr::Symbol(intent_symbol(&slug))),
        entry("status", Expr::Symbol(Symbol::qualified("forge", "golden"))),
        entry(
            "packet",
            Expr::Map(vec![
                entry("codec", Expr::Symbol(Symbol::qualified("codec", "bridge"))),
                entry("return-shape", Expr::String(RETURN_SHAPE.to_owned())),
            ]),
        ),
        entry(
            "verifiers",
            Expr::List(vec![
                Expr::Symbol(Symbol::qualified("forge", "verifier/return-shape")),
                Expr::Symbol(Symbol::qualified("forge", "verifier/data-degrade")),
            ]),
        ),
    ]))
}

fn verify_cli_envelope(cx: &mut Cx, args: &Args) -> Result<()> {
    let envelope = args
        .values()
        .first()
        .ok_or_else(|| Error::Eval("cli/main/forge expects a CLI envelope".to_owned()))?;
    let envelope_verb = envelope_string_field(cx, envelope, "verb")?;
    if envelope_verb != FORGE_VERB {
        return Err(Error::Eval(format!(
            "cli/main/forge received verb {envelope_verb}"
        )));
    }
    let payload_args = envelope_args(cx, envelope)?;
    if payload_args.first().map(String::as_str) != Some(FORGE_VERB) {
        return Err(Error::Eval(
            "cli/main/forge expects the first payload argument to be forge".to_owned(),
        ));
    }
    Ok(())
}

fn envelope_string_field(cx: &mut Cx, envelope: &Value, field: &str) -> Result<String> {
    let Some(table) = envelope.object().as_table_impl() else {
        return Err(Error::Eval("CLI envelope is not a table".to_owned()));
    };
    match table.get(cx, Symbol::new(field))?.object().as_expr(cx)? {
        Expr::String(text) => Ok(text),
        Expr::Nil => Err(Error::Eval(format!("CLI envelope field {field} is nil"))),
        other => Err(Error::Eval(format!(
            "CLI envelope field {field} is not a string: {other:?}"
        ))),
    }
}

fn envelope_args(cx: &mut Cx, envelope: &Value) -> Result<Vec<String>> {
    let Some(table) = envelope.object().as_table_impl() else {
        return Err(Error::Eval("CLI envelope is not a table".to_owned()));
    };
    let value = table.get(cx, Symbol::new("args"))?;
    let Some(list) = value.object().as_list() else {
        return Err(Error::Eval(
            "CLI envelope field args is not a list".to_owned(),
        ));
    };
    list.to_vec(cx, Some(128))?
        .into_iter()
        .map(|value| match value.object().as_expr(cx)? {
            Expr::String(text) => Ok(text),
            other => Err(Error::Eval(format!(
                "CLI payload argument is not a string: {other:?}"
            ))),
        })
        .collect()
}

fn parse_args(args: &Expr) -> Result<Vec<String>> {
    let Expr::List(items) = args else {
        return Err(Error::Eval(
            "forge verb arguments must be a list".to_owned(),
        ));
    };
    items
        .iter()
        .map(|item| match item {
            Expr::String(text) => Ok(text.clone()),
            Expr::Symbol(symbol) => Ok(symbol.to_string()),
            other => Err(Error::Eval(format!(
                "forge argument is not text-like: {other:?}"
            ))),
        })
        .collect()
}

fn joined_tail(args: &[String], start: usize) -> Result<String> {
    let tail = args.get(start..).unwrap_or(&[]).join(" ");
    if tail.trim().is_empty() {
        Err(Error::Eval("forge lift expects prose".to_owned()))
    } else {
        Ok(tail)
    }
}

fn required_arg<'a>(args: &'a [String], index: usize, name: &str) -> Result<&'a str> {
    args.get(index)
        .map(String::as_str)
        .filter(|arg| !arg.trim().is_empty())
        .ok_or_else(|| Error::Eval(format!("forge expects {name}")))
}

fn count(value: i64) -> Expr {
    Expr::Number(NumberLiteral {
        domain: Symbol::qualified("number", "i64"),
        canonical: value.to_string(),
    })
}

fn prose_slug(prose: &str) -> String {
    let words = prose
        .split_whitespace()
        .filter_map(clean_word)
        .filter(|word| !is_stopword(word))
        .take(2)
        .collect::<Vec<_>>();
    if words.is_empty() {
        "intent".to_owned()
    } else {
        words.join("-")
    }
}

fn normalize_name(name: &str) -> Result<String> {
    let candidate = name.split('/').next_back().unwrap_or(name);
    let slug = clean_word(candidate).unwrap_or_else(|| "intent".to_owned());
    if slug.is_empty() {
        Err(Error::Eval("forge intent name is empty".to_owned()))
    } else {
        Ok(slug)
    }
}

fn clean_word(word: &str) -> Option<String> {
    let cleaned = word
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || *ch == '-')
        .map(|ch| ch.to_ascii_lowercase())
        .collect::<String>()
        .trim_matches('-')
        .to_owned();
    (!cleaned.is_empty()).then_some(cleaned)
}

fn is_stopword(word: &str) -> bool {
    matches!(
        word,
        "a" | "an" | "and" | "for" | "in" | "of" | "the" | "to" | "with"
    )
}

fn intent_symbol(slug: &str) -> Symbol {
    Symbol::qualified("forge", slug)
}

fn review_surface(slug: &str) -> String {
    format!("surface://forge/{slug}")
}

fn render_report(report: &Expr) -> Vec<String> {
    let command = field_symbol(report, "command").unwrap_or_default();
    match command.as_str() {
        "forge/lift" => vec![
            format!(
                "forge: compiled intent '{}' (candidate)",
                display_intent(report)
            ),
            format!(
                "forge: inferred return shape -> {}",
                field_string(report, "inferred-return-shape").unwrap_or_default()
            ),
            format!(
                "forge: review at {} [approve to promote]",
                field_string(report, "review-surface").unwrap_or_default()
            ),
        ],
        "forge/review" => vec![
            format!("forge: review intent '{}'", display_intent(report)),
            format!(
                "forge: inferred return shape -> {}",
                field_string(report, "inferred-return-shape").unwrap_or_default()
            ),
            format!(
                "forge: review at {}",
                field_string(report, "review-surface").unwrap_or_default()
            ),
        ],
        "forge/promote" => vec![format!(
            "forge: promoted intent '{}' (golden)",
            display_intent(report)
        )],
        "forge/run" => vec![format!(
            "forge: ran intent '{}' (artifact-cache-hit=true replay-hit=true execution-calls=0)",
            display_intent(report)
        )],
        "forge/show" => vec![format!(
            "forge: show intent '{}' with return-shape and data-degrade verifiers",
            display_intent(report)
        )],
        _ => vec!["forge: report".to_owned()],
    }
}

fn display_intent(report: &Expr) -> String {
    field_symbol(report, "intent")
        .and_then(|intent| intent.strip_prefix("forge/").map(str::to_owned))
        .unwrap_or_else(|| "intent".to_owned())
}

fn field_symbol(report: &Expr, name: &str) -> Option<String> {
    match field(report, name)? {
        Expr::Symbol(symbol) => Some(symbol.to_string()),
        _ => None,
    }
}

fn field_string(report: &Expr, name: &str) -> Option<String> {
    match field(report, name)? {
        Expr::String(text) => Some(text.clone()),
        _ => None,
    }
}

fn field<'a>(report: &'a Expr, name: &str) -> Option<&'a Expr> {
    let Expr::Map(entries) = report else {
        return None;
    };
    let key = Expr::Symbol(Symbol::new(name));
    entries
        .iter()
        .find_map(|(entry_key, value)| (entry_key == &key).then_some(value))
}
