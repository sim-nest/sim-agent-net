// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! One external plan through the public roadmap command.

use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use crate::{
    GenerationHandle, LocalExecutionIdentity, LocalRoadmapPort, LocalRoadmapRequest,
    PublicLocalRoadmapPort,
};

fn plan(extra: &str) -> String {
    format!(
        "sim-selected-plan-v1
provider harbor
operation conduct
execution execution
owner owner/authored
generator owner/generator
seal seal-bytes
authored fact/src authored-bytes
generated fact/out generated-bytes
claim claim/src fact/src
proof leaf same same
mutate model/file before after
land model/land base next
direction begin-work
{extra}"
    )
}

fn request(
    document: &str,
    verb: &str,
    product: Option<&str>,
    answer: Option<&str>,
    answer_kind: Option<&str>,
    named_revision: Option<&str>,
) -> LocalRoadmapRequest {
    LocalRoadmapRequest {
        verb: verb.into(),
        observe: false,
        disposable_checkout: None,
        local_authority_token: None,
        identity: LocalExecutionIdentity {
            execution: "local/selected".into(),
            conduct: "conduct".into(),
            model_pick: "harbor".into(),
            proof_catalog: "catalog".into(),
            runner_generation: "generation-1".into(),
        },
        plan: Some(document.into()),
        product: product.map(str::to_owned),
        answer: answer.map(str::to_owned),
        answer_kind: answer_kind.map(str::to_owned),
        named_revision: named_revision.map(str::to_owned),
    }
}

fn call(
    port: &PublicLocalRoadmapPort,
    document: &str,
    verb: &str,
    product: Option<&str>,
    answer: Option<&str>,
    answer_kind: Option<&str>,
    named_revision: Option<&str>,
) -> Result<crate::LocalRoadmapReceipt, String> {
    let asked = request(document, verb, product, answer, answer_kind, named_revision);
    let generation = GenerationHandle::acquire(asked.identity.runner_generation.clone());
    port.invoke(&asked, generation)
}

fn field(detail: &str, name: &str) -> String {
    detail
        .split_whitespace()
        .find_map(|item| item.strip_prefix(&format!("{name}=")))
        .unwrap_or("")
        .to_owned()
}

#[test]
fn one_external_plan_admits_each_product_once() {
    let prove_docs = Arc::new(Mutex::new(Vec::new()));
    let world_docs = Arc::new(Mutex::new(Vec::new()));
    let prove_seen = Arc::clone(&prove_docs);
    let world_seen = Arc::clone(&world_docs);
    let port = PublicLocalRoadmapPort::with_products(
        Arc::new(move |document| {
            prove_seen.lock().expect("prove").push(document.to_owned());
            Ok(())
        }),
        Arc::new(move |document| {
            world_seen.lock().expect("world").push(document.to_owned());
            Ok(())
        }),
    );
    let document = plan("prove-document prove-doc\nworld-document world-doc\n");
    let published = call(&port, &document, "run", None, None, None, None).expect("publish");
    assert_eq!(field(&published.detail, "outcome"), "need-direction");
    assert_eq!(field(&published.detail, "effects"), "0");
    assert!(field(&published.detail, "seal").is_empty());
    let stale = call(
        &port,
        &document,
        "run",
        Some("direction"),
        Some("go"),
        None,
        Some("other"),
    );
    assert_eq!(stale.unwrap_err(), "direction revision is stale");
    let advice = call(
        &port,
        &document,
        "run",
        Some("direction"),
        Some("go"),
        Some("advice"),
        None,
    );
    assert_eq!(
        advice.unwrap_err(),
        "advisory input cannot authorize a decision"
    );
    let skipped = call(&port, &document, "run", Some("mutation"), None, None, None);
    assert_eq!(skipped.unwrap_err(), "direction has not been accepted");
    let accepted = call(
        &port,
        &document,
        "run",
        Some("direction"),
        Some("go"),
        None,
        None,
    )
    .expect("accept");
    assert_eq!(field(&accepted.detail, "outcome"), "accepted");
    assert_eq!(field(&accepted.detail, "effects"), "0");
    let again = call(
        &port,
        &document,
        "run",
        Some("direction"),
        Some("other"),
        None,
        None,
    );
    assert_eq!(again.unwrap_err(), "direction was already consumed");
    let early = call(&port, &document, "run", Some("land"), None, None, None);
    assert_eq!(early.unwrap_err(), "land is not the next product");
    for (product, effects) in [("proof", "1"), ("mutation", "2"), ("land", "3")] {
        let receipt = call(&port, &document, "run", Some(product), None, None, None)
            .unwrap_or_else(|err| panic!("{product}: {err}"));
        assert_eq!(field(&receipt.detail, "product"), product);
        assert_eq!(field(&receipt.detail, "outcome"), "admitted");
        assert_eq!(field(&receipt.detail, "effects"), effects);
    }
    assert_eq!(
        field(
            &call(&port, &document, "status", None, None, None, None)
                .expect("status")
                .detail,
            "effects"
        ),
        "3"
    );
    let proof_seal = field(
        &call(&port, &document, "explain", None, None, None, None)
            .expect("explain")
            .detail,
        "seal",
    );
    assert_eq!(proof_seal.len(), 64);
    let delivered =
        call(&port, &document, "run", Some("deliver"), None, None, None).expect("deliver");
    assert_eq!(field(&delivered.detail, "effects"), "4");
    let recovered =
        call(&port, &document, "run", Some("recover"), None, None, None).expect("recover");
    assert_eq!(field(&recovered.detail, "effects"), "5");
    assert_eq!(prove_docs.lock().expect("prove").as_slice(), ["prove-doc"]);
    assert_eq!(world_docs.lock().expect("world").as_slice(), ["world-doc"]);
    let repeated = call(&port, &document, "run", Some("proof"), None, None, None);
    assert_eq!(repeated.unwrap_err(), "proof was already admitted");
    let resumed = call(&port, &document, "resume", None, None, None, None).expect("resume");
    assert_eq!(field(&resumed.detail, "outcome"), "reopened");
    assert_eq!(field(&resumed.detail, "effects"), "5");
    let replayed = call(&port, &document, "replay", None, None, None, None).expect("replay");
    assert_eq!(field(&replayed.detail, "outcome"), "replayed");
    assert_eq!(field(&replayed.detail, "effects"), "5");
    assert_eq!(field(&replayed.detail, "provider-id").len(), 64);
    assert_eq!(field(&replayed.detail, "operation-id").len(), 64);
    assert_eq!(field(&replayed.detail, "opened").len(), 64);
}

#[test]
fn unloaded_documents_refuse_before_an_effect() {
    let port = PublicLocalRoadmapPort::default();
    let document = plan("prove-document prove-doc\nworld-document world-doc\n");
    let refused = call(&port, &document, "run", None, None, None, None);
    assert_eq!(refused.unwrap_err(), "prove product is not loaded");
    let status = call(&port, &document, "status", None, None, None, None);
    assert_eq!(status.unwrap_err(), "selected plan is not open");
    let world_only = plan("world-document world-doc\n");
    let refused = call(&port, &world_only, "run", None, None, None, None);
    assert_eq!(refused.unwrap_err(), "world delivery is not loaded");
}

#[test]
fn a_later_call_cannot_rename_the_plan_or_its_identity() {
    let port = PublicLocalRoadmapPort::default();
    let document = plan("");
    call(&port, &document, "run", None, None, None, None).expect("publish");
    let mut renamed = request(&document, "run", Some("direction"), Some("go"), None, None);
    renamed.identity.model_pick = "other-harbor".into();
    let generation = GenerationHandle::acquire(renamed.identity.runner_generation.clone());
    assert_eq!(
        port.invoke(&renamed, generation).unwrap_err(),
        "selected plan identity changed"
    );
    let mut other = request(&document, "status", None, None, None, None);
    other.plan = Some(plan("note extra\n"));
    let generation = GenerationHandle::acquire(other.identity.runner_generation.clone());
    let renamed_plan = port.invoke(&other, generation);
    assert_eq!(renamed_plan.unwrap_err(), "selected plan changed");
    let status = call(&port, &document, "status", None, None, None, None).expect("status");
    assert_eq!(status.identity.conduct, "conduct");
    assert_eq!(status.identity.model_pick, "harbor");
    assert_eq!(field(&status.detail, "effects"), "0");
}

#[test]
fn a_plan_without_documents_uses_the_runner_products() {
    let port = PublicLocalRoadmapPort::default();
    let document = plan("");
    call(&port, &document, "run", None, None, None, None).expect("publish");
    call(
        &port,
        &document,
        "run",
        Some("direction"),
        Some("go"),
        None,
        None,
    )
    .expect("accept");
    for product in ["proof", "mutation", "land", "deliver", "recover"] {
        call(&port, &document, "run", Some(product), None, None, None)
            .unwrap_or_else(|err| panic!("{product}: {err}"));
    }
    let status = call(&port, &document, "status", None, None, None, None).expect("status");
    assert_eq!(field(&status.detail, "effects"), "5");
}

#[test]
fn resume_during_an_effect_keeps_the_ticket() {
    let entered = Arc::new(std::sync::Barrier::new(2));
    let release = Arc::new(std::sync::Barrier::new(2));
    let entered_hook = Arc::clone(&entered);
    let release_hook = Arc::clone(&release);
    let port = Arc::new(PublicLocalRoadmapPort::with_products(
        Arc::new(move |_| {
            entered_hook.wait();
            release_hook.wait();
            Ok(())
        }),
        Arc::new(|_| Ok(())),
    ));
    let document = plan("prove-document prove-doc\n");
    call(&port, &document, "run", None, None, None, None).expect("publish");
    call(
        &port,
        &document,
        "run",
        Some("direction"),
        Some("go"),
        None,
        None,
    )
    .expect("accept");
    let worker_port = Arc::clone(&port);
    let worker_document = document.clone();
    let worker = thread::spawn(move || {
        call(
            &worker_port,
            &worker_document,
            "run",
            Some("proof"),
            None,
            None,
            None,
        )
    });
    entered.wait();
    let resumed = call(&port, &document, "resume", None, None, None, None);
    assert_eq!(resumed.unwrap_err(), "an effect is already admitted");
    release.wait();
    let proved = worker.join().expect("worker").expect("proof");
    assert_eq!(field(&proved.detail, "outcome"), "admitted");
    let status = call(&port, &document, "status", None, None, None, None).expect("status");
    assert_eq!(field(&status.detail, "effects"), "1");
}

#[test]
fn replay_during_an_effect_does_not_hold_the_service() {
    let entered = Arc::new(std::sync::Barrier::new(2));
    let release = Arc::new(std::sync::Barrier::new(2));
    let entered_hook = Arc::clone(&entered);
    let release_hook = Arc::clone(&release);
    let port = Arc::new(PublicLocalRoadmapPort::with_products(
        Arc::new(move |_| {
            entered_hook.wait();
            release_hook.wait();
            Ok(())
        }),
        Arc::new(|_| Ok(())),
    ));
    let document = plan("prove-document prove-doc\n");
    call(&port, &document, "run", None, None, None, None).expect("publish");
    call(
        &port,
        &document,
        "run",
        Some("direction"),
        Some("go"),
        None,
        None,
    )
    .expect("accept");
    let worker_port = Arc::clone(&port);
    let worker_document = document.clone();
    let worker = thread::spawn(move || {
        call(
            &worker_port,
            &worker_document,
            "run",
            Some("proof"),
            None,
            None,
            None,
        )
    });
    entered.wait();
    let replay_port = Arc::clone(&port);
    let replay_document = document.clone();
    let replayed = thread::spawn(move || {
        call(
            &replay_port,
            &replay_document,
            "replay",
            None,
            None,
            None,
            None,
        )
    });
    let deadline = Instant::now() + Duration::from_secs(8);
    while !replayed.is_finished() {
        assert!(Instant::now() < deadline, "replay held the service");
        thread::sleep(Duration::from_millis(20));
    }
    let replayed = replayed.join().expect("replay thread");
    assert_eq!(replayed.unwrap_err(), "an effect is already admitted");
    let resumed = call(&port, &document, "resume", None, None, None, None);
    assert_eq!(resumed.unwrap_err(), "an effect is already admitted");
    release.wait();
    let proved = worker.join().expect("worker").expect("proof");
    assert_eq!(field(&proved.detail, "outcome"), "admitted");
}

#[test]
fn unknown_plan_line_is_refused() {
    let port = PublicLocalRoadmapPort::default();
    let document = plan("surprise token\n");
    let refused = call(&port, &document, "run", None, None, None, None);
    assert_eq!(refused.unwrap_err(), "selected plan has unknown surprise");
}

#[test]
fn status_and_replay_do_not_wait_on_each_other() {
    let port = Arc::new(PublicLocalRoadmapPort::default());
    let document = plan("");
    call(&port, &document, "run", None, None, None, None).expect("publish");
    let left = Arc::clone(&port);
    let right = Arc::clone(&port);
    let left_document = document.clone();
    let right_document = document.clone();
    let status = thread::spawn(move || {
        for _ in 0..80 {
            call(&left, &left_document, "status", None, None, None, None).expect("status");
            call(&left, &left_document, "explain", None, None, None, None).expect("explain");
        }
    });
    let replay = thread::spawn(move || {
        for _ in 0..80 {
            call(&right, &right_document, "replay", None, None, None, None).expect("replay");
        }
    });
    let deadline = Instant::now() + Duration::from_secs(8);
    while !status.is_finished() || !replay.is_finished() {
        assert!(Instant::now() < deadline, "status and replay deadlocked");
        thread::sleep(Duration::from_millis(20));
    }
    status.join().expect("status thread");
    replay.join().expect("replay thread");
}
