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
