//! Shape-scoped retrieval over cached FORGE contract decks.

use std::collections::BTreeMap;

use sim_kernel::{
    Cx, Error, Expr, Result, ShapeRef, Symbol, Value,
    library::{ExportRecord, LoadedLib},
};
use sim_shape::{Shape, ShapeRelationKind, relate_shapes};

use crate::contracts::{assemble_contract_deck, export_value};
use crate::{ContractCard, ContractDeck};

/// Shape filters for contract-card retrieval.
#[derive(Clone)]
pub struct ShapeQuery {
    /// Wanted callable argument Shape. Candidate arguments must subsume it.
    pub args: Option<ShapeRef>,
    /// Wanted callable result Shape. Candidate results must be subshapes of it.
    pub result: Option<ShapeRef>,
    /// Maximum number of ranked cards returned. `0` means no cards are returned.
    pub limit: usize,
}

/// One contract card paired with its ranking score and explanation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RankedContractCard {
    /// The matched source-free contract card.
    pub card: ContractCard,
    /// Higher scores sort before lower scores.
    pub score: i32,
    /// Human-readable facts that contributed to the score.
    pub reasons: Vec<String>,
}

/// Counters and query facts from the most recent contract-deck query.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ContractQueryReport {
    /// Whether the deck came from an existing cache entry.
    pub cache_hit: bool,
    /// Cards excluded because a requested shape field was unavailable.
    pub skipped_missing_shapes: usize,
    /// Number of matching cards omitted because of the query limit.
    pub capped_results: usize,
    /// Number of cards that matched before applying the query limit.
    pub matched_before_limit: usize,
}

/// Cached runtime contract deck keyed by a cheap registry generation marker.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ContractDeckCache {
    generation: Option<RegistryGeneration>,
    deck: ContractDeck,
    shape_index: BTreeMap<ContractCardKey, CardShapes>,
    hits: usize,
    misses: usize,
    last_report: ContractQueryReport,
}

impl ContractDeckCache {
    /// Creates an empty contract-deck cache.
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of queries served from an unchanged registry generation.
    pub fn hits(&self) -> usize {
        self.hits
    }

    /// Number of times the deck was rebuilt for a new registry generation.
    pub fn misses(&self) -> usize {
        self.misses
    }

    /// Number of cards currently held by the cached deck.
    pub fn cached_card_count(&self) -> usize {
        self.deck.cards.len()
    }

    /// Facts recorded for the most recent query.
    pub fn last_report(&self) -> &ContractQueryReport {
        &self.last_report
    }
}

/// Query a cached FORGE contract deck by argument and result Shape.
///
/// Argument matching is contravariant: a candidate callable must accept at
/// least the requested arguments. Result matching is covariant: a candidate
/// callable must return a shape contained by the requested result shape.
pub fn query_contract_deck(
    cx: &mut Cx,
    cache: &mut ContractDeckCache,
    query: &ShapeQuery,
) -> Result<Vec<RankedContractCard>> {
    let cache_hit = ensure_cached_deck(cx, cache)?;
    let mut skipped_missing_shapes = 0;
    let mut ranked = Vec::new();

    for card in &cache.deck.cards {
        let key = ContractCardKey::from_card(card);
        let shapes = cache.shape_index.get(&key);
        let mut score = 0;
        let mut reasons = Vec::new();

        if let Some(wanted) = &query.args {
            let Some(candidate) = shapes.and_then(|shapes| shapes.args.as_ref()) else {
                skipped_missing_shapes += 1;
                continue;
            };
            let exact = shape_field_exact(cx, card.args_shape.as_ref(), wanted)?;
            let Some(points) = shape_relation_score(
                cx,
                candidate,
                wanted,
                QueryRelation::Subsumes,
                exact,
                "args",
            )?
            else {
                continue;
            };
            score += points.score;
            reasons.extend(points.reasons);
        }

        if let Some(wanted) = &query.result {
            let Some(candidate) = shapes.and_then(|shapes| shapes.result.as_ref()) else {
                skipped_missing_shapes += 1;
                continue;
            };
            let exact = shape_field_exact(cx, card.result_shape.as_ref(), wanted)?;
            let Some(points) = shape_relation_score(
                cx,
                candidate,
                wanted,
                QueryRelation::SubshapeOf,
                exact,
                "result",
            )?
            else {
                continue;
            };
            score += points.score;
            reasons.extend(points.reasons);
        }

        if query.args.is_none() && query.result.is_none() {
            reasons.push("unfiltered".to_owned());
        }

        ranked.push(RankedContractCard {
            card: card.clone(),
            score,
            reasons,
        });
    }

    ranked.sort_by(|left, right| {
        right
            .score
            .cmp(&left.score)
            .then_with(|| left.card.lib.cmp(&right.card.lib))
            .then_with(|| left.card.symbol.cmp(&right.card.symbol))
            .then_with(|| left.card.export_kind.cmp(&right.card.export_kind))
    });

    let matched_before_limit = ranked.len();
    let capped_results = matched_before_limit.saturating_sub(query.limit);
    ranked.truncate(query.limit);
    cache.last_report = ContractQueryReport {
        cache_hit,
        skipped_missing_shapes,
        capped_results,
        matched_before_limit,
    };

    Ok(ranked)
}

fn ensure_cached_deck(cx: &mut Cx, cache: &mut ContractDeckCache) -> Result<bool> {
    let generation = registry_generation(cx);
    if cache.generation == Some(generation) {
        cache.hits += 1;
        return Ok(true);
    }

    cache.deck = assemble_contract_deck(cx)?;
    cache.shape_index = collect_contract_shapes(cx)?;
    cache.generation = Some(generation);
    cache.misses += 1;
    Ok(false)
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct RegistryGeneration {
    lib_count: usize,
    export_count: usize,
    fingerprint: u64,
}

fn registry_generation(cx: &Cx) -> RegistryGeneration {
    let mut marker = RegistryGeneration {
        lib_count: 0,
        export_count: 0,
        fingerprint: FNV_OFFSET,
    };
    for loaded in cx.registry().libs() {
        marker.lib_count += 1;
        marker.export_count += loaded.exports.len();
        mix_loaded_lib(&mut marker.fingerprint, loaded);
    }
    marker
}

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

fn mix_loaded_lib(hash: &mut u64, loaded: &LoadedLib) {
    mix_u64(hash, loaded.id.0 as u64);
    mix_symbol(hash, &loaded.manifest.id);
    mix_bytes(hash, loaded.manifest.version.0.as_bytes());
    mix_u64(hash, loaded.trusted as u64);
    for export in &loaded.exports {
        mix_export(hash, export);
    }
}

fn mix_export(hash: &mut u64, export: &ExportRecord) {
    mix_symbol(hash, export.kind.symbol());
    mix_symbol(hash, &export.symbol);
    mix_bytes(hash, format!("{:?}", export.state).as_bytes());
}

fn mix_symbol(hash: &mut u64, symbol: &Symbol) {
    mix_bytes(hash, symbol.as_qualified_str().as_bytes());
}

fn mix_u64(hash: &mut u64, value: u64) {
    mix_bytes(hash, &value.to_le_bytes());
}

fn mix_bytes(hash: &mut u64, bytes: &[u8]) {
    for byte in bytes {
        *hash ^= u64::from(*byte);
        *hash = hash.wrapping_mul(FNV_PRIME);
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct ContractCardKey {
    lib: Symbol,
    export_kind: Symbol,
    symbol: Symbol,
}

impl ContractCardKey {
    fn from_card(card: &ContractCard) -> Self {
        Self {
            lib: card.lib.clone(),
            export_kind: card.export_kind.clone(),
            symbol: card.symbol.clone(),
        }
    }

    fn from_export(loaded: &LoadedLib, export: &ExportRecord) -> Self {
        Self {
            lib: loaded.manifest.id.clone(),
            export_kind: export.kind.symbol().clone(),
            symbol: export.symbol.clone(),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct CardShapes {
    args: Option<ShapeRef>,
    result: Option<ShapeRef>,
}

fn collect_contract_shapes(cx: &mut Cx) -> Result<BTreeMap<ContractCardKey, CardShapes>> {
    let loaded_libs = cx.registry().libs().to_vec();
    let mut index = BTreeMap::new();

    for loaded in loaded_libs {
        for export in &loaded.exports {
            let Some(value) = export_value(cx, export) else {
                continue;
            };
            let Some(callable) = value.object().as_callable() else {
                continue;
            };
            let args = callable.browse_args_shape(cx)?.filter(value_is_shape);
            let result = callable.browse_result_shape(cx)?.filter(value_is_shape);
            index.insert(
                ContractCardKey::from_export(&loaded, export),
                CardShapes { args, result },
            );
        }
    }

    Ok(index)
}

fn value_is_shape(value: &Value) -> bool {
    value.object().as_shape().is_some()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum QueryRelation {
    Subsumes,
    SubshapeOf,
}

fn query_relation_matches(
    cx: &mut Cx,
    candidate: &dyn Shape,
    wanted: &dyn Shape,
    relation: QueryRelation,
) -> Result<bool> {
    let relation_kind = relate_shapes(cx, candidate, wanted, &[])?.kind;
    Ok(match relation {
        QueryRelation::Subsumes => matches!(
            relation_kind,
            ShapeRelationKind::Equal | ShapeRelationKind::RightSubshape
        ),
        QueryRelation::SubshapeOf => matches!(
            relation_kind,
            ShapeRelationKind::Equal | ShapeRelationKind::LeftSubshape
        ),
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ShapeScore {
    score: i32,
    reasons: Vec<String>,
}

fn shape_relation_score(
    cx: &mut Cx,
    candidate: &ShapeRef,
    wanted: &ShapeRef,
    relation: QueryRelation,
    exact: bool,
    label: &str,
) -> Result<Option<ShapeScore>> {
    let candidate = shape_ref_as_shape("candidate", candidate)?;
    let wanted = shape_ref_as_shape("wanted", wanted)?;
    if !query_relation_matches(cx, candidate, wanted, relation)? {
        return Ok(None);
    }

    let relation_kind = relate_shapes(cx, candidate, wanted, &[])?.kind;
    let (mut score, relation_reason) = match (relation, relation_kind) {
        (_, ShapeRelationKind::Equal) => (120, format!("{label} exact")),
        (QueryRelation::Subsumes, ShapeRelationKind::RightSubshape) => {
            (80, format!("{label} subsumes query"))
        }
        (QueryRelation::SubshapeOf, ShapeRelationKind::LeftSubshape) => {
            (90, format!("{label} narrows query"))
        }
        _ => return Ok(None),
    };
    let mut reasons = vec![relation_reason];
    if exact {
        score += 10;
        reasons.push(format!("{label} field exact"));
    }
    Ok(Some(ShapeScore { score, reasons }))
}

fn shape_ref_as_shape<'a>(label: &str, value: &'a ShapeRef) -> Result<&'a dyn Shape> {
    value
        .object()
        .as_shape()
        .ok_or_else(|| Error::Eval(format!("{label} ShapeQuery value is not a Shape")))
}

fn shape_field_exact(cx: &mut Cx, candidate: Option<&Expr>, wanted: &ShapeRef) -> Result<bool> {
    let Some(candidate) = candidate else {
        return Ok(false);
    };
    Ok(candidate == &wanted.object().as_expr(cx)?)
}
