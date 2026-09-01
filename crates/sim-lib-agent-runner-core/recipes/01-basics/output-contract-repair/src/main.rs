use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use sim_kernel::{Cx, DefaultFactory, Expr, NoopEvalPolicy, Result, Symbol};
use sim_lib_agent_runner_core::{
    ModelCard, ModelRequest, ModelResponse, ModelRunner, OutputContract,
};
use sim_shape::{ExprKind, ExprKindShape, GrammarDialect, Shape};

fn main() -> Result<()> {
    let mut cx = Cx::new(
        Arc::new(NoopEvalPolicy),
        Arc::new(DefaultFactory),
        sim_kernel::HandleSeed::new(0x7ccd_50fc_a0ab_8385),
    );
    let shape = ExprKindShape::new(ExprKind::Bool);
    let shape_expr = Expr::Symbol(Symbol::qualified("shape", "Bool"));
    let contract = OutputContract::for_shape(q("codec", "bridge"), shape_expr, &shape, true);
    let providers = providers();

    let graph_present = contract.grammar_graph.is_some();
    let schema_provider = select_provider(&contract, &providers);

    let mut gbnf_contract = contract.clone();
    gbnf_contract.grammar = Some("root ::= \"true\" | \"false\"".to_owned());
    gbnf_contract.grammar_dialect = Some(GrammarDialect::Gbnf);
    let gbnf_provider = select_provider(&gbnf_contract, &providers);

    let runner = FakeRunner::new(
        q("runner", "fake-output-contract"),
        vec![Expr::String("not-bool".to_owned()), Expr::Bool(true)],
    );
    let (response, attempts) = repair_until_accepted(&mut cx, &runner, &shape, contract, 2)?;
    let accepted = response.content.first().expect("accepted response content");

    println!("graph metadata present: {graph_present}");
    println!("json-schema provider: {}", schema_provider.name);
    println!("gbnf provider: {}", gbnf_provider.name);
    println!("repair attempts: {attempts}");
    println!("accepted output: {accepted:?}");
    Ok(())
}

fn repair_until_accepted(
    cx: &mut Cx,
    runner: &dyn ModelRunner,
    shape: &dyn Shape,
    contract: OutputContract,
    max_repairs: usize,
) -> Result<(ModelResponse, usize)> {
    let mut request = ModelRequest::new(Expr::String("return a boolean".to_owned()), Vec::new());
    contract.into_extra_entries(&mut request.extra);

    for attempt in 1..=max_repairs + 1 {
        let response = runner.infer(cx, request.clone())?;
        if let Some(expr) = response.content.first() {
            let matched = shape.check_expr(cx, expr)?;
            if matched.accepted {
                return Ok((response, attempt));
            }
        }
        request.messages.push(Expr::String(
            "repair: output must match the bool shape".to_owned(),
        ));
    }
    panic!("fake runner did not repair output within the bound");
}

struct Provider {
    name: &'static str,
    dialects: &'static [GrammarDialect],
}

fn providers() -> [Provider; 2] {
    [
        Provider {
            name: "schema-runner",
            dialects: &[GrammarDialect::JsonSchema],
        },
        Provider {
            name: "gbnf-runner",
            dialects: &[GrammarDialect::Gbnf],
        },
    ]
}

fn select_provider<'a>(contract: &OutputContract, providers: &'a [Provider]) -> &'a Provider {
    let dialect = contract
        .grammar_dialect
        .expect("output contract should name a grammar dialect");
    providers
        .iter()
        .find(|provider| provider.dialects.contains(&dialect))
        .expect("provider for grammar dialect")
}

struct FakeRunner {
    runner: Symbol,
    outputs: Mutex<VecDeque<Expr>>,
}

impl FakeRunner {
    fn new(runner: Symbol, outputs: Vec<Expr>) -> Self {
        Self {
            runner,
            outputs: Mutex::new(outputs.into()),
        }
    }
}

impl ModelRunner for FakeRunner {
    fn card(&self) -> ModelCard {
        ModelCard::new(
            self.runner.clone(),
            "fake-repair",
            Symbol::new("fake"),
            Symbol::new("local"),
        )
    }

    fn infer(&self, _cx: &mut Cx, _request: ModelRequest) -> Result<ModelResponse> {
        let output = self
            .outputs
            .lock()
            .expect("fake output queue")
            .pop_front()
            .unwrap_or(Expr::Bool(true));
        Ok(ModelResponse::new(
            self.runner.clone(),
            "fake-repair",
            vec![output],
            Symbol::new("stop"),
        ))
    }
}

fn q(namespace: &str, name: &str) -> Symbol {
    Symbol::qualified(namespace, name)
}
