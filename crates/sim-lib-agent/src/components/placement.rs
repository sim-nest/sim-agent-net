use super::loaded_site::LoadedSite;
use super::model::{AgentComponent, ComponentBackend};
use super::placement_cards::{ModelSiteCard, model_sites_expr};
use crate::{
    AI_RUNNER_PLACEMENT_CAPABILITY,
    util::{installed_codecs, stringish_from_value, value_from_expr},
};
use sim_kernel::{
    Args, CORE_LOCAL_EVAL_FABRIC_CLASS_ID, CapabilityName, ClassRef, Cx, Datum, DatumStore, Effect,
    Error, EvalFabric, EvalReply, EvalRequest, Expr, Object, ObjectCompat, Ref, Result, Symbol,
    Value, core_any_ref, effect,
};
use sim_lib_server::{EvalSite, ServerFrame, eval_reply_from_frame, server_frame_from_request};
use sim_lib_stream_fabric::{ContentKey, EffectLedgerCassette, EvalCassette, LedgeredRelayFabric};
use sim_value::access::field as map_field;
use std::{
    any::Any,
    collections::BTreeMap,
    sync::{Arc, LazyLock, Mutex},
};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct ModelSiteKey(String);

impl ModelSiteKey {
    pub(crate) fn new(key: String) -> Result<Self> {
        if key.is_empty() {
            return Err(Error::Eval("model site key must not be empty".to_owned()));
        }
        Ok(Self(key))
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone)]
struct ModelSiteEntry {
    card: ModelSiteCard,
    site: Arc<dyn EvalSite>,
}

#[derive(Default)]
pub(crate) struct ModelCatalog {
    sites: BTreeMap<ModelSiteKey, ModelSiteEntry>,
}

static MODEL_CATALOG: LazyLock<Mutex<ModelCatalog>> =
    LazyLock::new(|| Mutex::new(ModelCatalog::default()));

pub(crate) fn runner_place_value(cx: &mut Cx, args: Args) -> Result<Value> {
    let (key, runner, replace) = parse_runner_place_args(cx, args)?;
    let component = runner
        .object()
        .downcast_ref::<AgentComponent>()
        .ok_or_else(|| Error::Eval("runner/place expects a runner component".to_owned()))?;
    if !matches!(&component.backend, ComponentBackend::Runner(_)) {
        return Err(Error::Eval(
            "runner/place expects a runner component".to_owned(),
        ));
    }
    let key = ModelSiteKey::new(key)?;
    let card = ModelSiteCard::from_runner(&key, component)?;
    let site: Arc<dyn EvalSite> = Arc::new(component.clone());
    let card = register_model_site(cx, key, site, card, replace)?;
    card_value(cx, &card)
}

pub(crate) fn model_sites_value(cx: &mut Cx, args: Args) -> Result<Value> {
    if !args.values().is_empty() {
        return Err(Error::Eval("model/sites expects no arguments".to_owned()));
    }
    let cards = model_site_cards(cx)?;
    value_from_expr(cx, &model_sites_expr(&cards))
}

pub(crate) fn model_site_card_value(cx: &mut Cx, args: Args) -> Result<Value> {
    let key = match args.into_vec().as_slice() {
        [key] => stringish_from_value(cx, key.clone(), "model/site-card expects a model site key")?,
        _ => {
            return Err(Error::Eval(
                "model/site-card expects one model site key".to_owned(),
            ));
        }
    };
    let entry = resolve_model_site(cx, &ModelSiteKey::new(key)?)?;
    value_from_expr(cx, &entry.card.to_expr())
}

pub(crate) fn model_at_value(cx: &mut Cx, args: Args) -> Result<Value> {
    let key = match args.into_vec().as_slice() {
        [key] => stringish_from_value(cx, key.clone(), "model/at expects a model site key")?,
        _ => {
            return Err(Error::Eval(
                "model/at expects one model site key".to_owned(),
            ));
        }
    };
    cx.factory()
        .opaque(Arc::new(ModelPlacement::new(ModelSiteKey::new(key)?)))
}

pub(crate) fn model_cached_value(cx: &mut Cx, args: Args) -> Result<Value> {
    let target = match args.into_vec().as_slice() {
        [target] => target.clone(),
        _ => {
            return Err(Error::Eval(
                "model/cached expects one EvalFabric value".to_owned(),
            ));
        }
    };
    let ledger = Arc::new(EffectLedgerCassette::new());
    let cassette = Arc::new(EvalCassette::new(ledger));
    cached_model_fabric_value(cx, target, cassette)
}

pub(crate) fn cached_model_fabric_value(
    cx: &mut Cx,
    target: Value,
    cassette: Arc<EvalCassette>,
) -> Result<Value> {
    if target.object().as_eval_fabric().is_none() {
        return Err(Error::Eval(
            "model/cached expects a placement or runner EvalFabric".to_owned(),
        ));
    }
    cx.factory()
        .opaque(Arc::new(ModelCachedFabric::new(target, cassette)))
}

fn parse_runner_place_args(cx: &mut Cx, args: Args) -> Result<(String, Value, bool)> {
    match args.into_vec().as_slice() {
        [key, runner] => Ok((
            stringish_from_value(cx, key.clone(), "runner/place expects a model site key")?,
            runner.clone(),
            false,
        )),
        [key, runner, option, value] if is_replace_option(cx, option)? => Ok((
            stringish_from_value(cx, key.clone(), "runner/place expects a model site key")?,
            runner.clone(),
            boolish_from_value(cx, value.clone(), "runner/place :replace expects a boolean")?,
        )),
        [_, _, option, _] => Err(Error::Eval(format!(
            "runner/place unsupported option {:?}",
            option.object().as_expr(cx)?
        ))),
        _ => Err(Error::Eval(
            "runner/place expects a model site key and runner".to_owned(),
        )),
    }
}

fn is_replace_option(cx: &mut Cx, value: &Value) -> Result<bool> {
    match value.object().as_expr(cx)? {
        Expr::Symbol(symbol) if symbol.namespace.is_none() => {
            Ok(matches!(symbol.name.as_ref(), ":replace" | "replace"))
        }
        Expr::String(text) => Ok(matches!(text.as_str(), ":replace" | "replace")),
        _ => Ok(false),
    }
}

fn boolish_from_value(cx: &mut Cx, value: Value, context: &'static str) -> Result<bool> {
    match value.object().as_expr(cx)? {
        Expr::Bool(value) => Ok(value),
        _ => Err(Error::Eval(context.to_owned())),
    }
}

fn register_model_site(
    cx: &mut Cx,
    key: ModelSiteKey,
    site: Arc<dyn EvalSite>,
    card: ModelSiteCard,
    replace: bool,
) -> Result<ModelSiteCard> {
    let effect = model_placement_effect(cx, &key, &card, replace)?;
    let result_card = card.clone();
    let stored_key = key.clone();
    let stored_card = card.clone();
    effect::resolve_effect(cx, effect, move |cx, _effect| {
        let result = Ref::Content(
            cx.datum_store_mut()
                .intern(Datum::try_from(result_card.to_expr())?)?,
        );
        let mut catalog = catalog()?;
        if !replace && catalog.sites.contains_key(&stored_key) {
            return Err(Error::Eval(format!(
                "model site key {} is already registered",
                stored_key.as_str()
            )));
        }
        catalog.sites.insert(
            stored_key,
            ModelSiteEntry {
                card: stored_card,
                site,
            },
        );
        Ok(result)
    })?;
    Ok(card)
}

fn model_placement_effect(
    cx: &mut Cx,
    key: &ModelSiteKey,
    card: &ModelSiteCard,
    replace: bool,
) -> Result<Effect> {
    let input = Ref::Content(cx.datum_store_mut().intern(Datum::Node {
        tag: Symbol::qualified("agent", "ModelPlacementInput"),
        fields: vec![
            (Symbol::new("key"), Datum::String(key.as_str().to_owned())),
            (Symbol::new("card"), Datum::try_from(card.to_expr())?),
            (Symbol::new("replace"), Datum::Bool(replace)),
        ],
    })?);
    Effect::new(
        model_placement_effect_kind(),
        Ref::Symbol(placement_key_symbol(key.as_str())),
        input,
        core_any_ref(),
        effect::effect_resume_op_key(),
        effect::effect_abort_op_key(),
    )
    .requiring(CapabilityName::new(AI_RUNNER_PLACEMENT_CAPABILITY))
    .with_replay_key(Some(Ref::Symbol(Symbol::qualified(
        "agent",
        "model-placement-v1",
    ))))
}

fn model_placement_effect_kind() -> Symbol {
    Symbol::qualified("effect", "model-placement")
}

fn model_site_cards(cx: &Cx) -> Result<Vec<ModelSiteCard>> {
    let mut cards = BTreeMap::new();
    let codecs = installed_codecs(cx);
    for symbol in cx.registry().sites().keys() {
        let key = ModelSiteKey::new(symbol.as_qualified_str())?;
        cards.insert(
            key.clone(),
            ModelSiteCard::from_loaded(&key, symbol.to_string(), codecs.clone()),
        );
    }
    let catalog = catalog()?;
    for (key, entry) in &catalog.sites {
        cards.insert(key.clone(), entry.card.clone());
    }
    Ok(cards.into_values().collect())
}

fn resolve_model_site(cx: &Cx, key: &ModelSiteKey) -> Result<ModelSiteEntry> {
    if let Some(entry) = catalog()?.sites.get(key).cloned() {
        return Ok(entry);
    }
    if let Some(entry) = loaded_model_site_entry(cx, key)? {
        return Ok(entry);
    }
    Err(Error::Eval(format!(
        "model site key {} is not registered",
        key.as_str()
    )))
}

fn loaded_model_site_entry(cx: &Cx, key: &ModelSiteKey) -> Result<Option<ModelSiteEntry>> {
    let symbol = placement_key_symbol(key.as_str());
    let Some(value) = cx.registry().site_by_symbol(&symbol).cloned() else {
        return Ok(None);
    };
    let codecs = installed_codecs(cx);
    let site: Arc<dyn EvalSite> = Arc::new(LoadedSite::new(value, codecs.clone()));
    let card = ModelSiteCard::from_loaded(key, symbol.to_string(), codecs);
    Ok(Some(ModelSiteEntry { card, site }))
}

fn placement_key_symbol(key: &str) -> Symbol {
    match key.split_once('/') {
        Some((namespace, name)) if !namespace.is_empty() && !name.is_empty() => {
            Symbol::qualified(namespace.to_owned(), name.to_owned())
        }
        _ => Symbol::new(key.to_owned()),
    }
}

fn catalog() -> Result<std::sync::MutexGuard<'static, ModelCatalog>> {
    MODEL_CATALOG
        .lock()
        .map_err(|_| Error::Eval("model catalog lock poisoned".to_owned()))
}

fn card_value(cx: &mut Cx, card: &ModelSiteCard) -> Result<Value> {
    value_from_expr(cx, &card.to_expr())
}

pub(in crate::components) fn routed_model_site_frame(
    cx: &mut Cx,
    fallback_codec: &Symbol,
    request: &EvalRequest,
) -> Result<Option<(Arc<dyn EvalSite>, ServerFrame)>> {
    let Some(key) = routing_placement_key(&request.expr)? else {
        return Ok(None);
    };
    let entry = resolve_model_site(cx, &ModelSiteKey::new(key)?)?;
    let codec = entry
        .card
        .codecs()
        .first()
        .cloned()
        .unwrap_or_else(|| fallback_codec.clone());
    let mut routed_request = request.clone();
    routed_request.expr = without_routing_placement(&routed_request.expr);
    let routed_frame = server_frame_from_request(cx, &codec, routed_request)?;
    Ok(Some((entry.site, routed_frame)))
}

fn routing_placement_key(expr: &Expr) -> Result<Option<String>> {
    let Some(routing) = map_field(expr, "routing") else {
        return Ok(None);
    };
    let Some(placement) = map_field(routing, "placement") else {
        return Ok(None);
    };
    match placement {
        Expr::String(key) => Ok(Some(key.clone())),
        Expr::Symbol(symbol) => Ok(Some(symbol.to_string())),
        _ => Err(Error::Eval(
            "routing placement must be a model site key".to_owned(),
        )),
    }
}

fn without_routing_placement(expr: &Expr) -> Expr {
    let Expr::Map(entries) = expr else {
        return expr.clone();
    };
    Expr::Map(
        entries
            .iter()
            .filter_map(|(key, value)| {
                if symbol_key_matches(key, "routing") {
                    return without_map_field(value, "placement")
                        .map(|routing| (key.clone(), routing));
                }
                Some((key.clone(), value.clone()))
            })
            .collect(),
    )
}

fn without_map_field(expr: &Expr, field: &str) -> Option<Expr> {
    let Expr::Map(entries) = expr else {
        return Some(expr.clone());
    };
    let filtered = entries
        .iter()
        .filter(|(key, _)| !symbol_key_matches(key, field))
        .cloned()
        .collect::<Vec<_>>();
    (!filtered.is_empty()).then_some(Expr::Map(filtered))
}

fn symbol_key_matches(expr: &Expr, field: &str) -> bool {
    matches!(expr, Expr::Symbol(symbol) if symbol.name.as_ref() == field)
}

#[derive(Clone)]
struct ValueEvalFabric {
    target: Value,
}

impl ValueEvalFabric {
    fn new(target: Value) -> Self {
        Self { target }
    }
}

impl EvalFabric for ValueEvalFabric {
    fn realize(&self, cx: &mut Cx, request: EvalRequest) -> Result<EvalReply> {
        self.target
            .object()
            .as_eval_fabric()
            .ok_or_else(|| Error::Eval("cached target is not an EvalFabric".to_owned()))?
            .realize(cx, request)
    }
}

struct ModelCachedFabric {
    relay: LedgeredRelayFabric<ValueEvalFabric>,
    cassette: Arc<EvalCassette>,
}

impl ModelCachedFabric {
    fn new(target: Value, cassette: Arc<EvalCassette>) -> Self {
        Self {
            relay: LedgeredRelayFabric::new(ValueEvalFabric::new(target), cassette.clone()),
            cassette,
        }
    }

    fn mark_reply(cx: &mut Cx, mut reply: EvalReply, hit: bool) -> Result<EvalReply> {
        let expr = reply.value.object().as_expr(cx)?;
        reply.value = value_from_expr(cx, &set_cache_hit(expr, hit)?)?;
        Ok(reply)
    }
}

impl Object for ModelCachedFabric {
    fn display(&self, _cx: &mut Cx) -> Result<String> {
        Ok("#<model-cached-fabric>".to_owned())
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl ObjectCompat for ModelCachedFabric {
    fn class(&self, cx: &mut Cx) -> Result<ClassRef> {
        if let Some(value) = cx
            .registry()
            .class_by_symbol(&Symbol::qualified("core", "LocalEvalFabric"))
        {
            return Ok(value.clone());
        }
        cx.factory().class_stub(
            CORE_LOCAL_EVAL_FABRIC_CLASS_ID,
            Symbol::qualified("core", "LocalEvalFabric"),
        )
    }

    fn as_expr(&self, cx: &mut Cx) -> Result<Expr> {
        self.as_table(cx)?.object().as_expr(cx)
    }

    fn as_table(&self, cx: &mut Cx) -> Result<Value> {
        cx.factory().table(vec![
            (
                Symbol::new("kind"),
                cx.factory().symbol(Symbol::new("model-cached"))?,
            ),
            (
                Symbol::new("entries"),
                cx.factory().number_literal(
                    Symbol::qualified("numbers", "f64"),
                    self.cassette.len().to_string(),
                )?,
            ),
        ])
    }

    fn as_eval_fabric(&self) -> Option<&dyn EvalFabric> {
        Some(self)
    }
}

impl EvalFabric for ModelCachedFabric {
    fn realize(&self, cx: &mut Cx, request: EvalRequest) -> Result<EvalReply> {
        let key = ContentKey::from_request(&request);
        let hit = self.cassette.get(&key).is_some();
        let reply = self.relay.realize(cx, request)?;
        Self::mark_reply(cx, reply, hit)
    }
}

fn set_cache_hit(expr: Expr, hit: bool) -> Result<Expr> {
    let Expr::Map(mut entries) = expr else {
        return Err(Error::Eval(
            "model/cached response must be a model-response map".to_owned(),
        ));
    };
    entries.retain(|(key, _)| !symbol_key_matches(key, "cache-hit"));
    entries.push((Expr::Symbol(Symbol::new("cache-hit")), Expr::Bool(hit)));
    Ok(Expr::Map(entries))
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ModelPlacement {
    key: ModelSiteKey,
}

impl ModelPlacement {
    fn new(key: ModelSiteKey) -> Self {
        Self { key }
    }
}

impl Object for ModelPlacement {
    fn display(&self, _cx: &mut Cx) -> Result<String> {
        Ok(format!("#<model-placement {}>", self.key.as_str()))
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl ObjectCompat for ModelPlacement {
    fn class(&self, cx: &mut Cx) -> Result<ClassRef> {
        if let Some(value) = cx
            .registry()
            .class_by_symbol(&Symbol::qualified("core", "LocalEvalFabric"))
        {
            return Ok(value.clone());
        }
        cx.factory().class_stub(
            CORE_LOCAL_EVAL_FABRIC_CLASS_ID,
            Symbol::qualified("core", "LocalEvalFabric"),
        )
    }

    fn as_expr(&self, cx: &mut Cx) -> Result<Expr> {
        self.as_table(cx)?.object().as_expr(cx)
    }

    fn as_table(&self, cx: &mut Cx) -> Result<Value> {
        cx.factory().table(vec![
            (
                Symbol::new("kind"),
                cx.factory().symbol(Symbol::new("model-placement"))?,
            ),
            (
                Symbol::new("key"),
                cx.factory().string(self.key.as_str().to_owned())?,
            ),
        ])
    }

    fn as_eval_fabric(&self) -> Option<&dyn EvalFabric> {
        Some(self)
    }
}

impl EvalFabric for ModelPlacement {
    fn realize(&self, cx: &mut Cx, request: EvalRequest) -> Result<EvalReply> {
        let entry = resolve_model_site(cx, &self.key)?;
        let codec = entry.card.codecs().first().cloned().ok_or_else(|| {
            Error::Eval(format!(
                "model site key {} has no installed codec",
                self.key.as_str()
            ))
        })?;
        let frame = server_frame_from_request(cx, &codec, request)?;
        let reply = entry.site.answer(cx, frame)?;
        eval_reply_from_frame(cx, &reply)
    }
}
