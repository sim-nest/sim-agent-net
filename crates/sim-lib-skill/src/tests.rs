use std::sync::Arc;

use sim_kernel::{
    Args, ClaimPattern, Cx, DefaultFactory, EagerPolicy, Error, Expr, Ref, Symbol, Value,
};
use sim_shape::{AnyShape, ListShape, NumberValueShape, shape_value};
use sim_value::build::entry;

use crate::{
    FixtureBehavior, FixtureSkillSpec, FixtureTransport, SkillCacheMode, SkillCard,
    SkillCassetteMode, SkillPolicy, SkillPrivacyPolicy, install_skill_lib, skill_call_capability,
    skill_call_symbol, skill_card_symbol, skill_install_capability, skill_install_symbol,
    skill_list_symbol, skill_specific_call_capability, skill_transport_value,
};
#[cfg(all(feature = "cache", feature = "cassette"))]
use crate::{skill_audit_capability, skill_audit_symbol};

#[test]
fn skill_call_uses_fixture_transport_through_skill_callable() {
    let mut cx = skill_cx();
    cx.grant(skill_install_capability());
    cx.grant(skill_call_capability());
    cx.grant(skill_specific_call_capability("math.add"));
    let fixture = Arc::new(FixtureTransport::new("math"));
    fixture.insert("add", FixtureBehavior::SumNumbers).unwrap();
    let card = sum_card("math.add");
    install_with_op(&mut cx, fixture.clone(), card);

    let target = cx.factory().string("math.add".to_owned()).unwrap();
    let left = number_value(&mut cx, 2);
    let right = number_value(&mut cx, 3);

    let result = cx
        .call_function(&skill_call_symbol(), Args::new(vec![target, left, right]))
        .unwrap();

    assert_eq!(number_expr(&mut cx, result), "5");
    assert_eq!(fixture.call_count(), 1);
}

#[test]
fn skill_list_and_card_return_installed_skill_cards() {
    let mut cx = skill_cx();
    cx.grant(skill_install_capability());
    let fixture = Arc::new(FixtureTransport::new("math"));
    fixture.insert("add", FixtureBehavior::SumNumbers).unwrap();
    let card_to_install = sum_card("math.add");
    install_with_op(&mut cx, fixture, card_to_install);

    let listed = cx
        .call_function(&skill_list_symbol(), Args::default())
        .unwrap();
    let Expr::List(items) = listed.object().as_expr(&mut cx).unwrap() else {
        panic!("skill/list should return a list");
    };
    assert_eq!(items.len(), 1);

    let target = cx.factory().string("math.add".to_owned()).unwrap();
    let card = cx
        .call_function(&skill_card_symbol(), Args::new(vec![target]))
        .unwrap();
    assert_eq!(
        table_field(&mut cx, card, "id"),
        Expr::String("math.add".to_owned())
    );
}

#[test]
fn input_shape_failure_does_not_call_transport() {
    let mut cx = skill_cx();
    cx.grant(skill_install_capability());
    cx.grant(skill_call_capability());
    cx.grant(skill_specific_call_capability("math.add"));
    let fixture = Arc::new(FixtureTransport::new("math"));
    fixture.insert("add", FixtureBehavior::SumNumbers).unwrap();
    let card = sum_card("math.add");
    install_with_op(&mut cx, fixture.clone(), card);

    let target = cx.factory().string("math.add".to_owned()).unwrap();
    let invalid = cx.factory().string("bad".to_owned()).unwrap();
    let valid = number_value(&mut cx, 3);

    let err = cx
        .call_function(
            &skill_call_symbol(),
            Args::new(vec![target, invalid, valid]),
        )
        .unwrap_err();

    assert_wrong_shape(err);
    assert_eq!(fixture.call_count(), 0);
}

#[test]
fn output_shape_failure_is_reported_after_one_transport_call() {
    let mut cx = skill_cx();
    cx.grant(skill_install_capability());
    cx.grant(skill_call_capability());
    cx.grant(skill_specific_call_capability("bad.output"));
    let fixture = Arc::new(FixtureTransport::new("bad"));
    fixture
        .insert(
            "text",
            FixtureBehavior::ConstantString("not a number".to_owned()),
        )
        .unwrap();
    let card = SkillCard::fixture(FixtureSkillSpec {
        id: "bad.output".to_owned(),
        symbol: Symbol::qualified("skill", "bad.output"),
        title: "Bad Output".to_owned(),
        description: "Returns a string where a number is expected.".to_owned(),
        input_shape: sum_args_shape(),
        output_shape: number_shape("bad-output-result"),
        transport_id: "bad".to_owned(),
        operation: "text".to_owned(),
    });
    install_with_op(&mut cx, fixture.clone(), card);

    let target = cx.factory().string("bad.output".to_owned()).unwrap();
    let left = number_value(&mut cx, 1);
    let right = number_value(&mut cx, 2);
    let err = cx
        .call_function(&skill_call_symbol(), Args::new(vec![target, left, right]))
        .unwrap_err();

    assert_wrong_shape(err);
    assert_eq!(fixture.call_count(), 1);
}

#[test]
fn core_crate_has_no_mandatory_agent_openai_server_network_or_process_dependencies() {
    let cargo = include_str!("../Cargo.toml");
    let dependencies = cargo
        .split("[dependencies]")
        .nth(1)
        .and_then(|tail| tail.split("[dev-dependencies]").next())
        .unwrap();
    for forbidden in [
        "sim-lib-agent",
        "sim-lib-openai-server",
        "sim-lib-server",
        "sim-lib-agent-runner-http",
        "sim-lib-agent-runner-process",
    ] {
        for line in dependencies.lines().filter(|line| line.contains(forbidden)) {
            assert!(
                line.contains("optional = true"),
                "core skill crate must not require {forbidden}"
            );
        }
    }
}

#[test]
fn skill_card_expr_conversion_round_trips_public_descriptor() {
    let mut cx = skill_cx();
    let card = sum_card("math.add");
    let expr = card.to_expr(&mut cx).unwrap();

    let decoded = SkillCard::from_expr(&expr).unwrap();

    assert_eq!(decoded.to_expr(&mut cx).unwrap(), expr);
}

#[test]
fn skill_card_absent_policy_defaults() {
    let mut expr = skill_card_expr();
    remove_expr_field(&mut expr, "policy");

    let decoded = SkillCard::from_expr(&expr).unwrap();

    assert_eq!(decoded.policy, SkillPolicy::default());
}

#[test]
fn skill_card_malformed_policy_rejects_even_when_string_keyed() {
    let mut expr = skill_card_expr();
    remove_expr_field(&mut expr, "policy");
    push_expr_field(
        &mut expr,
        Expr::String("policy".to_owned()),
        Expr::String("not a policy map".to_owned()),
    );

    let err = skill_card_decode_err(&expr);

    assert!(err.to_string().contains("SkillCard policy"));
}

#[test]
fn skill_card_malformed_policy_value_rejects() {
    let mut expr = skill_card_expr();
    replace_expr_field(
        &mut expr,
        "policy",
        Expr::Map(vec![entry("privacy", Expr::Bool(true))]),
    );

    let err = skill_card_decode_err(&expr);

    assert!(err.to_string().contains("invalid policy value"));
}

#[test]
fn skill_card_unknown_policy_key_rejects() {
    let mut expr = skill_card_expr();
    replace_expr_field(
        &mut expr,
        "policy",
        Expr::Map(vec![
            entry("privacy", Expr::Symbol(Symbol::new("no-raw"))),
            entry("unknown", Expr::Bool(true)),
        ]),
    );

    let err = skill_card_decode_err(&expr);

    assert!(err.to_string().contains("unknown field unknown"));
}

#[test]
fn skill_card_policy_subfields_default_independently() {
    let mut expr = skill_card_expr();
    replace_expr_field(
        &mut expr,
        "policy",
        Expr::Map(vec![entry(
            "cache",
            Expr::Symbol(Symbol::new("read-through")),
        )]),
    );

    let decoded = SkillCard::from_expr(&expr).unwrap();

    assert_eq!(decoded.policy.privacy, SkillPrivacyPolicy::NoRaw);
    assert_eq!(decoded.policy.cache, SkillCacheMode::ReadThrough);
    assert_eq!(decoded.policy.cassette, SkillCassetteMode::Disabled);
    assert!(!decoded.policy.idempotent);
    assert_eq!(decoded.policy.semantic_key, None);
}

#[test]
fn skill_install_accepts_decoded_skill_card_expression() {
    let mut cx = skill_cx();
    cx.grant(skill_install_capability());
    cx.grant(skill_call_capability());
    cx.grant(skill_specific_call_capability("echo.args"));
    let fixture = Arc::new(FixtureTransport::new("echo"));
    fixture.insert("args", FixtureBehavior::EchoArgs).unwrap();
    let card = SkillCard::fixture(FixtureSkillSpec {
        id: "echo.args".to_owned(),
        symbol: Symbol::qualified("skill", "echo.args"),
        title: "Echo Args".to_owned(),
        description: "Echo argument lists through the fixture transport.".to_owned(),
        input_shape: any_shape(),
        output_shape: any_shape(),
        transport_id: "echo".to_owned(),
        operation: "args".to_owned(),
    });
    let transport = skill_transport_value(&mut cx, fixture.clone()).unwrap();
    let expr = card.to_expr(&mut cx).unwrap();
    let card = cx.factory().expr(expr).unwrap();
    cx.call_function(&skill_install_symbol(), Args::new(vec![transport, card]))
        .unwrap();

    let target = cx.factory().string("echo.args".to_owned()).unwrap();
    let value = cx.factory().string("payload".to_owned()).unwrap();
    let result = cx
        .call_function(&skill_call_symbol(), Args::new(vec![target, value]))
        .unwrap();

    assert!(matches!(
        result.object().as_expr(&mut cx).unwrap(),
        Expr::List(items) if items == vec![Expr::String("payload".to_owned())]
    ));
    assert_eq!(fixture.call_count(), 1);
}

#[test]
fn binding_skill_card_publishes_browse_claims_for_callable_symbol() {
    let mut cx = skill_cx();
    cx.grant(skill_install_capability());
    let fixture = Arc::new(FixtureTransport::new("math"));
    fixture.insert("add", FixtureBehavior::SumNumbers).unwrap();
    let card = sum_card("math.add");
    install_with_op(&mut cx, fixture, card);

    let claims = cx
        .query_facts(ClaimPattern {
            subject: Some(Ref::Symbol(Symbol::qualified("skill", "math.add"))),
            predicate: Some(sim_kernel::card::card_kind_predicate()),
            object: None,
            include_revoked: false,
        })
        .unwrap();

    assert!(
        claims
            .iter()
            .any(|claim| { claim.object == Ref::Symbol(Symbol::qualified("skill", "card")) })
    );
}

#[cfg(all(feature = "cache", feature = "cassette"))]
#[test]
fn repeated_idempotent_skill_call_hits_cache() {
    let mut cx = skill_cx();
    grant_call_caps(&mut cx, "math.cached");
    let fixture = Arc::new(FixtureTransport::new("math"));
    fixture.insert("add", FixtureBehavior::SumNumbers).unwrap();
    let card = sum_card("math.cached")
        .with_cache_mode(SkillCacheMode::ReadThrough)
        .with_idempotent(true)
        .with_semantic_key("math.add.v1");
    install_with_op(&mut cx, fixture.clone(), card);
    let target = cx.factory().string("math.cached".to_owned()).unwrap();

    let first = call_two_numbers(&mut cx, target.clone(), 2, 3);
    let second = call_two_numbers(&mut cx, target, 2, 3);

    assert_eq!(number_expr(&mut cx, first), "5");
    assert_eq!(number_expr(&mut cx, second), "5");
    assert_eq!(fixture.call_count(), 1);
}

#[cfg(all(feature = "cache", feature = "cassette"))]
#[test]
fn cassette_replay_returns_recorded_value_without_transport_call() {
    let mut cx = skill_cx();
    grant_call_caps(&mut cx, "cassette.echo");
    let fixture = Arc::new(FixtureTransport::new("cassette"));
    fixture
        .insert(
            "echo",
            FixtureBehavior::ConstantString("recorded".to_owned()),
        )
        .unwrap();
    let card = SkillCard::fixture(FixtureSkillSpec {
        id: "cassette.echo".to_owned(),
        symbol: Symbol::qualified("skill", "cassette.echo"),
        title: "Cassette Echo".to_owned(),
        description: "Records one response for replay.".to_owned(),
        input_shape: any_shape(),
        output_shape: any_shape(),
        transport_id: "cassette".to_owned(),
        operation: "echo".to_owned(),
    })
    .with_cassette_mode(SkillCassetteMode::RecordReplay);
    install_with_op(&mut cx, fixture.clone(), card);
    let target = cx.factory().string("cassette.echo".to_owned()).unwrap();
    let payload = cx.factory().string("payload".to_owned()).unwrap();

    let first = cx
        .call_function(
            &skill_call_symbol(),
            Args::new(vec![target.clone(), payload.clone()]),
        )
        .unwrap();
    fixture
        .insert(
            "echo",
            FixtureBehavior::ConstantString("changed".to_owned()),
        )
        .unwrap();
    let second = cx
        .call_function(&skill_call_symbol(), Args::new(vec![target, payload]))
        .unwrap();

    assert_eq!(
        first.object().as_expr(&mut cx).unwrap(),
        Expr::String("recorded".to_owned())
    );
    assert_eq!(
        second.object().as_expr(&mut cx).unwrap(),
        Expr::String("recorded".to_owned())
    );
    assert_eq!(fixture.call_count(), 1);
}

#[cfg(all(feature = "cache", feature = "cassette"))]
#[test]
fn skill_audit_redacts_raw_payloads() {
    let mut cx = skill_cx();
    grant_call_caps(&mut cx, "audit.echo");
    cx.grant(skill_audit_capability());
    let fixture = Arc::new(FixtureTransport::new("audit"));
    fixture.insert("echo", FixtureBehavior::EchoArgs).unwrap();
    let card = SkillCard::fixture(FixtureSkillSpec {
        id: "audit.echo".to_owned(),
        symbol: Symbol::qualified("skill", "audit.echo"),
        title: "Audit Echo".to_owned(),
        description: "Echoes arguments while audit stays redacted.".to_owned(),
        input_shape: any_shape(),
        output_shape: any_shape(),
        transport_id: "audit".to_owned(),
        operation: "echo".to_owned(),
    });
    install_with_op(&mut cx, fixture, card);
    let target = cx.factory().string("audit.echo".to_owned()).unwrap();
    let secret = "secret-token";
    let payload = cx.factory().string(secret.to_owned()).unwrap();

    cx.call_function(&skill_call_symbol(), Args::new(vec![target, payload]))
        .unwrap();
    let audit = cx
        .call_function(&skill_audit_symbol(), Args::default())
        .unwrap();
    let rendered = format!("{:?}", audit.object().as_expr(&mut cx).unwrap());

    assert!(rendered.contains("audit-entry"));
    assert!(!rendered.contains(secret));
    assert!(rendered.contains("redacted"));
}

fn skill_cx() -> Cx {
    let mut cx = Cx::new(Arc::new(EagerPolicy), Arc::new(DefaultFactory));
    install_skill_lib(&mut cx).unwrap();
    cx
}

fn skill_card_expr() -> Expr {
    let mut cx = skill_cx();
    sum_card("math.add").to_expr(&mut cx).unwrap()
}

fn remove_expr_field(expr: &mut Expr, name: &str) {
    map_entries_mut(expr).retain(|(key, _)| !expr_key_is(key, name));
}

fn push_expr_field(expr: &mut Expr, key: Expr, value: Expr) {
    map_entries_mut(expr).push((key, value));
}

fn replace_expr_field(expr: &mut Expr, name: &str, value: Expr) {
    let entries = map_entries_mut(expr);
    let Some((_, existing)) = entries.iter_mut().find(|(key, _)| expr_key_is(key, name)) else {
        panic!("missing test field {name}");
    };
    *existing = value;
}

fn map_entries_mut(expr: &mut Expr) -> &mut Vec<(Expr, Expr)> {
    let Expr::Map(entries) = expr else {
        panic!("test card expression must be a map");
    };
    entries
}

fn expr_key_is(key: &Expr, name: &str) -> bool {
    match key {
        Expr::Symbol(symbol) if symbol.namespace.is_none() => symbol.name.as_ref() == name,
        Expr::String(text) => text == name,
        _ => false,
    }
}

fn skill_card_decode_err(expr: &Expr) -> Error {
    match SkillCard::from_expr(expr) {
        Ok(_) => panic!("malformed SkillCard expression unexpectedly decoded"),
        Err(err) => err,
    }
}

#[cfg(all(feature = "cache", feature = "cassette"))]
fn grant_call_caps(cx: &mut Cx, id: &str) {
    cx.grant(skill_install_capability());
    cx.grant(skill_call_capability());
    cx.grant(skill_specific_call_capability(id));
}

fn install_with_op(cx: &mut Cx, fixture: Arc<FixtureTransport>, card: SkillCard) {
    let transport = skill_transport_value(cx, fixture).unwrap();
    let card = card.value(cx).unwrap();
    cx.call_function(&skill_install_symbol(), Args::new(vec![transport, card]))
        .unwrap();
}

fn sum_card(id: &str) -> SkillCard {
    SkillCard::fixture(FixtureSkillSpec {
        id: id.to_owned(),
        symbol: Symbol::qualified("skill", id.to_owned()),
        title: "Add Numbers".to_owned(),
        description: "Add two numbers with a fixture transport.".to_owned(),
        input_shape: sum_args_shape(),
        output_shape: number_shape("sum-result"),
        transport_id: "math".to_owned(),
        operation: "add".to_owned(),
    })
}

fn sum_args_shape() -> Value {
    shape_value(
        Symbol::qualified("skill", "sum-args"),
        Arc::new(ListShape::new(vec![
            Arc::new(NumberValueShape),
            Arc::new(NumberValueShape),
        ])),
    )
}

fn number_shape(name: &str) -> Value {
    shape_value(
        Symbol::qualified("skill", name.to_owned()),
        Arc::new(NumberValueShape),
    )
}

fn any_shape() -> Value {
    shape_value(Symbol::new("Any"), Arc::new(AnyShape))
}

fn number_value(cx: &mut Cx, value: u32) -> Value {
    cx.factory()
        .number_literal(Symbol::qualified("numbers", "f64"), value.to_string())
        .unwrap()
}

#[cfg(all(feature = "cache", feature = "cassette"))]
fn call_two_numbers(cx: &mut Cx, target: Value, left: u32, right: u32) -> Value {
    let left = number_value(cx, left);
    let right = number_value(cx, right);
    cx.call_function(&skill_call_symbol(), Args::new(vec![target, left, right]))
        .unwrap()
}

fn number_expr(cx: &mut Cx, value: Value) -> String {
    match value.object().as_expr(cx).unwrap() {
        Expr::Number(number) => number.canonical,
        other => panic!("expected number, got {other:?}"),
    }
}

fn assert_wrong_shape(err: Error) {
    let Error::WrongShape { diagnostics, .. } = err else {
        panic!("expected WrongShape");
    };
    assert!(!diagnostics.is_empty());
}

fn table_field(cx: &mut Cx, value: Value, field: &str) -> Expr {
    let Expr::Map(entries) = value.object().as_expr(cx).unwrap() else {
        panic!("expected map");
    };
    entries
        .into_iter()
        .find_map(|(key, value)| match key {
            Expr::Symbol(symbol) if symbol.namespace.is_none() && symbol.name.as_ref() == field => {
                Some(value)
            }
            _ => None,
        })
        .unwrap_or_else(|| panic!("missing field {field}"))
}
