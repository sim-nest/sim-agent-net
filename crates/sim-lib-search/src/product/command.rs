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
        )?;
        Ok(())
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
