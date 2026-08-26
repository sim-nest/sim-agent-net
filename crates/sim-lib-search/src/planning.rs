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

/// Frozen planner inputs that apply uniformly to every candidate card.
pub struct SearchPlanRequest<'a> {
    /// Exact input Shape required by the caller.
    pub input: &'a ShapeRef,
    /// Exact output Shape required by the caller.
    pub output: &'a ShapeRef,
    /// Capabilities granted to the planned search.
    pub granted: &'a [CapabilityName],
    /// Transports whose providers may execute concurrently.
    pub concurrent_safe_transports: &'a BTreeSet<String>,
    /// Finite site, page, time, concurrency, call, and fetch limits.
    pub limits: PlanLimits,
    /// Immutable policy revision recorded in the plan.
    pub policy_revision: String,
    /// Immutable configuration revision recorded in the plan.
    pub config_revision: String,
}

/// Discovers only exact-shape, authorized Retriever cards and freezes a plan.
pub fn plan_search(
    cards: &[SkillCard],
    request: SearchPlanRequest<'_>,
) -> Result<PlanReceipt, SearchFailure> {
    let SearchPlanRequest {
        input,
        output,
        granted,
        concurrent_safe_transports,
        limits,
        policy_revision,
        config_revision,
    } = request;
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
            policy_revision,
            config_revision,
        },
        omissions,
    })
}
