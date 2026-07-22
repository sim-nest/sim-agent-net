use std::sync::Arc;

use sim_kernel::{
    CapabilityName, Claim, ClaimPattern, Cx, Datum, Expr, Ref, Result, Symbol, Test, TestReport,
    Value,
};

#[cfg(feature = "runner")]
use crate::skill_as_runner_symbol;
#[cfg(feature = "agent")]
use crate::skill_as_tool_symbol;
#[cfg(any(feature = "cache", feature = "cassette"))]
use crate::skill_audit_symbol;
#[cfg(feature = "serve")]
use crate::skill_serve_mcp_symbol;
use crate::{
    SkillCard, manifest_name, ops::SkillFunctionKind, skill_bind_symbol, skill_call_symbol,
    skill_card_symbol, skill_install_symbol, skill_list_symbol,
};
#[cfg(feature = "openai")]
use crate::{skill_openai_tool_symbol, skill_openai_tools_symbol};

pub(crate) fn publish_skill_browse_metadata(cx: &mut Cx) -> Result<()> {
    for doc in skill_function_docs() {
        publish_help_doc(cx, &doc)?;
        register_example(cx, doc)?;
    }
    Ok(())
}

pub(crate) fn publish_card_claims(cx: &mut Cx, card: &SkillCard) -> Result<()> {
    let subject = Ref::Symbol(card.symbol.clone());
    insert_ref_claim(
        cx,
        &subject,
        sim_kernel::card::card_kind_predicate(),
        Ref::Symbol(Symbol::qualified("skill", "card")),
    )?;
    insert_datum_claim(
        cx,
        &subject,
        sim_kernel::card::card_help_predicate(),
        Datum::String(card.description.clone()),
    )?;
    insert_shape_claim(
        cx,
        &subject,
        sim_kernel::card::card_args_predicate(),
        &card.input_shape,
    )?;
    insert_shape_claim(
        cx,
        &subject,
        sim_kernel::card::card_result_predicate(),
        &card.output_shape,
    )?;
    insert_datum_claim(
        cx,
        &subject,
        sim_kernel::card::card_shape_known_predicate(),
        Datum::Bool(true),
    )?;
    for capability in &card.capabilities {
        insert_datum_claim(
            cx,
            &subject,
            sim_kernel::card::card_requires_predicate(),
            Datum::String(capability.as_str().to_owned()),
        )?;
    }
    insert_ref_claim(
        cx,
        &subject,
        sim_kernel::card::card_see_also_predicate(),
        Ref::Symbol(skill_call_symbol()),
    )
}

fn insert_shape_claim(cx: &mut Cx, subject: &Ref, predicate: Symbol, shape: &Value) -> Result<()> {
    if let Some(symbol) = shape.object().as_shape().and_then(|shape| shape.symbol()) {
        insert_ref_claim(cx, subject, predicate, Ref::Symbol(symbol))
    } else {
        let expr = shape.object().as_expr(cx)?;
        insert_datum_claim(cx, subject, predicate, Datum::try_from(expr)?)
    }
}

fn publish_help_doc(cx: &mut Cx, doc: &SkillFunctionDoc) -> Result<()> {
    let subject = Ref::Symbol(doc.symbol());
    let predicate = help_doc_predicate();
    if !claims_for(cx, &subject, predicate.clone())?.is_empty() {
        return Ok(());
    }
    let help = help_doc_expr(doc);
    insert_datum_claim(cx, &subject, predicate, Datum::try_from(help)?)
}

fn register_example(cx: &mut Cx, doc: SkillFunctionDoc) -> Result<()> {
    let name = doc.example_name();
    if cx.registry().registered_test(&name).is_some() {
        return Ok(());
    }
    cx.registry_mut().register_test(
        name,
        manifest_name(),
        Arc::new(SkillExample { doc: doc.clone() }),
        vec![doc.symbol()],
    )
}

fn insert_ref_claim(cx: &mut Cx, subject: &Ref, predicate: Symbol, object: Ref) -> Result<()> {
    cx.insert_fact(Claim::public(subject.clone(), predicate, object))?;
    Ok(())
}

fn insert_datum_claim(cx: &mut Cx, subject: &Ref, predicate: Symbol, object: Datum) -> Result<()> {
    let claim = Claim::content_object(cx.datum_store_mut(), subject.clone(), predicate, object)?;
    cx.insert_fact(claim)?;
    Ok(())
}

fn claims_for(cx: &Cx, subject: &Ref, predicate: Symbol) -> Result<Vec<Claim>> {
    cx.query_facts(ClaimPattern {
        subject: Some(subject.clone()),
        predicate: Some(predicate),
        object: None,
        include_revoked: false,
    })
}

fn help_doc_predicate() -> Symbol {
    Symbol::qualified("browse", "help-doc")
}

fn help_doc_expr(doc: &SkillFunctionDoc) -> Expr {
    Expr::Map(vec![
        (field("subject"), Expr::Symbol(doc.symbol())),
        (
            field("kind"),
            Expr::Symbol(Symbol::qualified("core", "function")),
        ),
        (field("summary"), Expr::String(doc.summary.to_owned())),
        (field("detail"), Expr::String(doc.detail.to_owned())),
        (field("exported-by"), Expr::Symbol(manifest_name())),
        (
            field("stability"),
            Expr::Symbol(Symbol::new("experimental")),
        ),
        (
            field("capabilities"),
            Expr::List(
                doc.capabilities
                    .iter()
                    .map(|capability| Expr::String((*capability).to_owned()))
                    .collect(),
            ),
        ),
        (field("demand"), Expr::List(Vec::new())),
        (
            field("see-also"),
            Expr::List(
                skill_core_symbols()
                    .into_iter()
                    .filter(|symbol| symbol != &doc.symbol())
                    .map(Expr::Symbol)
                    .collect(),
            ),
        ),
    ])
}

#[derive(Clone)]
struct SkillFunctionDoc {
    kind: SkillFunctionKind,
    summary: &'static str,
    detail: &'static str,
    capabilities: &'static [&'static str],
    example_expr: Expr,
}

impl SkillFunctionDoc {
    fn symbol(&self) -> Symbol {
        self.kind.symbol()
    }

    fn example_name(&self) -> Symbol {
        Symbol::qualified("skill-example", self.symbol().name.to_string())
    }
}

struct SkillExample {
    doc: SkillFunctionDoc,
}

impl Test for SkillExample {
    fn symbol(&self) -> Symbol {
        self.doc.example_name()
    }

    fn lib(&self) -> Symbol {
        manifest_name()
    }

    fn describe(&self, cx: &mut Cx) -> Result<Value> {
        cx.factory().table(vec![
            (Symbol::new("name"), cx.factory().symbol(self.symbol())?),
            (
                Symbol::new("subjects"),
                cx.factory()
                    .list(vec![cx.factory().symbol(self.doc.symbol())?])?,
            ),
            (Symbol::new("lib"), cx.factory().symbol(manifest_name())?),
            (
                Symbol::new("mode"),
                cx.factory().symbol(Symbol::qualified("test", "example"))?,
            ),
            (
                Symbol::new("expr"),
                cx.factory().expr(self.doc.example_expr.clone())?,
            ),
            (
                Symbol::new("expr-codec"),
                cx.factory().symbol(Symbol::qualified("codec", "lisp"))?,
            ),
            (Symbol::new("expected"), cx.factory().nil()?),
            (Symbol::new("expected-codec"), cx.factory().nil()?),
            (Symbol::new("expected-error"), cx.factory().nil()?),
            (Symbol::new("codecs"), cx.factory().list(Vec::new())?),
            (Symbol::new("example"), cx.factory().bool(true)?),
            (
                Symbol::new("capabilities"),
                cx.factory().list(
                    self.doc
                        .capabilities
                        .iter()
                        .map(|capability| {
                            cx.factory()
                                .symbol(CapabilityName::new(*capability).as_symbol())
                        })
                        .collect::<Result<Vec<_>>>()?,
                )?,
            ),
        ])
    }

    fn run(&self, cx: &mut Cx) -> Result<TestReport> {
        if let Some(missing) = self
            .doc
            .capabilities
            .iter()
            .map(|capability| CapabilityName::new(*capability))
            .find(|capability| !cx.capabilities().contains(capability))
        {
            return Ok(TestReport::skipped(
                self.symbol(),
                Some(format!("missing capability {missing}")),
            ));
        }
        Ok(TestReport::from_result(self.symbol(), true, None))
    }
}

fn skill_function_docs() -> Vec<SkillFunctionDoc> {
    let docs = vec![
        SkillFunctionDoc {
            kind: SkillFunctionKind::Install,
            summary: "installs a skill transport and binds allowed cards",
            detail: "skill/install accepts a SkillTransport value followed by one or more SkillCard descriptors, stores the transport in the runtime skill registry, and binds each card to its normal callable symbol.",
            capabilities: &["skill.install"],
            example_expr: Expr::List(vec![
                Expr::Symbol(skill_install_symbol()),
                Expr::Symbol(Symbol::qualified("example", "transport")),
                Expr::Symbol(Symbol::qualified("example", "card")),
            ]),
        },
        SkillFunctionDoc {
            kind: SkillFunctionKind::Bind,
            summary: "binds SkillCard values to installed skill transports",
            detail: "skill/bind accepts one or more SkillCard values whose transport id is already installed and registers each card symbol as a SkillCallable.",
            capabilities: &["skill.bind"],
            example_expr: Expr::List(vec![
                Expr::Symbol(skill_bind_symbol()),
                Expr::Symbol(Symbol::qualified("example", "card")),
            ]),
        },
        SkillFunctionDoc {
            kind: SkillFunctionKind::List,
            summary: "lists visible SkillCard values in the runtime registry",
            detail: "skill/list returns the currently bound SkillCard values. Each card exposes public descriptor metadata and can be browsed through its callable symbol.",
            capabilities: &[],
            example_expr: Expr::List(vec![Expr::Symbol(skill_list_symbol())]),
        },
        SkillFunctionDoc {
            kind: SkillFunctionKind::Card,
            summary: "fetches one SkillCard by id or callable symbol",
            detail: "skill/card accepts a stable skill id string or a callable symbol and returns the matching SkillCard value, or nil when no card is bound.",
            capabilities: &[],
            example_expr: Expr::List(vec![
                Expr::Symbol(skill_card_symbol()),
                Expr::String("math.add".to_owned()),
            ]),
        },
        SkillFunctionDoc {
            kind: SkillFunctionKind::Call,
            summary: "calls a bound skill through its SkillCallable",
            detail: "skill/call accepts a skill id or symbol followed by skill arguments, resolves the bound SkillCard, and executes the same SkillCallable registered for that card.",
            capabilities: &["skill.call"],
            example_expr: Expr::List(vec![
                Expr::Symbol(skill_call_symbol()),
                Expr::String("math.add".to_owned()),
                Expr::Number(sim_kernel::NumberLiteral {
                    domain: Symbol::qualified("numbers", "f64"),
                    canonical: "1".to_owned(),
                }),
                Expr::Number(sim_kernel::NumberLiteral {
                    domain: Symbol::qualified("numbers", "f64"),
                    canonical: "2".to_owned(),
                }),
            ]),
        },
    ];
    #[cfg(feature = "agent")]
    let docs = {
        let mut docs = docs;
        docs.push(SkillFunctionDoc {
            kind: SkillFunctionKind::AsTool,
            summary: "projects one bound skill into an agent Tool",
            detail: "skill/as-tool accepts a skill id, symbol, or SkillCard value, builds the existing sim-lib-agent Tool around the bound SkillCallable, registers it with the agent tool registry, and returns that Tool value.",
            capabilities: &[],
            example_expr: Expr::List(vec![
                Expr::Symbol(skill_as_tool_symbol()),
                Expr::String("math.add".to_owned()),
            ]),
        });
        docs
    };
    #[cfg(any(feature = "cache", feature = "cassette"))]
    let docs = {
        let mut docs = docs;
        docs.push(SkillFunctionDoc {
            kind: SkillFunctionKind::Audit,
            summary: "returns redacted recent skill call summaries",
            detail: "skill/audit returns recent SkillCallable executions with skill id, transport kind, policy status, and redacted input and output markers; raw payloads and credentials are not included.",
            capabilities: &["skill.audit"],
            example_expr: Expr::List(vec![Expr::Symbol(skill_audit_symbol())]),
        });
        docs
    };
    #[cfg(feature = "openai")]
    let docs = {
        let mut docs = docs;
        docs.extend([
            SkillFunctionDoc {
                kind: SkillFunctionKind::OpenAiTool,
                summary: "projects one bound skill into an OpenAI tool descriptor",
                detail: "skill/openai-tool accepts a skill id, symbol, or SkillCard value and returns an OpenAI function-tool request descriptor whose name resolves through the server-owned skill callable registry.",
                capabilities: &[],
                example_expr: Expr::List(vec![
                    Expr::Symbol(skill_openai_tool_symbol()),
                    Expr::String("math.add".to_owned()),
                ]),
            },
            SkillFunctionDoc {
                kind: SkillFunctionKind::OpenAiTools,
                summary: "lists OpenAI tool descriptors for selected bound skills",
                detail: "skill/openai-tools accepts zero or more skill ids, symbols, or SkillCard values, defaults to all tool-role skills, and returns OpenAI request descriptors accepted by the server-owned tool registry.",
                capabilities: &[],
                example_expr: Expr::List(vec![Expr::Symbol(skill_openai_tools_symbol())]),
            },
        ]);
        docs
    };
    #[cfg(feature = "runner")]
    let docs = {
        let mut docs = docs;
        docs.push(SkillFunctionDoc {
            kind: SkillFunctionKind::AsRunner,
            summary: "projects one model-role skill into a ModelRunner",
            detail: "skill/as-runner accepts a skill id, symbol, or SkillCard value with the model role and returns the existing sim-lib-agent runner component backed by the bound SkillCallable.",
            capabilities: &[],
            example_expr: Expr::List(vec![
                Expr::Symbol(skill_as_runner_symbol()),
                Expr::String("model.answer".to_owned()),
            ]),
        });
        docs
    };
    #[cfg(feature = "serve")]
    let docs = {
        let mut docs = docs;
        docs.push(SkillFunctionDoc {
            kind: SkillFunctionKind::ServeMcp,
            summary: "describes the MCP server projection for selected skills",
            detail: "skill/serve-mcp accepts optional profile and capability data, filters bound SkillCards by role and caller capabilities, and returns the in-process MCP serving projection consumed by MCP transports.",
            capabilities: &["skill.serve"],
            example_expr: Expr::List(vec![Expr::Symbol(skill_serve_mcp_symbol())]),
        });
        docs
    };
    docs
}

fn skill_core_symbols() -> Vec<Symbol> {
    let symbols = vec![
        skill_install_symbol(),
        skill_bind_symbol(),
        skill_list_symbol(),
        skill_card_symbol(),
        skill_call_symbol(),
    ];
    #[cfg(any(feature = "cache", feature = "cassette"))]
    let symbols = {
        let mut symbols = symbols;
        symbols.push(skill_audit_symbol());
        symbols
    };
    #[cfg(feature = "agent")]
    let symbols = {
        let mut symbols = symbols;
        symbols.push(skill_as_tool_symbol());
        symbols
    };
    #[cfg(feature = "openai")]
    let symbols = {
        let mut symbols = symbols;
        symbols.extend([skill_openai_tool_symbol(), skill_openai_tools_symbol()]);
        symbols
    };
    #[cfg(feature = "runner")]
    let symbols = {
        let mut symbols = symbols;
        symbols.push(skill_as_runner_symbol());
        symbols
    };
    #[cfg(feature = "serve")]
    let symbols = {
        let mut symbols = symbols;
        symbols.push(skill_serve_mcp_symbol());
        symbols
    };
    symbols
}

fn field(name: &str) -> Expr {
    Expr::Symbol(Symbol::new(name.to_owned()))
}
