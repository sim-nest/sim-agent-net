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
