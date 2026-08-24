//! Deterministic federation of provider-neutral search skills.
//!
//! This crate plans first, dispatches within explicit bounds, and records every
//! omission and policy decision. It never owns provider syntax, HTTP policy,
//! rank mathematics, page decoding, or prose answer generation.
#![forbid(unsafe_code)]
mod product;

pub use product::{
    SEARCH_CAPABILITY, SEARCH_VERB, SearchCommandLib, SearchConfig, SearchMode, SearchOperation,
    SearchProduct, SearchProductError, SearchRecord, install_search_skill, search_input_shape,
    search_output_shape,
};

use sim_kernel::{CapabilityName, ContentId, Datum, ShapeRef};
use sim_lib_agent_runner_core::fenced_data_text_for_id;
use sim_lib_rank::{
    EmbeddingIndex, FusionLimits, RankLimits, RankedFusion, RankedList, reciprocal_rank_fusion,
    retrieve_limited,
};
use sim_lib_search_core::{
    AliasEvidence, Citation, ProviderClaim, SearchObservation, SearchPage, SearchQuery,
};
use sim_lib_skill::{SkillCacheMode, SkillCard, SkillRole};
use sim_lib_web_core::{EvidenceSelector, WebRepresentation};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
};

/// Network-free cookbook descriptors embedded for discovery.
pub static RECIPES: sim_cookbook::EmbeddedDir =
    include!(concat!(env!("OUT_DIR"), "/cookbook_recipes.rs"));

/// A stable selected retriever in an immutable plan.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlannedSite {
    pub card_id: String,
    pub transport_id: String,
    pub concurrent_safe: bool,
    pub cache_mode: SkillCacheMode,
}

/// All authority and resource choices frozen before dispatch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchPlan {
    pub sites: Vec<PlannedSite>,
    pub max_pages_per_site: usize,
    pub deadline_millis: u64,
    pub concurrency: usize,
    pub total_calls: usize,
    pub fetch_count: usize,
    pub fetch_bytes: usize,
    pub policy_revision: String,
    pub config_revision: String,
}

/// Explicit planning limits; every quantity is finite and nonzero where used.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlanLimits {
    pub max_sites: usize,
    pub max_pages_per_site: usize,
    pub deadline_millis: u64,
    pub concurrency: usize,
    pub total_calls: usize,
    pub fetch_count: usize,
    pub fetch_bytes: usize,
}

/// Why an otherwise discoverable card was not planned.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlanOmission {
    pub card_id: String,
    pub reason: String,
}

/// Planner result retains rejected cards for audit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlanReceipt {
    pub plan: SearchPlan,
    pub omissions: Vec<PlanOmission>,
}

/// Discovers only exact-shape, authorized Retriever cards and freezes a plan.
pub fn plan_search(
    cards: &[SkillCard],
    input: &ShapeRef,
    output: &ShapeRef,
    granted: &[CapabilityName],
    concurrent_safe_transports: &BTreeSet<String>,
    limits: PlanLimits,
    policy_revision: impl Into<String>,
    config_revision: impl Into<String>,
) -> Result<PlanReceipt, SearchFailure> {
    if limits.max_sites == 0
        || limits.max_pages_per_site == 0
        || limits.deadline_millis == 0
        || limits.concurrency == 0
        || limits.total_calls == 0
    {
        return Err(SearchFailure::Plan(
            "limits must be finite and positive".into(),
        ));
    }
    let mut selected = Vec::new();
    let mut omissions = Vec::new();
    let mut ordered: Vec<_> = cards.iter().collect();
    ordered.sort_by(|a, b| a.id.cmp(&b.id));
    for card in ordered {
        let reason = if !card.roles.contains(&SkillRole::Retriever) {
            Some("role is not Retriever")
        } else if card.input_shape != *input || card.output_shape != *output {
            Some("exact Shape mismatch")
        } else if !card.capabilities.iter().all(|c| granted.contains(c)) {
            Some("capability not granted")
        } else if selected.len() == limits.max_sites {
            Some("site bound reached")
        } else {
            None
        };
        if let Some(reason) = reason {
            omissions.push(PlanOmission {
                card_id: card.id.clone(),
                reason: reason.into(),
            });
        } else {
            selected.push(PlannedSite {
                card_id: card.id.clone(),
                transport_id: card.transport_id.clone(),
                concurrent_safe: concurrent_safe_transports.contains(&card.transport_id),
                cache_mode: card.policy.cache.clone(),
            });
        }
    }
    if selected.len() > limits.total_calls {
        selected.truncate(limits.total_calls);
    }
    Ok(PlanReceipt {
        plan: SearchPlan {
            sites: selected,
            max_pages_per_site: limits.max_pages_per_site,
            deadline_millis: limits.deadline_millis,
            concurrency: limits.concurrency,
            total_calls: limits.total_calls,
            fetch_count: limits.fetch_count,
            fetch_bytes: limits.fetch_bytes,
            policy_revision: policy_revision.into(),
            config_revision: config_revision.into(),
        },
        omissions,
    })
}

/// Cooperative cancellation checked before every provider dispatch.
#[derive(Clone, Default)]
pub struct SearchCancellation(Arc<AtomicBool>);
impl SearchCancellation {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

/// One provider execution, including partial evidence and raw identities.
#[derive(Clone, Debug)]
pub struct SiteOutcome {
    pub site_id: String,
    pub pages: Vec<SearchPage>,
    pub raw_response_ids: Vec<ContentId>,
    pub notices: Vec<String>,
    pub corrections: Vec<String>,
    pub failure: Option<String>,
}

/// Injected provider boundary. Implementations normally adapt skill dispatch.
pub trait RetrieverSite: Send + Sync {
    fn call(
        &self,
        query: &SearchQuery,
        page_limit: usize,
        cancel: &SearchCancellation,
    ) -> SiteOutcome;
}

/// One complete partial run; failure in one site cannot erase another.
#[derive(Clone, Debug)]
pub struct SearchRun {
    pub query: SearchQuery,
    pub plan: SearchPlan,
    pub sites: Vec<SiteOutcome>,
    pub alias_clusters: Vec<AliasCluster>,
    pub aliases: Vec<AliasEvidence>,
    pub rank: Option<RankedFusion<String>>,
    pub judge: Option<JudgeReceipt>,
    pub omissions: Vec<TypedOmission>,
}

/// Typed absence or denial retained in replay records.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TypedOmission {
    pub stage: String,
    pub subject: String,
    pub reason: String,
}

/// Executes concurrent-safe transports in bounded batches, serializing all others.
/// Results are always committed in planned site order.
pub fn dispatch(
    plan: &SearchPlan,
    query: &SearchQuery,
    sites: &BTreeMap<String, Arc<dyn RetrieverSite>>,
    cancel: &SearchCancellation,
) -> Vec<SiteOutcome> {
    let mut outcomes = BTreeMap::new();
    for chunk in plan.sites.chunks(plan.concurrency) {
        let mut joins = Vec::new();
        for selected in chunk {
            let Some(site) = sites.get(&selected.card_id).cloned() else {
                outcomes.insert(selected.card_id.clone(), missing(&selected.card_id));
                continue;
            };
            if cancel.is_cancelled() {
                outcomes.insert(selected.card_id.clone(), cancelled(&selected.card_id));
                continue;
            }
            if selected.concurrent_safe {
                let q = query.clone();
                let c = cancel.clone();
                let id = selected.card_id.clone();
                let pages = plan.max_pages_per_site;
                joins.push((id, thread::spawn(move || site.call(&q, pages, &c))));
            } else {
                outcomes.insert(
                    selected.card_id.clone(),
                    site.call(query, plan.max_pages_per_site, cancel),
                );
            }
        }
        for (id, join) in joins {
            outcomes.insert(
                id.clone(),
                join.join()
                    .unwrap_or_else(|_| failed(&id, "provider worker panicked")),
            );
        }
    }
    plan.sites
        .iter()
        .map(|s| {
            outcomes
                .remove(&s.card_id)
                .unwrap_or_else(|| missing(&s.card_id))
        })
        .collect()
}
fn missing(id: &str) -> SiteOutcome {
    failed(id, "planned site unavailable")
}
fn cancelled(id: &str) -> SiteOutcome {
    failed(id, "cancelled")
}
fn failed(id: &str, why: &str) -> SiteOutcome {
    SiteOutcome {
        site_id: id.into(),
        pages: vec![],
        raw_response_ids: vec![],
        notices: vec![],
        corrections: vec![],
        failure: Some(why.into()),
    }
}

/// Named conservative URI identity evidence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AliasRule {
    pub name: String,
    pub tracking_parameters: BTreeSet<String>,
}
/// An alias cluster retains every original URI and provider.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AliasCluster {
    pub key: String,
    pub original_uris: Vec<String>,
    pub providers: Vec<String>,
}

/// Clusters exact normalized identities, adding only explicitly named evidence.
pub fn cluster_aliases(
    sites: &[SiteOutcome],
    rule: Option<&AliasRule>,
) -> Result<(Vec<AliasCluster>, Vec<AliasEvidence>), SearchFailure> {
    let mut groups: BTreeMap<String, (BTreeSet<String>, BTreeSet<String>)> = BTreeMap::new();
    let mut evidence = Vec::new();
    for site in sites {
        for page in &site.pages {
            for observation in &page.observations {
                let original = observation.retrieval_uri.clone();
                let key = rule
                    .map(|r| strip_tracking(&original, &r.tracking_parameters))
                    .unwrap_or_else(|| original.clone());
                if key != original {
                    evidence.push(AliasEvidence {
                        left_uri: original.clone(),
                        right_uri: key.clone(),
                        basis: format!("tracking-rule:{}", rule.expect("present").name),
                        evidence_id: Datum::String(format!("{original}\n{key}"))
                            .content_id()
                            .map_err(|e| SearchFailure::Record(e.to_string()))?,
                    });
                }
                let entry = groups.entry(key).or_default();
                entry.0.insert(original);
                if let Some(claim) = &observation.claim {
                    entry.1.insert(claim.provider.clone());
                }
            }
        }
    }
    evidence.sort_by(|a, b| (&a.left_uri, &a.right_uri).cmp(&(&b.left_uri, &b.right_uri)));
    Ok((
        groups
            .into_iter()
            .map(|(key, (uris, providers))| AliasCluster {
                key,
                original_uris: uris.into_iter().collect(),
                providers: providers.into_iter().collect(),
            })
            .collect(),
        evidence,
    ))
}
fn strip_tracking(uri: &str, parameters: &BTreeSet<String>) -> String {
    let Some((base, q)) = uri.split_once('?') else {
        return uri.into();
    };
    let kept: Vec<_> = q
        .split('&')
        .filter(|p| !parameters.contains(p.split('=').next().unwrap_or("")))
        .collect();
    if kept.is_empty() {
        base.into()
    } else {
        format!("{base}?{}", kept.join("&"))
    }
}

/// Uses the shared RRF implementation over alias cluster keys.
pub fn fuse(
    sites: &[SiteOutcome],
    clusters: &[AliasCluster],
    rrf_k: usize,
    limits: FusionLimits,
) -> Result<RankedFusion<String>, SearchFailure> {
    let lookup: BTreeMap<_, _> = clusters
        .iter()
        .flat_map(|c| c.original_uris.iter().map(move |u| (u, c.key.clone())))
        .collect();
    let mut lists = Vec::new();
    for site in sites {
        let keys = site
            .pages
            .iter()
            .flat_map(|p| p.observations.iter())
            .filter_map(|o| lookup.get(&o.retrieval_uri).cloned())
            .collect();
        lists.push(RankedList::new(site.site_id.clone(), 1.0, keys).map_err(rank)?);
    }
    reciprocal_rank_fusion(lists, rrf_k, limits).map_err(rank)
}
fn rank(e: sim_lib_rank::RankError) -> SearchFailure {
    SearchFailure::Rank(e.to_string())
}

/// Optional learned reranking request; candidates are fenced by content id.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JudgeRequest {
    pub model_site_id: String,
    pub policy_revision: String,
    pub input_id: ContentId,
    pub fenced_candidates: String,
}
/// Full judge result; invalid output records failure and preserves RRF order.
#[derive(Clone, Debug, PartialEq)]
pub struct JudgeReceipt {
    pub model_site_id: String,
    pub policy_revision: String,
    pub input_id: ContentId,
    pub output_id: Option<ContentId>,
    pub deltas: BTreeMap<String, f64>,
    pub failure: Option<String>,
}
pub trait Judge: Send + Sync {
    fn judge(&self, request: &JudgeRequest) -> Result<(ContentId, BTreeMap<String, f64>), String>;
}
pub fn call_judge(
    judge: &dyn Judge,
    model_site_id: &str,
    policy_revision: &str,
    candidates: &[String],
) -> JudgeReceipt {
    let datum = Datum::Vector(candidates.iter().cloned().map(Datum::String).collect());
    let input_id = datum
        .content_id()
        .expect("bounded strings are content-addressable");
    let fenced = fenced_data_text_for_id("search-candidates", &candidates.join("\n"), &input_id);
    let request = JudgeRequest {
        model_site_id: model_site_id.into(),
        policy_revision: policy_revision.into(),
        input_id: input_id.clone(),
        fenced_candidates: fenced,
    };
    match judge.judge(&request) {
        Ok((output_id, deltas)) if deltas.values().all(|v| v.is_finite()) => JudgeReceipt {
            model_site_id: model_site_id.into(),
            policy_revision: policy_revision.into(),
            input_id,
            output_id: Some(output_id),
            deltas,
            failure: None,
        },
        Ok(_) => JudgeReceipt {
            model_site_id: model_site_id.into(),
            policy_revision: policy_revision.into(),
            input_id,
            output_id: None,
            deltas: BTreeMap::new(),
            failure: Some("invalid judge output".into()),
        },
        Err(e) => JudgeReceipt {
            model_site_id: model_site_id.into(),
            policy_revision: policy_revision.into(),
            input_id,
            output_id: None,
            deltas: BTreeMap::new(),
            failure: Some(e),
        },
    }
}

/// No-network local corpus using caller-supplied embeddings and ordinary pages.
pub fn local_corpus_page(
    index: &impl EmbeddingIndex,
    query_embedding: &[f32],
    documents: &BTreeMap<String, (String, String)>,
    query: SearchQuery,
    limit: usize,
) -> Result<SearchPage, SearchFailure> {
    let mut budget = RankLimits::default();
    let hits = retrieve_limited(index, query_embedding, limit, &mut budget).map_err(rank)?;
    let observations = hits
        .into_iter()
        .filter_map(|hit| {
            documents.get(&hit.id).map(|(uri, title)| {
                SearchObservation::checked(
                    uri,
                    Some(ProviderClaim {
                        provider: "local-corpus".into(),
                        uri: uri.clone(),
                        title: Some(title.clone()),
                        snippet: None,
                        position: None,
                    }),
                    None,
                )
            })
        })
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| SearchFailure::Record(e.to_string()))?;
    Ok(SearchPage {
        query,
        observations,
        continuation: None,
    })
}

/// Independently authorized captured landing page.
#[derive(Clone, Debug)]
pub struct CapturedPage {
    pub uri: String,
    pub representation: WebRepresentation,
    pub selectors: Vec<EvidenceSelector>,
    pub policy_receipt: String,
}
pub trait PageCapturer: Send + Sync {
    fn capture(&self, uri: &str, max_bytes: usize) -> Result<CapturedPage, String>;
}
/// Distinct operation: provider claims only.
pub fn query(
    plan: SearchPlan,
    query: SearchQuery,
    sites: &BTreeMap<String, Arc<dyn RetrieverSite>>,
    cancel: &SearchCancellation,
) -> Result<SearchRun, SearchFailure> {
    let outcomes = dispatch(&plan, &query, sites, cancel);
    let (clusters, aliases) = cluster_aliases(&outcomes, None)?;
    let rank = if clusters.is_empty() {
        None
    } else {
        Some(fuse(
            &outcomes,
            &clusters,
            60,
            FusionLimits::new(100, 1000, query.limit as usize).map_err(rank)?,
        )?)
    };
    Ok(SearchRun {
        query,
        plan,
        sites: outcomes,
        alias_clusters: clusters,
        aliases,
        rank,
        judge: None,
        omissions: vec![],
    })
}
/// Distinct operation: selection plus independently authorized page capture.
pub fn inspect(run: SearchRun, capturer: &dyn PageCapturer) -> (SearchRun, Vec<CapturedPage>) {
    let mut captures = Vec::new();
    let mut omissions = run.omissions.clone();
    for key in run
        .rank
        .as_ref()
        .into_iter()
        .flat_map(|r| r.items.iter().map(|i| i.key.as_str()))
        .take(run.plan.fetch_count)
    {
        match capturer.capture(key, run.plan.fetch_bytes) {
            Ok(page) => captures.push(page),
            Err(reason) => omissions.push(TypedOmission {
                stage: "fetch".into(),
                subject: key.into(),
                reason,
            }),
        }
    }
    let mut run = run;
    run.omissions = omissions;
    (run, captures)
}
/// Distinct operation: verified normalized selectors, never prose synthesis.
pub fn research(
    run: SearchRun,
    captures: Vec<CapturedPage>,
) -> Result<ResearchBundle, SearchFailure> {
    let mut selectors = Vec::new();
    let mut representations = Vec::new();
    for capture in captures {
        representations.push(capture.representation.content_id.clone());
        for selector in capture.selectors {
            selector
                .verify(&capture.representation)
                .map_err(|e| SearchFailure::Record(e.to_string()))?;
            selectors.push(
                Citation::checked(&capture.representation, selector)
                    .map_err(|e| SearchFailure::Record(e.to_string()))?,
            );
        }
    }
    Ok(ResearchBundle {
        run,
        representations,
        selectors,
    })
}
#[derive(Clone, Debug)]
pub struct ResearchBundle {
    pub run: SearchRun,
    pub representations: Vec<ContentId>,
    pub selectors: Vec<Citation>,
}

/// Fences an untrusted provider claim while preserving its epistemic label.
pub fn fenced_claim(claim: &ProviderClaim) -> Result<String, SearchFailure> {
    let text = format!(
        "PROVIDER CLAIM (unverified)\ntitle: {}\nsnippet: {}",
        claim.title.as_deref().unwrap_or(""),
        claim.snippet.as_deref().unwrap_or("")
    );
    let id = Datum::String(text.clone())
        .content_id()
        .map_err(|e| SearchFailure::Record(e.to_string()))?;
    Ok(fenced_data_text_for_id("provider-claim", &text, &id))
}
/// Fences captured text under its independently observed content identity.
pub fn fenced_capture(rep: &WebRepresentation) -> String {
    fenced_data_text_for_id("captured-page-observation", &rep.text, &rep.content_id)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SearchFailure {
    Plan(String),
    Rank(String),
    Record(String),
}
impl fmt::Display for SearchFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for SearchFailure {}

#[cfg(test)]
mod tests;
