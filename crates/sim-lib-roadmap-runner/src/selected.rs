// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! One selected plan conducted through the public roadmap command.
//!
//! Each call admits at most one effect. Proof, mutation, landing, delivery,
//! and recovery are separate calls. A world or prove document is executed by
//! the caller-supplied product hook; this crate does not own those commands.

use std::sync::{Arc, Mutex};

use sim_kernel::{ContentId, Symbol};
use sim_lib_journal::MemoryBackend;
use sim_roadmap_core::PhaseId;
use sim_roadmap_exec_core::{
    AttemptId, ExecutionEvent, ExecutionId, ExecutionPolicy, ExecutionPolicyId, FileImage,
    MutationId, MutationPlan,
};
use sim_work_core::CapabilityGrant;

use crate::{
    AuthorityGrant, CancellationPort, DirectionKind, DirectionQuestion, DriveResult,
    EffectiveCeiling, ExecutionIdentity, ExecutionRecord, GenerationHandle, InputBatch, Limits,
    LocalExecutionIdentity, LocalRoadmapReceipt, LocalRoadmapRequest, OpenRequest, OwnedLimit,
    PublicLocalRoadmapPort, ReadinessPort, RoadmapRunnerService, ServiceError, conduct_one,
};

#[path = "selected_bind.rs"]
mod bind;
#[path = "selected_effect.rs"]
mod effect;

#[cfg(test)]
#[path = "selected_tests.rs"]
mod tests;

const ORDER: [&str; 5] = ["proof", "mutation", "land", "deliver", "recover"];

struct Body {
    plan: bind::Plan,
    bound: bind::Bound,
    prove_hook: Option<crate::ProductHook>,
    deliver_hook: Option<crate::ProductHook>,
    events: Vec<ExecutionEvent>,
    next_head: u8,
    attempt: AttemptId,
    source_deck: ContentId,
    postimage: FileImage,
    expected: Option<ContentId>,
    directed: bool,
    done: Vec<String>,
    seal_hex: String,
    landed: Option<crate::ModelTip>,
    admitting: Option<String>,
}

#[derive(Clone)]
struct ProductPort {
    body: Arc<Mutex<Body>>,
}

struct Ready(PhaseId);
struct Never;

type Service = RoadmapRunnerService<MemoryBackend, Ready, ProductPort, Never>;

pub(crate) struct Session {
    backend: Arc<MemoryBackend>,
    request: OpenRequest,
    service: Arc<Mutex<Service>>,
    body: Arc<Mutex<Body>>,
    plan_text: String,
    identity: LocalExecutionIdentity,
}

pub(crate) fn conduct_request(
    port: &PublicLocalRoadmapPort,
    request: &LocalRoadmapRequest,
    generation: GenerationHandle,
) -> Result<LocalRoadmapReceipt, String> {
    if generation.identity() != request.identity.runner_generation {
        return Err("generation identity drift".into());
    }
    let plan_text = request.plan.as_deref().ok_or("selected plan is missing")?;
    let mut sessions = port
        .sessions
        .lock()
        .map_err(|_| "selected plan store poisoned")?;
    let key = request.identity.execution.clone();
    if !sessions.contains_key(&key) {
        if request.verb.as_str() != "run" {
            return Err("selected plan is not open".into());
        }
        let plan = bind::parse(plan_text)?;
        if plan.prove_document.is_some() && port.prove_hook.is_none() {
            return Err("prove product is not loaded".into());
        }
        if plan.world_document.is_some() && port.deliver_hook.is_none() {
            return Err("world delivery is not loaded".into());
        }
        sessions.insert(
            key.clone(),
            open_session(port, plan_text, plan, request.identity.clone())?,
        );
    }
    let session = sessions.get(&key).expect("session");
    if session.identity != request.identity {
        return Err("selected plan identity changed".into());
    }
    if session.plan_text != plan_text {
        return Err("selected plan changed".into());
    }
    let service = Arc::clone(&session.service);
    let body = Arc::clone(&session.body);
    let backend = Arc::clone(&session.backend);
    let opened = session.request.clone();
    let identity = session.identity.clone();
    drop(sessions);
    match request.verb.as_str() {
        "run" => run_product(&service, &body, &opened, &identity, request),
        "resume" => {
            reopen(&service, &backend, &opened, &body)?;
            receipt(&service, &body, &identity, "resume", "reopened")
        }
        "status" | "explain" | "replay" => {
            let outcome = if request.verb == "replay" {
                let guard = lock_service(&service)?;
                if guard.effect_outstanding() {
                    return Err("an effect is already admitted".into());
                }
                guard.replay().map_err(|err| err.to_string())?;
                "replayed"
            } else {
                "read"
            };
            let before = effect_count(&service);
            let _ = conduct_one(&service, &InputBatch::no_change(), &grant())
                .map_err(|err| err.to_string())?;
            if effect_count(&service) != before {
                return Err("a query appended an effect".into());
            }
            receipt(&service, &body, &identity, &request.verb, outcome)
        }
        _ => Err("unsupported local roadmap verb".into()),
    }
}

fn open_session(
    port: &PublicLocalRoadmapPort,
    text: &str,
    plan: bind::Plan,
    identity: LocalExecutionIdentity,
) -> Result<Session, String> {
    let bound = bind::bind(&plan)?;
    let source_deck = bind::content(&plan.seal);
    let postimage = FileImage {
        path: plan.land_path.clone(),
        content: Some(bound.postimage_id.clone()),
    };
    let request = open_request(&plan, &source_deck, postimage.clone())?;
    let body = Arc::new(Mutex::new(Body {
        plan,
        bound,
        prove_hook: port.prove_hook.clone(),
        deliver_hook: port.deliver_hook.clone(),
        events: Vec::new(),
        next_head: 40,
        attempt: request.attempt.clone(),
        source_deck,
        postimage,
        expected: None,
        directed: false,
        done: Vec::new(),
        seal_hex: String::new(),
        landed: None,
        admitting: None,
    }));
    let backend = Arc::new(MemoryBackend::new());
    let service = Arc::new(Mutex::new(
        RoadmapRunnerService::open(
            Arc::clone(&backend),
            Ready(request.phase.clone()),
            ProductPort {
                body: Arc::clone(&body),
            },
            Never,
            request.clone(),
            Limits::default(),
        )
        .map_err(|err| err.to_string())?,
    ));
    Ok(Session {
        backend,
        request,
        service,
        body,
        plan_text: text.to_owned(),
        identity,
    })
}

fn run_product(
    service: &Arc<Mutex<Service>>,
    body: &Arc<Mutex<Body>>,
    opened: &OpenRequest,
    identity: &LocalExecutionIdentity,
    request: &LocalRoadmapRequest,
) -> Result<LocalRoadmapReceipt, String> {
    let product = request.product.as_deref().unwrap_or("direction");
    if product == "direction" {
        return direct(service, body, opened, identity, request);
    }
    if !ORDER.contains(&product) {
        return Err(format!("unknown product {product}"));
    }
    {
        let mut body = lock(body)?;
        if !body.directed {
            return Err("direction has not been accepted".into());
        }
        if body.done.iter().any(|done| done == product) {
            return Err(format!("{product} was already admitted"));
        }
        if body.done.len() >= ORDER.len() || ORDER[body.done.len()] != product {
            return Err(format!("{product} is not the next product"));
        }
        if body.admitting.is_some() {
            return Err("an effect is already admitted".into());
        }
        body.admitting = Some(product.to_owned());
    }
    let mut inputs = InputBatch::conduct();
    inputs.product = Some(Symbol::new(product));
    let driven = match conduct_one(service, &inputs, &grant()) {
        Ok(driven) => driven,
        Err(err) => {
            clear_admitting(body);
            return Err(err.to_string());
        }
    };
    let mut guard = lock(body)?;
    guard.admitting = None;
    if !matches!(driven, DriveResult::Advanced { .. }) {
        return Err(format!("{product} was not admitted"));
    }
    guard.done.push(product.to_owned());
    drop(guard);
    receipt(service, body, identity, product, "admitted")
}

fn direct(
    service: &Arc<Mutex<Service>>,
    body: &Arc<Mutex<Body>>,
    opened: &OpenRequest,
    identity: &LocalExecutionIdentity,
    request: &LocalRoadmapRequest,
) -> Result<LocalRoadmapReceipt, String> {
    {
        let mut body = lock(body)?;
        if body.directed {
            return Err("direction was already consumed".into());
        }
        if body.admitting.is_some() {
            return Err("an effect is already admitted".into());
        }
        body.admitting = Some("direction".to_owned());
    }
    let inputs = effect::direction_inputs(body, opened, request);
    let inputs = match inputs {
        Ok(inputs) => inputs,
        Err(err) => {
            clear_admitting(body);
            return Err(err);
        }
    };
    let driven = match conduct_one(service, &inputs, &grant()) {
        Ok(driven) => driven,
        Err(err) => {
            clear_admitting(body);
            return Err(err.to_string());
        }
    };
    clear_admitting(body);
    match driven {
        DriveResult::NeedDirection { request: published } => {
            lock(body)?.expected = Some(published.expected_revision);
            receipt(service, body, identity, "direction", "need-direction")
        }
        DriveResult::Advanced { .. } => {
            lock(body)?.directed = true;
            receipt(service, body, identity, "direction", "accepted")
        }
        DriveResult::Quiescent { .. } => Err("direction changed nothing".into()),
        other => Err(format!("direction was not admitted: {other:?}")),
    }
}

fn reopen(
    service: &Arc<Mutex<Service>>,
    backend: &Arc<MemoryBackend>,
    opened: &OpenRequest,
    body: &Arc<Mutex<Body>>,
) -> Result<(), String> {
    let before = effect_count(service);
    {
        let mut guard = lock_service(service)?;
        if guard.effect_outstanding() {
            return Err("an effect is already admitted".into());
        }
        *guard = RoadmapRunnerService::open(
            Arc::clone(backend),
            Ready(opened.phase.clone()),
            ProductPort {
                body: Arc::clone(body),
            },
            Never,
            opened.clone(),
            Limits::default(),
        )
        .map_err(|err| err.to_string())?;
        let inspection = guard.inspect().map_err(|err| err.to_string())?;
        let replayed = guard.replay().map_err(|err| err.to_string())?;
        if replayed.state != inspection.transition.state {
            return Err("reopened reducer does not match the journal".into());
        }
    }
    let driven =
        conduct_one(service, &InputBatch::no_change(), &grant()).map_err(|err| err.to_string())?;
    if !matches!(driven, DriveResult::Quiescent { .. }) {
        return Err("recovery wake was not quiescent".into());
    }
    if effect_count(service) != before {
        return Err("recovery wake appended an effect".into());
    }
    Ok(())
}

fn receipt(
    service: &Mutex<Service>,
    body: &Mutex<Body>,
    identity: &LocalExecutionIdentity,
    product: &str,
    outcome: &str,
) -> Result<LocalRoadmapReceipt, String> {
    // Release the service before the body. Replay holds the service while the
    // receipt reader locks the body, so the opposite order deadlocks.
    let effects = effect_count(service);
    let body = lock(body)?;
    let detail = format!(
        "opened={} seal={} provider-id={} operation-id={} product={product} outcome={outcome} effects={effects}",
        bind::hex(&body.bound.revision.id),
        body.seal_hex,
        bind::hex(&bind::content(body.plan.provider.as_bytes())),
        bind::hex(&bind::content(body.plan.operation.as_bytes())),
    );
    Ok(LocalRoadmapReceipt {
        outcome: "ok".into(),
        identity: identity.clone(),
        journal_acknowledged: true,
        detail,
    })
}

fn effect_count(service: &Mutex<Service>) -> usize {
    let Ok(service) = service.lock() else {
        return 0;
    };
    service
        .inspect()
        .map(|item| {
            item.journal
                .records
                .iter()
                .filter(|record| matches!(record, ExecutionRecord::EffectRequested { .. }))
                .count()
        })
        .unwrap_or(0)
}

fn lock_service(service: &Mutex<Service>) -> Result<std::sync::MutexGuard<'_, Service>, String> {
    service
        .lock()
        .map_err(|_| "selected plan service poisoned".to_owned())
}

fn clear_admitting(body: &Mutex<Body>) {
    if let Ok(mut body) = lock(body) {
        body.admitting = None;
    }
}

fn lock(body: &Mutex<Body>) -> Result<std::sync::MutexGuard<'_, Body>, String> {
    body.lock()
        .map_err(|_| "selected plan body poisoned".to_owned())
}

fn grant() -> CapabilityGrant {
    CapabilityGrant {
        capabilities: vec![Symbol::new("begin")],
    }
}

fn open_request(
    plan: &bind::Plan,
    source_deck: &ContentId,
    postimage: FileImage,
) -> Result<OpenRequest, String> {
    let policy_id = ExecutionPolicyId::new("policy").map_err(|err| err.to_string())?;
    let policy = ExecutionPolicy {
        id: policy_id.clone(),
        source_deck: source_deck.clone(),
        required_promises: vec![],
        required_proofs: vec![],
    };
    Ok(OpenRequest {
        authority: AuthorityGrant {
            identity: ExecutionIdentity {
                execution: ExecutionId::new(&plan.execution).map_err(|err| err.to_string())?,
                policy: policy_id,
                roadmap: bind::content(b"roadmap"),
                source_deck: source_deck.clone(),
                conduct: bind::content(plan.operation.as_bytes()),
                model: bind::content(plan.provider.as_bytes()),
                launcher: bind::content(b"launcher"),
                runner: bind::content(b"runner"),
            },
            ceiling: EffectiveCeiling::intersect([OwnedLimit {
                owner: Symbol::new("caller"),
                unit: Symbol::new("turns"),
                amount: 8,
            }]),
            grant: bind::cid_byte(7),
        },
        phase: PhaseId::new("leaf").map_err(|err| format!("{err:?}"))?,
        attempt: AttemptId::new("attempt").map_err(|err| err.to_string())?,
        policy,
        mutation: MutationPlan::new(
            MutationId::new("mutation").map_err(|err| err.to_string())?,
            vec![],
            vec![postimage],
        )
        .map_err(|err| err.to_string())?,
        resources: vec![],
        direction: Some(DirectionQuestion {
            id: bind::content(plan.direction.as_bytes()),
            question: plan.direction.clone(),
            answer_kind: DirectionKind::Decision,
            required_authority: Some(Symbol::new("begin")),
        }),
        handoff: None,
    })
}

impl ReadinessPort for Ready {
    fn admitted_leaf(&self, _: &ContentId) -> Result<Option<PhaseId>, ServiceError> {
        Ok(Some(self.0.clone()))
    }
}

impl CancellationPort for Never {
    fn cancellation_requested(&self, _: &ExecutionId) -> bool {
        false
    }
}
