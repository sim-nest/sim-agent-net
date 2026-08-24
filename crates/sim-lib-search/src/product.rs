//! Loadable CLI and Skill/MCP projection for the search behavior.

use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

use sim_kernel::{
    AbiVersion, Args, Callable, CapabilityName, Cx, Datum, Error, Export, Expr, Factory, Lib,
    LibManifest, LibTarget, Linker, LoadCx, Object, ObjectCompat, Result, ShapeRef, Symbol, Value,
    Version,
};
use sim_lib_skill::{
    FixtureSkillSpec, SkillCard, SkillEventSink, SkillRole, SkillTransport, skill_registry,
};
use sim_shape::{AnyShape, shape_value};

/// CLI verb contributed by this library.
pub const SEARCH_VERB: &str = "search";
/// Authority required by every operation that can perform HTTP egress.
pub const SEARCH_CAPABILITY: &str = "net/http";
const TRANSPORT_ID: &str = "search/product";

/// Search operation shared by CLI and projected tools.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchOperation {
    Query,
    Fetch,
    Research,
    Show,
}

impl SearchOperation {
    fn parse(value: &str) -> std::result::Result<Self, SearchProductError> {
        match value {
            "query" => Ok(Self::Query),
            "fetch" => Ok(Self::Fetch),
            "research" => Ok(Self::Research),
            "show" => Ok(Self::Show),
            _ => Err(SearchProductError::Usage(format!(
                "unknown search operation {value}"
            ))),
        }
    }
    fn name(self) -> &'static str {
        match self {
            Self::Query => "query",
            Self::Fetch => "fetch",
            Self::Research => "research",
            Self::Show => "show",
        }
    }
}

/// Network/replay selection. Fixture is a deterministic live fake.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchMode {
    Fixture,
    Cassette,
    Offline,
}

/// Fully resolved, secret-free product configuration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchConfig {
    pub mode: SearchMode,
    pub sites: Vec<String>,
    pub safe_search: String,
    pub category: Option<String>,
    pub language: Option<String>,
    pub time_range: Option<String>,
    pub page_limit: u32,
    pub call_budget: u32,
    pub byte_budget: usize,
    pub egress_zone: String,
    pub cache_mode: String,
    pub store: String,
    pub principal_ref: Option<String>,
    pub granted_http: bool,
}

impl Default for SearchConfig {
    fn default() -> Self {
        Self {
            mode: SearchMode::Offline,
            sites: vec!["fixture/local".into()],
            safe_search: "moderate".into(),
            category: None,
            language: None,
            time_range: None,
            page_limit: 1,
            call_budget: 4,
            byte_budget: 262_144,
            egress_zone: "public-web".into(),
            cache_mode: "prefer".into(),
            store: ".sim/search".into(),
            principal_ref: None,
            granted_http: false,
        }
    }
}

/// Canonical Shape record returned identically through CLI JSON and Skill/MCP.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchRecord {
    pub id: String,
    pub datum: Datum,
}

impl SearchRecord {
    /// Canonical JSON projection; it is a projection of `datum`, not a CLI DTO.
    pub fn json(&self) -> String {
        datum_json(&self.datum)
    }
    /// Human rendering keeps epistemic and failure labels distinct.
    pub fn human(&self) -> String {
        format!(
            "SearchRun: {}\nprovider claim: fixture result (unverified)\ncapture: capture:{}\npolicy/robots: allowed fixture/no-network\nalias evidence: none\nrank contribution: fixture=1\ncitation: representation:{}#text=SIM\nfidelity warning: fixture content\npartial failure: none\n",
            self.id,
            &self.id[..16],
            &self.id[..16]
        )
    }
}

/// Product-level validation or execution failure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SearchProductError {
    Usage(String),
    Capability(String),
    Config(String),
    NotFound(String),
}
impl std::fmt::Display for SearchProductError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for SearchProductError {}

/// One execution/store shared by CLI and Skill/MCP projections.
#[derive(Clone, Default)]
pub struct SearchProduct {
    records: Arc<Mutex<BTreeMap<String, SearchRecord>>>,
}

impl SearchProduct {
    /// Executes one operation. Show is strictly store-only.
    pub fn execute(
        &self,
        operation: SearchOperation,
        input: &str,
        config: &SearchConfig,
    ) -> std::result::Result<SearchRecord, SearchProductError> {
        if operation == SearchOperation::Show {
            return self
                .records
                .lock()
                .map_err(|_| SearchProductError::Config("search store poisoned".into()))?
                .get(input)
                .cloned()
                .ok_or_else(|| SearchProductError::NotFound(input.into()));
        }
        if input.trim().is_empty() {
            return Err(SearchProductError::Usage(
                "query or URI must not be empty".into(),
            ));
        }
        if config.page_limit == 0 || config.call_budget == 0 || config.byte_budget == 0 {
            return Err(SearchProductError::Config(
                "budgets must be positive".into(),
            ));
        }
        if matches!(config.mode, SearchMode::Fixture) && !config.granted_http {
            return Err(SearchProductError::Capability(
                "missing net/http capability; no socket opened".into(),
            ));
        }
        let mode = match config.mode {
            SearchMode::Fixture => "live-fake",
            SearchMode::Cassette => "cassette",
            SearchMode::Offline => "offline",
        };
        let datum = node(
            "search/Run",
            vec![
                field("operation", Datum::Symbol(Symbol::new(operation.name()))),
                field("input", Datum::String(input.into())),
                field("mode", Datum::Symbol(Symbol::new(mode))),
                field(
                    "sites",
                    Datum::Vector(config.sites.iter().cloned().map(Datum::String).collect()),
                ),
                field("safe-search", Datum::String(config.safe_search.clone())),
                field(
                    "language",
                    config.language.clone().map_or(Datum::Nil, Datum::String),
                ),
                field(
                    "category",
                    config.category.clone().map_or(Datum::Nil, Datum::String),
                ),
                field(
                    "time",
                    config.time_range.clone().map_or(Datum::Nil, Datum::String),
                ),
                field("egress-zone", Datum::String(config.egress_zone.clone())),
                field("cache", Datum::String(config.cache_mode.clone())),
                field("store", Datum::String(config.store.clone())),
                field(
                    "principal",
                    config
                        .principal_ref
                        .clone()
                        .map_or(Datum::Nil, Datum::String),
                ),
                field(
                    "provider-claims",
                    Datum::Vector(vec![node(
                        "search/ProviderClaim",
                        vec![
                            field("provider", Datum::String("fixture".into())),
                            field(
                                "title",
                                Datum::String("SIM deterministic search fixture".into()),
                            ),
                        ],
                    )]),
                ),
                field(
                    "captures",
                    Datum::Vector(vec![Datum::String("fixture:capture".into())]),
                ),
                field(
                    "policy",
                    Datum::String("robots=allowed; network=fixture-only".into()),
                ),
                field(
                    "rank",
                    Datum::Vector(vec![Datum::String("fixture=1".into())]),
                ),
                field(
                    "citations",
                    Datum::Vector(vec![Datum::String("fixture:capture#text=SIM".into())]),
                ),
                field(
                    "warnings",
                    Datum::Vector(vec![Datum::String("fixture fidelity".into())]),
                ),
                field("failures", Datum::Vector(Vec::new())),
            ],
        );
        let content_id = datum
            .content_id()
            .map_err(|e| SearchProductError::Config(e.to_string()))?;
        let id = format!(
            "{}:{}",
            content_id.algorithm,
            content_id
                .bytes
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        );
        let record = SearchRecord {
            id: id.clone(),
            datum,
        };
        self.records
            .lock()
            .map_err(|_| SearchProductError::Config("search store poisoned".into()))?
            .insert(id, record.clone());
        Ok(record)
    }
}

/// Exact stable input Shape identity used by both projections.
pub fn search_input_shape() -> ShapeRef {
    shape_value(
        Symbol::qualified("search", "OperationInput-v1"),
        Arc::new(AnyShape),
    )
}
/// Exact stable output Shape identity used by both projections.
pub fn search_output_shape() -> ShapeRef {
    shape_value(
        Symbol::qualified("search", "SearchRun-v1"),
        Arc::new(AnyShape),
    )
}

/// Installs the four search Cards into the existing Skill registry.
pub fn install_search_skill(cx: &mut Cx, product: SearchProduct) -> Result<Vec<SkillCard>> {
    let registry = skill_registry(cx)?;
    let transport = Arc::new(SearchSkillTransport { product });
    registry.install_transport(transport)?;
    let mut cards = Vec::new();
    for operation in [
        SearchOperation::Query,
        SearchOperation::Fetch,
        SearchOperation::Research,
        SearchOperation::Show,
    ] {
        let id = format!("search/{}", operation.name());
        let mut card = SkillCard::fixture(FixtureSkillSpec {
            id: id.clone(),
            symbol: Symbol::qualified("search", operation.name()),
            title: format!("Search {}", operation.name()),
            description: "Canonical search operation; returns search/SearchRun-v1".into(),
            input_shape: search_input_shape(),
            output_shape: search_output_shape(),
            transport_id: TRANSPORT_ID.into(),
            operation: operation.name().into(),
        })
        .with_role(SkillRole::Retriever);
        card.capabilities.clear();
        if operation != SearchOperation::Show {
            card.capabilities
                .push(CapabilityName::new(SEARCH_CAPABILITY));
        }
        registry.bind_card(cx, card.clone())?;
        cards.push(card);
    }
    Ok(cards)
}

struct SearchSkillTransport {
    product: SearchProduct,
}
impl SkillTransport for SearchSkillTransport {
    fn id(&self) -> &str {
        TRANSPORT_ID
    }
    fn kind(&self) -> &str {
        "search"
    }
    fn discover(&self, _cx: &mut Cx) -> Result<Vec<SkillCard>> {
        Ok(Vec::new())
    }
    fn call(
        &self,
        cx: &mut Cx,
        card: &SkillCard,
        args: Value,
        _events: Option<&mut dyn SkillEventSink>,
    ) -> Result<Value> {
        let operation = SearchOperation::parse(&card.operation).map_err(product_error)?;
        let input = match args.object().as_expr(cx)? {
            Expr::String(s) => s,
            Expr::List(items) => items
                .first()
                .and_then(|v| {
                    if let Expr::String(s) = v {
                        Some(s.clone())
                    } else {
                        None
                    }
                })
                .ok_or_else(|| Error::Eval("search input must contain a string".into()))?,
            _ => return Err(Error::Eval("search input must be a string".into())),
        };
        let mut config = SearchConfig::default();
        config.mode = SearchMode::Fixture;
        config.granted_http = operation == SearchOperation::Show
            || cx
                .capabilities()
                .contains(&CapabilityName::new(SEARCH_CAPABILITY));
        let record = self
            .product
            .execute(operation, &input, &config)
            .map_err(product_error)?;
        cx.factory().make(Expr::from(record.datum))
    }
    fn health(&self, cx: &mut Cx) -> Result<Value> {
        cx.factory().bool(true)
    }
}

/// Loadable CLI library. Concrete behavior remains here, outside the `sim` frame.
#[derive(Clone, Default)]
pub struct SearchCommandLib {
    product: SearchProduct,
}
impl SearchCommandLib {
    pub fn new() -> Self {
        Self::default()
    }
}
impl Lib for SearchCommandLib {
    fn manifest(&self) -> LibManifest {
        LibManifest {
            id: Symbol::qualified("lib", "search-command"),
            version: Version(env!("CARGO_PKG_VERSION").into()),
            abi: AbiVersion { major: 0, minor: 1 },
            target: LibTarget::HostRegistered,
            requires: vec![],
            capabilities: vec![],
            exports: vec![Export::Function {
                symbol: sim_run_core::cli_main_entrypoint_symbol(SEARCH_VERB),
                function_id: None,
            }],
        }
    }
    fn load(&self, cx: &mut LoadCx, linker: &mut Linker<'_>) -> Result<()> {
        linker.function_value(
            sim_run_core::cli_main_entrypoint_symbol(SEARCH_VERB),
            cx.factory().opaque(Arc::new(SearchEntrypoint {
                product: self.product.clone(),
            }))?,
        )
    }
}

#[derive(Clone)]
struct SearchEntrypoint {
    product: SearchProduct,
}
impl Object for SearchEntrypoint {
    fn display(&self, _: &mut Cx) -> Result<String> {
        Ok("cli/main/search".into())
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}
impl ObjectCompat for SearchEntrypoint {
    fn as_callable(&self) -> Option<&dyn Callable> {
        Some(self)
    }
}
impl Callable for SearchEntrypoint {
    fn call(&self, cx: &mut Cx, args: Args) -> Result<Value> {
        let argv = envelope_args(cx, args.values().first())?;
        if argv.len() < 3 {
            return Err(Error::Eval(
                "usage: sim search query|fetch|research|show INPUT [OPTIONS]".into(),
            ));
        }
        let operation = SearchOperation::parse(&argv[1]).map_err(product_error)?;
        let (config, json) = parse_config(&argv[3..])?;
        let record = self
            .product
            .execute(operation, &argv[2], &config)
            .map_err(product_error)?;
        if json {
            println!("{}", record.json());
        } else {
            print!("{}", record.human());
        }
        cx.factory().bool(true)
    }
}

fn parse_config(args: &[String]) -> Result<(SearchConfig, bool)> {
    let mut c = SearchConfig::default();
    let mut json = false;
    let mut i = 0;
    while i < args.len() {
        let take = |i: &mut usize| -> Result<String> {
            *i += 1;
            args.get(*i)
                .cloned()
                .ok_or_else(|| Error::Eval("missing option value".into()))
        };
        match args[i].as_str() {
            "--json" => json = true,
            "--live-fake" => c.mode = SearchMode::Fixture,
            "--cassette" => c.mode = SearchMode::Cassette,
            "--offline" | "--replay" => c.mode = SearchMode::Offline,
            "--allow-net-http" => c.granted_http = true,
            "--site" => c.sites.push(take(&mut i)?),
            "--safe-search" => c.safe_search = take(&mut i)?,
            "--category" => c.category = Some(take(&mut i)?),
            "--language" => c.language = Some(take(&mut i)?),
            "--time" => c.time_range = Some(take(&mut i)?),
            "--egress-zone" => c.egress_zone = take(&mut i)?,
            "--cache" => c.cache_mode = take(&mut i)?,
            "--store" => c.store = take(&mut i)?,
            "--principal" => c.principal_ref = Some(take(&mut i)?),
            "--pages" => {
                c.page_limit = take(&mut i)?
                    .parse()
                    .map_err(|_| Error::Eval("invalid --pages".into()))?
            }
            "--calls" => {
                c.call_budget = take(&mut i)?
                    .parse()
                    .map_err(|_| Error::Eval("invalid --calls".into()))?
            }
            "--bytes" => {
                c.byte_budget = take(&mut i)?
                    .parse()
                    .map_err(|_| Error::Eval("invalid --bytes".into()))?
            }
            other => return Err(Error::Eval(format!("unknown search option {other}"))),
        }
        i += 1;
    }
    Ok((c, json))
}
fn envelope_args(cx: &mut Cx, envelope: Option<&Value>) -> Result<Vec<String>> {
    let e = envelope.ok_or_else(|| Error::Eval("missing search envelope".into()))?;
    let t = e
        .object()
        .as_table_impl()
        .ok_or_else(|| Error::Eval("search envelope is not a table".into()))?;
    let v = t.get(cx, Symbol::new("args"))?;
    let Expr::List(xs) = v.object().as_expr(cx)? else {
        return Err(Error::Eval("search args are not a list".into()));
    };
    xs.into_iter()
        .map(|x| {
            if let Expr::String(s) = x {
                Ok(s)
            } else {
                Err(Error::Eval("search arg is not a string".into()))
            }
        })
        .collect()
}
fn product_error(e: SearchProductError) -> Error {
    Error::Eval(e.to_string())
}
fn field(name: &str, value: Datum) -> (Symbol, Datum) {
    (Symbol::new(name), value)
}
fn node(tag: &str, fields: Vec<(Symbol, Datum)>) -> Datum {
    Datum::Node {
        tag: Symbol::new(tag),
        fields,
    }
}
fn esc(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
}
fn datum_json(d: &Datum) -> String {
    match d {
        Datum::Nil => "null".into(),
        Datum::Bool(v) => v.to_string(),
        Datum::String(v) => format!("\"{}\"", esc(v)),
        Datum::Symbol(v) => format!("\"{}\"", esc(&v.to_string())),
        Datum::Vector(v) | Datum::List(v) => format!(
            "[{}]",
            v.iter().map(datum_json).collect::<Vec<_>>().join(",")
        ),
        Datum::Node { tag, fields } => {
            let mut rows = vec![format!("\"$shape\":\"{}\"", esc(&tag.to_string()))];
            rows.extend(
                fields
                    .iter()
                    .map(|(k, v)| format!("\"{}\":{}", esc(&k.to_string()), datum_json(v))),
            );
            format!("{{{}}}", rows.join(","))
        }
        other => format!("\"{}\"", esc(&format!("{other:?}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cli_and_mcp_share_stable_record_and_show_is_network_free() {
        let p = SearchProduct::default();
        let mut c = SearchConfig::default();
        c.mode = SearchMode::Fixture;
        c.granted_http = true;
        let a = p.execute(SearchOperation::Query, "sim", &c).unwrap();
        let b = p.execute(SearchOperation::Query, "sim", &c).unwrap();
        assert_eq!(a, b);
        let mut denied = c.clone();
        denied.granted_http = false;
        assert!(matches!(
            p.execute(SearchOperation::Fetch, "x", &denied),
            Err(SearchProductError::Capability(_))
        ));
        assert_eq!(p.execute(SearchOperation::Show, &a.id, &denied).unwrap(), a);
    }
    #[test]
    fn modes_and_partial_labels_are_canonical() {
        let p = SearchProduct::default();
        for mode in [
            SearchMode::Fixture,
            SearchMode::Cassette,
            SearchMode::Offline,
        ] {
            let mut c = SearchConfig::default();
            c.mode = mode;
            c.granted_http = true;
            let r = p.execute(SearchOperation::Research, "sim", &c).unwrap();
            assert!(r.json().contains("search/Run"));
            assert!(r.human().contains("partial failure:"));
        }
    }
}
