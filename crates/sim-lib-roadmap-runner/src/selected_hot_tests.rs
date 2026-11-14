// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Hot command samples through the loaded roadmap command.

use std::time::Instant;

use sim_kernel::{Args, Cx, Expr, Symbol, Value};

use crate::LocalRoadmapRunnerLib;

const SAMPLES: usize = 20;
const HOT_CEILING_US: u128 = 300_000_000;

fn within_hot_ceiling(elapsed_us: u128) -> bool {
    elapsed_us <= HOT_CEILING_US
}

fn current_plan() -> String {
    "\
sim-selected-plan-v1
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
observation fact/src current
observation fact/out current
"
    .into()
}

fn expired_plan() -> String {
    current_plan().replace(
        "observation fact/src current",
        "observation fact/src expired",
    )
}

fn bare_plan() -> String {
    current_plan()
        .lines()
        .filter(|line| !line.starts_with("observation "))
        .collect::<Vec<_>>()
        .join("\n")
}

fn call(cx: &mut Cx, verb: &str, execution: &str, plan: &str) -> Result<Value, String> {
    let text = [
        "roadmap".to_owned(),
        verb.to_owned(),
        "--conduct".to_owned(),
        "conduct".to_owned(),
        "--model-pick".to_owned(),
        "harbor".to_owned(),
        "--proof-catalog".to_owned(),
        "catalog".to_owned(),
        "--runner-generation".to_owned(),
        "generation-1".to_owned(),
        "--execution".to_owned(),
        execution.to_owned(),
        "--plan".to_owned(),
        plan.to_owned(),
    ];
    let args = cx
        .factory()
        .list(
            text.into_iter()
                .map(|part| cx.factory().string(part).expect("string"))
                .collect(),
        )
        .expect("list");
    let envelope = cx
        .factory()
        .table(vec![(Symbol::new("args"), args)])
        .expect("table");
    let entry = Symbol::qualified("cli", "main/roadmap");
    cx.call_function(&entry, Args::new(vec![envelope]))
        .map_err(|err| err.to_string())
}

fn field(cx: &mut Cx, value: &Value, name: &str) -> String {
    let table = value.object().as_table_impl().expect("table");
    let got = table.get(cx, Symbol::new(name)).expect(name);
    match got.object().as_expr(cx).expect(name) {
        Expr::String(text) => text,
        other => panic!("field {name} is {other:?}"),
    }
}

fn detail_field(detail: &str, name: &str) -> String {
    detail
        .split_whitespace()
        .find_map(|item| item.strip_prefix(&format!("{name}=")))
        .unwrap_or("")
        .to_owned()
}

fn acknowledged(cx: &mut Cx, value: &Value) -> bool {
    let table = value.object().as_table_impl().expect("table");
    let got = table
        .get(cx, Symbol::new("journal-acknowledged"))
        .expect("journal-acknowledged");
    match got.object().as_expr(cx).expect("journal-acknowledged") {
        Expr::Bool(value) => value,
        other => panic!("field journal-acknowledged is {other:?}"),
    }
}

fn assert_quiescent(cx: &mut Cx, value: &Value, cache: &str) {
    let outcome = field(cx, value, "outcome");
    let detail = field(cx, value, "detail");
    assert_eq!(outcome, "ok");
    assert!(acknowledged(cx, value));
    assert_eq!(detail_field(&detail, "product"), "hot");
    assert_eq!(detail_field(&detail, "outcome"), "quiescent");
    assert_eq!(detail_field(&detail, "effects"), "0");
    assert_eq!(detail_field(&detail, "parsed"), "1");
    assert_eq!(detail_field(&detail, "bound"), "1");
    assert_eq!(detail_field(&detail, "cache"), cache);
    assert_eq!(detail_field(&detail, "serialized"), "1");
    assert_eq!(detail_field(&detail, "generator"), "0");
    assert_eq!(detail_field(&detail, "mutation"), "0");
    assert_eq!(detail_field(&detail, "outward"), "0");
    assert!(detail_field(&detail, "seal").is_empty());
    assert_eq!(detail_field(&detail, "opened").len(), 64);
    assert!(!detail.is_empty());
}

#[test]
fn hot_noop_ceiling_admits_bound_and_refuses_next() {
    assert!(within_hot_ceiling(HOT_CEILING_US));
    assert!(!within_hot_ceiling(HOT_CEILING_US + 1));
    assert_eq!(HOT_CEILING_US, 300_000_000);
    assert_eq!(HOT_CEILING_US + 1, 300_000_001);
}

#[test]
fn hot_noop_expired_observation_returns_refresh_need() {
    let mut cx = sim_kernel::testing::bare_cx();
    cx.load_lib(&LocalRoadmapRunnerLib::new()).unwrap();
    let expired = call(&mut cx, "run", "local/hot-refresh", &expired_plan()).expect("refresh");
    assert_eq!(field(&mut cx, &expired, "outcome"), "refresh-need");
    assert!(!acknowledged(&mut cx, &expired));
    let detail = field(&mut cx, &expired, "detail");
    assert_eq!(detail_field(&detail, "outcome"), "refresh-need");
    assert_eq!(detail_field(&detail, "effects"), "0");
    assert_eq!(detail_field(&detail, "parsed"), "1");
    assert_eq!(detail_field(&detail, "bound"), "0");
    assert_eq!(detail_field(&detail, "cache"), "skipped");
    assert_eq!(detail_field(&detail, "generator"), "0");
    assert_eq!(detail_field(&detail, "mutation"), "0");
    assert_eq!(detail_field(&detail, "outward"), "0");
    let admitted = call(&mut cx, "run", "local/hot-refresh", &current_plan()).expect("admit");
    assert_quiescent(&mut cx, &admitted, "admitted");
}

#[test]
fn hot_noop_unknown_observation_is_refused() {
    let mut cx = sim_kernel::testing::bare_cx();
    cx.load_lib(&LocalRoadmapRunnerLib::new()).unwrap();
    let plan = current_plan().replace("observation fact/src current", "observation fact/src stale");
    let refused = call(&mut cx, "status", "local/hot-stale", &plan).expect_err("stale");
    assert!(refused.contains("not current or expired"), "{refused}");
}

#[test]
fn hot_noop_absent_observations_keep_direction() {
    let mut cx = sim_kernel::testing::bare_cx();
    cx.load_lib(&LocalRoadmapRunnerLib::new()).unwrap();
    let published = call(&mut cx, "run", "local/hot-direction", &bare_plan()).expect("publish");
    let detail = field(&mut cx, &published, "detail");
    assert_eq!(field(&mut cx, &published, "outcome"), "ok");
    assert_eq!(detail_field(&detail, "outcome"), "need-direction");
    assert_eq!(detail_field(&detail, "effects"), "0");
    assert_ne!(detail_field(&detail, "product"), "hot");
}

#[test]
fn hot_noop_twenty_samples_stay_inside_five_minutes() {
    let mut cx = sim_kernel::testing::bare_cx();
    cx.load_lib(&LocalRoadmapRunnerLib::new()).unwrap();
    let plan = current_plan();
    let warm = call(&mut cx, "status", "local/hot-noop", &plan).expect("warm");
    assert_quiescent(&mut cx, &warm, "admitted");
    let mut raw_us = Vec::with_capacity(SAMPLES);
    let mut effects = 0u64;
    let mut generator = 0u64;
    let mut mutation = 0u64;
    let mut outward = 0u64;
    let mut bound = 0usize;
    let mut hits = 0usize;
    let mut quiescent = 0usize;
    let mut serialized = 0usize;
    for _ in 0..SAMPLES {
        let started = Instant::now();
        let value = call(&mut cx, "status", "local/hot-noop", &plan).expect("sample");
        let elapsed_us = started.elapsed().as_micros();
        assert!(
            within_hot_ceiling(elapsed_us),
            "sample {elapsed_us} exceeds {HOT_CEILING_US}"
        );
        assert_quiescent(&mut cx, &value, "hit");
        let detail = field(&mut cx, &value, "detail");
        effects += count(&detail, "effects");
        generator += count(&detail, "generator");
        mutation += count(&detail, "mutation");
        outward += count(&detail, "outward");
        bound += usize::from(detail_field(&detail, "bound") == "1");
        hits += usize::from(detail_field(&detail, "cache") == "hit");
        quiescent += usize::from(detail_field(&detail, "outcome") == "quiescent");
        serialized += usize::from(detail_field(&detail, "serialized") == "1");
        raw_us.push(elapsed_us);
    }
    assert_eq!(raw_us.len(), SAMPLES);
    let max_us = raw_us.iter().copied().max().unwrap_or(0);
    let within = raw_us.iter().all(|sample| within_hot_ceiling(*sample));
    let raw = raw_us
        .iter()
        .map(u128::to_string)
        .collect::<Vec<_>>()
        .join(" ");
    println!(
        "hot-noop samples {SAMPLES} warm 1 effects {effects} bound {bound} cache-hit {hits} quiescent {quiescent} serialized {serialized} generator {generator} mutation {mutation} outward {outward} ceiling-us {HOT_CEILING_US} admits-us {HOT_CEILING_US} refuses-us {} max-us {max_us} within {within} raw-us {raw}",
        HOT_CEILING_US + 1
    );
}

fn count(detail: &str, name: &str) -> u64 {
    detail_field(detail, name).parse().expect(name)
}
