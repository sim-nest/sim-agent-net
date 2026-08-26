use sim_kernel::Symbol;

use crate::*;

/// Pure, total transition function. Rejections never return a partially changed state.
pub fn reduce(
    policy: &ExecutionPolicy,
    plan: &MutationPlan,
    execution: &ExecutionId,
    phase: &sim_roadmap_core::PhaseId,
    attempt: &AttemptId,
    current: &Transition,
    event: &ExecutionEvent,
) -> Result<Transition, ExecutionFailure> {
    correlate(plan, execution, phase, attempt, current, event)?;
    let mut next = current.clone();
    next.requested_effects.clear();
    let o = &event.observation;
    next.journal_head = o.journal_head.clone();
    match (current.state, o.kind.to_string().as_str()) {
        (PhaseRunState::Planned, "start") => {
            next.state = PhaseRunState::Running;
            request(
                &mut next,
                "apply-mutation",
                execution,
                phase,
                Some(plan.id.clone()),
                None,
            );
        }
        (PhaseRunState::Running, "image-observed")
        | (PhaseRunState::Reconciling, "image-observed") => observe_image(
            plan,
            &mut next,
            o.image
                .clone()
                .ok_or_else(|| ExecutionFailure::InvalidObservation(o.kind.clone()))?,
        )?,
        (PhaseRunState::Running, "mutation-committed") => {
            next.state = PhaseRunState::Reconciling;
            request(
                &mut next,
                "run-proof",
                execution,
                phase,
                Some(plan.id.clone()),
                o.proof_cursor.clone(),
            );
        }
        (PhaseRunState::Reconciling, "promise-discharged") => {
            let d = o
                .discharge
                .clone()
                .ok_or_else(|| ExecutionFailure::InvalidObservation(o.kind.clone()))?;
            next.discharges.retain(|v| v.promise != d.promise);
            next.discharges.push(d);
            next.discharges.sort_by(|a, b| a.promise.cmp(&b.promise));
        }
        (PhaseRunState::Reconciling, "proof-unresolved") => {
            next.unresolved.push(
                o.unresolved
                    .clone()
                    .ok_or_else(|| ExecutionFailure::InvalidObservation(o.kind.clone()))?,
            );
            next.unresolved = sorted_unique(next.unresolved);
        }
        (PhaseRunState::Reconciling, "source-deck-current") => {
            next.current_source_deck = o.source_deck.clone()
        }
        (PhaseRunState::Reconciling, "parent-accepted") => next.parent_acceptance_retained = true,
        (PhaseRunState::Reconciling, "succeed") => {
            succeed(policy, plan, execution, phase, &mut next)?
        }
        (PhaseRunState::Running | PhaseRunState::Reconciling, "fail") => {
            next.state = PhaseRunState::Failed
        }
        (
            PhaseRunState::Planned | PhaseRunState::Running | PhaseRunState::Reconciling,
            "cancel",
        ) => next.state = PhaseRunState::Cancelled,
        _ => return Err(ExecutionFailure::InvalidState),
    }
    Ok(next)
}

pub fn replay(
    policy: &ExecutionPolicy,
    plan: &MutationPlan,
    execution: &ExecutionId,
    phase: &sim_roadmap_core::PhaseId,
    attempt: &AttemptId,
    initial: &Transition,
    events: &[ExecutionEvent],
) -> Result<Transition, ExecutionFailure> {
    events.iter().try_fold(initial.clone(), |s, e| {
        reduce(policy, plan, execution, phase, attempt, &s, e)
    })
}

fn correlate(
    plan: &MutationPlan,
    execution: &ExecutionId,
    phase: &sim_roadmap_core::PhaseId,
    attempt: &AttemptId,
    current: &Transition,
    event: &ExecutionEvent,
) -> Result<(), ExecutionFailure> {
    if &event.execution != execution {
        return Err(ExecutionFailure::WrongExecution);
    }
    if &event.phase != phase {
        return Err(ExecutionFailure::WrongPhase);
    }
    if &event.attempt != attempt {
        return Err(ExecutionFailure::WrongAttempt);
    }
    if event.observation.journal_head == current.journal_head {
        return Err(ExecutionFailure::WrongJournalHead);
    }
    if event
        .observation
        .mutation
        .as_ref()
        .is_some_and(|m| m != &plan.id)
    {
        return Err(ExecutionFailure::WrongMutation);
    }
    if let Some(c) = &event.observation.proof_cursor
        && (c.journal_head != current.journal_head || c.sequence == 0)
    {
        return Err(ExecutionFailure::WrongProofCursor);
    }
    Ok(())
}
fn observe_image(
    plan: &MutationPlan,
    next: &mut Transition,
    image: FileImage,
) -> Result<(), ExecutionFailure> {
    match plan.classify(&image) {
        ImageClass::Postimage | ImageClass::PreAndPost => {
            next.committed_postimages.push(image);
            next.committed_postimages = sorted_unique(next.committed_postimages.clone());
            Ok(())
        }
        ImageClass::Preimage => Ok(()),
        ImageClass::Foreign => Err(ExecutionFailure::ForeignImage),
    }
}
fn request(
    next: &mut Transition,
    kind: &str,
    execution: &ExecutionId,
    phase: &sim_roadmap_core::PhaseId,
    mutation: Option<MutationId>,
    proof_cursor: Option<ProofCursor>,
) {
    next.requested_effects.push(EffectRequest {
        kind: Symbol::new(kind),
        execution: execution.clone(),
        phase: phase.clone(),
        mutation,
        proof_cursor,
    })
}
fn succeed(
    policy: &ExecutionPolicy,
    plan: &MutationPlan,
    execution: &ExecutionId,
    phase: &sim_roadmap_core::PhaseId,
    next: &mut Transition,
) -> Result<(), ExecutionFailure> {
    if next.committed_postimages != plan.postimages {
        return Err(ExecutionFailure::SuccessInvariant(
            "postimages not committed",
        ));
    }
    if next.current_source_deck.as_ref() != Some(&policy.source_deck) {
        return Err(ExecutionFailure::SuccessInvariant("source deck is stale"));
    }
    if policy.required_promises.iter().any(|p| {
        !next
            .discharges
            .iter()
            .any(|d| &d.promise == p && d.proven())
    }) {
        return Err(ExecutionFailure::SuccessInvariant(
            "required promise is not proven",
        ));
    }
    if !next.parent_acceptance_retained {
        return Err(ExecutionFailure::SuccessInvariant(
            "parent acceptance not retained",
        ));
    }
    if next.unresolved.iter().any(|u| u.mandatory)
        || policy
            .required_proofs
            .iter()
            .any(|p| next.unresolved.iter().any(|u| u.mandatory && &u.proof == p))
    {
        return Err(ExecutionFailure::SuccessInvariant(
            "mandatory proof unresolved",
        ));
    }
    next.state = PhaseRunState::Succeeded;
    next.receipt = Some(PhaseReceipt {
        execution: execution.clone(),
        phase: phase.clone(),
        source_deck: policy.source_deck.clone(),
        journal_head: next.journal_head.clone(),
        committed_postimages: next.committed_postimages.clone(),
        discharges: next.discharges.clone(),
        parent_acceptance_retained: true,
    });
    Ok(())
}
