fn decode(error: SearchError) -> SearchHttpError {
    SearchHttpError::Decode(error.to_string())
}

trait SearchPolicyExt {
    fn with_search_defaults(self) -> Self;
}
impl SearchPolicyExt for SkillPolicy {
    fn with_search_defaults(mut self) -> Self {
        self.idempotent = true;
        self.cache = SkillCacheMode::ReadThrough;
        self.cassette = SkillCassetteMode::RecordReplay;
        self
    }
}

impl<C: SearchWireCodec + Send + Sync> SkillTransport for HttpSearchTransport<C> {
    fn id(&self) -> &str {
        &self.id
    }
    fn kind(&self) -> &str {
        "search-http"
    }
    fn discover(&self, _cx: &mut Cx) -> SimResult<Vec<SkillCard>> {
        Ok(vec![self.card()])
    }
    fn call(
        &self,
        cx: &mut Cx,
        card: &SkillCard,
        args: Value,
        _events: Option<&mut dyn SkillEventSink>,
    ) -> SimResult<Value> {
        if card.operation != "search" {
            return Err(Error::Eval("unsupported search site operation".into()));
        }
        let query = query_from_expr(args.object().as_expr(cx)?)?;
        let receipt = self
            .search(query, CallMode::Live)
            .map_err(|e| Error::Eval(e.to_string()))?;
        page_value(
            cx,
            receipt
                .pages
                .last()
                .ok_or_else(|| Error::Eval("search returned no page".into()))?,
        )
    }
    fn health(&self, cx: &mut Cx) -> SimResult<Value> {
        cx.factory().expr(datum_expr(&self.health_observation()))
    }
}

fn query_shape(id: &str) -> sim_kernel::ShapeRef {
    shape_value(
        Symbol::qualified("search-http", format!("{id}-query")),
        Arc::new(ListShape::new(vec![Arc::new(FieldShape::anonymous(vec![
            FieldSpec::required(
                Symbol::new("text"),
                Arc::new(ExprKindShape::new(ExprKind::String)),
            ),
            FieldSpec::required(
                Symbol::new("limit"),
                Arc::new(ExprKindShape::new(ExprKind::Number)),
            ),
        ]))])),
    )
}
fn page_shape(id: &str) -> sim_kernel::ShapeRef {
    shape_value(
        Symbol::qualified("search-http", format!("{id}-page")),
        Arc::new(FieldShape::anonymous(vec![FieldSpec::required(
            Symbol::new("observations"),
            Arc::new(ListShape::new(vec![Arc::new(AnyShape)])),
        )])),
    )
}
fn query_from_expr(expr: Expr) -> SimResult<SearchQuery> {
    let Expr::List(mut args) = expr else {
        return Err(Error::Eval("search expects one query table".into()));
    };
    if args.len() != 1 {
        return Err(Error::Eval("search expects one query table".into()));
    }
    let Expr::Map(fields) = args.remove(0) else {
        return Err(Error::Eval("search query must be a table".into()));
    };
    let get = |name: &str| {
        fields
            .iter()
            .find_map(|(k, v)| matches!(k, Expr::Symbol(s) if s.name.as_ref() == name).then_some(v))
    };
    let text = match get("text") {
        Some(Expr::String(s)) => s.clone(),
        _ => return Err(Error::Eval("search query text must be a string".into())),
    };
    let limit = match get("limit") {
        Some(Expr::Number(n)) => n
            .canonical
            .parse()
            .map_err(|_| Error::Eval("search limit must be u32".into()))?,
        _ => return Err(Error::Eval("search query limit must be a number".into())),
    };
    SearchQuery::checked(text, Vec::<SearchSite>::new(), None, limit)
        .map_err(|e| Error::Eval(e.to_string()))
}
fn page_value(cx: &mut Cx, page: &SearchPage) -> SimResult<Value> {
    let observations = cx.factory().list(
        page.observations
            .iter()
            .map(|o| cx.factory().expr(observation_expr(o)))
            .collect::<SimResult<Vec<_>>>()?,
    )?;
    cx.factory().table(vec![
        (Symbol::new("observations"), observations),
        (
            Symbol::new("continuation"),
            match &page.continuation {
                Some(v) => cx.factory().string(v.clone())?,
                None => cx.factory().nil()?,
            },
        ),
    ])
}
fn observation_expr(o: &SearchObservation) -> Expr {
    Expr::Map(vec![
        (
            Expr::Symbol(Symbol::new("retrieval-uri")),
            Expr::String(o.retrieval_uri.clone()),
        ),
        (
            Expr::Symbol(Symbol::new("claim")),
            o.claim.as_ref().map(claim_expr).unwrap_or(Expr::Nil),
        ),
        (
            Expr::Symbol(Symbol::new("capture-id")),
            o.capture_id
                .as_ref()
                .map(|id| Expr::String(id.bytes.iter().map(|byte| format!("{byte:02x}")).collect()))
                .unwrap_or(Expr::Nil),
        ),
    ])
}
fn claim_expr(c: &ProviderClaim) -> Expr {
    Expr::Map(vec![
        (
            Expr::Symbol(Symbol::new("provider")),
            Expr::String(c.provider.clone()),
        ),
        (
            Expr::Symbol(Symbol::new("uri")),
            Expr::String(c.uri.clone()),
        ),
        (
            Expr::Symbol(Symbol::new("title")),
            c.title.clone().map(Expr::String).unwrap_or(Expr::Nil),
        ),
        (
            Expr::Symbol(Symbol::new("snippet")),
            c.snippet.clone().map(Expr::String).unwrap_or(Expr::Nil),
        ),
    ])
}
fn datum_expr(d: &Datum) -> Expr {
    match d {
        Datum::Nil => Expr::Nil,
        Datum::Bool(v) => Expr::Bool(*v),
        Datum::String(v) => Expr::String(v.clone()),
        Datum::Bytes(v) => Expr::Bytes(v.clone()),
        Datum::Symbol(v) => Expr::Symbol(v.clone()),
        Datum::Number(v) => Expr::Number(v.clone()),
        Datum::List(v) => Expr::List(v.iter().map(datum_expr).collect()),
        Datum::Vector(v) => Expr::Vector(v.iter().map(datum_expr).collect()),
        Datum::Set(v) => Expr::Set(v.iter().map(datum_expr).collect()),
        Datum::Map(v) => Expr::Map(
            v.iter()
                .map(|(k, v)| (datum_expr(k), datum_expr(v)))
                .collect(),
        ),
        Datum::Node { tag, fields } => Expr::Map(
            std::iter::once((Expr::Symbol(Symbol::new("kind")), Expr::Symbol(tag.clone())))
                .chain(
                    fields
                        .iter()
                        .map(|(k, v)| (Expr::Symbol(k.clone()), datum_expr(v))),
                )
                .collect(),
        ),
    }
}
