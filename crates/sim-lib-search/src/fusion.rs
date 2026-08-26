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
