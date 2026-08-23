use sim_codec::{Input, Output, decode_with_codec, encode_with_codec};
use sim_codec_bridge::{
    BridgeBook, BridgeCallArgument, BridgeCallPayload, BridgeHeader, BridgePacket, BridgePart,
    BridgeProvenance, CallArgumentMedia, content_id_string, stamp_packet_cid,
};
use sim_kernel::{
    Cx, Datum, EncodeOptions, EvalFabric, Expr, ReadPolicy, Result, Symbol, encode::EncodePosition,
};
use sim_lib_agent_runner_core::{
    InjectionFence, ModelResponse, OutputContract, terminal_model_content,
};
use sim_shape::{check_value_report, shape_value};
use sim_value::{access::field, build::entry};

use crate::model::output_contract_for_packet;
use crate::parent::parent_token;
use crate::repair::{AskFailure, RepairPolicy};
use crate::rx::{effective_caps, rx_check, shape_from_contract_expr};
use crate::tx::{eval_request_for_checked_packet, prepare_packet};

/// Default codec for packing ASK call arguments and answers.
pub fn ask_default_codec() -> Symbol {
    Symbol::qualified("codec", "json")
}

/// Builds an ASK request packet with model parameters omitted.
pub fn ask_packet(
    cx: &mut Cx,
    name: &str,
    params: Vec<(String, Expr)>,
    return_shape: Expr,
    to: &str,
) -> Result<BridgePacket> {
    ask_packet_with_model_params(cx, name, params, Vec::new(), return_shape, to)
}

/// Builds an ASK request packet.
///
/// Argument values are encoded through the default ASK codec at data position,
/// wrapped in deterministic injection fences, and only then stored in the
/// packet.
pub fn ask_packet_with_model_params(
    cx: &mut Cx,
    name: &str,
    params: Vec<(String, Expr)>,
    model_params: Vec<(String, Expr)>,
    return_shape: Expr,
    to: &str,
) -> Result<BridgePacket> {
    let codec = ask_default_codec();
    let mut call = BridgeCallPayload::new(symbol_from_name(name));
    for (name, value) in params {
        call = call.with_arg(pack_argument(cx, &name, &codec, &value)?);
    }
    for (name, value) in model_params {
        call = call.with_model_param(symbol_from_name(&name), value);
    }
    Ok(BridgePacket {
        header: BridgeHeader {
            cid: None,
            move_kind: Symbol::new("request"),
            from: "sim".to_owned(),
            to: vec![to.to_owned()],
            role: Symbol::new("implementer"),
            parents: Vec::new(),
            task: Symbol::new("C1"),
            output: Symbol::new("O1"),
            ceiling: ask_capability_ceiling(),
            context: Vec::new(),
            provenance: BridgeProvenance::default(),
        },
        body: vec![
            BridgePart {
                id: Symbol::new("C1"),
                kind: Symbol::qualified("bridge", "Call"),
                payload: call.to_expr(),
            },
            BridgePart {
                id: Symbol::new("O1"),
                kind: Symbol::qualified("bridge", "Return"),
                payload: Expr::Map(vec![
                    entry("codec", Expr::Symbol(codec)),
                    entry("shape", return_shape),
                ]),
            },
        ],
        warrant: None,
    })
}

fn ask_capability_ceiling() -> Vec<Symbol> {
    [
        Symbol::qualified("ai", "run"),
        Symbol::qualified("capability", "ai-runner"),
        Symbol::qualified("capability", "ai-runner-local"),
        Symbol::qualified("capability", "ai-runner-network"),
        Symbol::qualified("capability", "ai-runner-secret"),
        Symbol::qualified("capability", "exec"),
        Symbol::qualified("capability", "host.process"),
    ]
    .into()
}

/// Runs an ASK packet with the default bounded repair policy.
pub fn run_ask(cx: &mut Cx, target: &dyn EvalFabric, packet: BridgePacket) -> Result<BridgePacket> {
    run_ask_with_policy(cx, target, packet, RepairPolicy::default())
}

/// Runs an ASK packet with an explicit bounded repair policy.
pub fn run_ask_with_policy(
    cx: &mut Cx,
    target: &dyn EvalFabric,
    mut packet: BridgePacket,
    policy: RepairPolicy,
) -> Result<BridgePacket> {
    let book = BridgeBook::standard();
    let max_retries = policy.retries();
    for attempt in 0..=max_retries {
        match run_ask_once(cx, target, packet)? {
            AskAttempt::Answer(answer) => return Ok(answer),
            AskAttempt::RepairNeeded { checked, failure } if attempt < max_retries => {
                packet = repair_packet_for_failure(cx, &checked, &failure, attempt + 1)?;
            }
            AskAttempt::RepairNeeded { failure, .. } => {
                return Err(sim_kernel::Error::Eval(format!(
                    "bridge ask failed after {} attempt(s): {}",
                    attempt + 1,
                    failure.message()
                )));
            }
        }
    }
    unreachable!("bounded ASK loop always returns inside the retry range")
}

/// Result of one checked ASK exchange, with repair kept as typed control flow.
#[derive(Clone, Debug, PartialEq)]
pub enum AskAttempt {
    /// The single exchange produced a checked BRIDGE reply packet.
    Answer(BridgePacket),
    /// The model replied, but decoding or the declared return Shape requires repair.
    RepairNeeded {
        /// Exact checked request packet that the repair must parent.
        checked: BridgePacket,
        /// Typed failure used to construct a bounded repair request.
        failure: AskFailure,
    },
}

/// Performs exactly one ASK exchange: TX checks, one model call, and RX checks.
///
/// This function never retries. Callers that own repetition can use the typed
/// [`AskAttempt::RepairNeeded`] result; [`run_ask_with_policy`] remains the
/// compatibility wrapper that owns the existing bounded loop.
pub fn run_ask_once(
    cx: &mut Cx,
    target: &dyn EvalFabric,
    packet: BridgePacket,
) -> Result<AskAttempt> {
    let book = BridgeBook::standard();
    let checked = prepare_packet(cx, &book, &packet)?;
    let request = eval_request_for_checked_packet(cx, &book, &checked)?;
    let caps = effective_caps(cx, &checked)?;
    let reply = cx.with_capabilities(caps, |cx| target.realize(cx, request))?;
    let response = ModelResponse::try_from(reply.value.object().as_expr(cx)?)?;
    match answer_packet(cx, &book, &checked, &response)? {
        Ok(answer) => Ok(AskAttempt::Answer(answer)),
        Err(failure) => Ok(AskAttempt::RepairNeeded { checked, failure }),
    }
}

pub(crate) fn pack_argument(
    cx: &mut Cx,
    name: &str,
    codec: &Symbol,
    value: &Expr,
) -> Result<BridgeCallArgument> {
    let output = encode_with_codec(
        cx,
        codec,
        value,
        EncodeOptions {
            position: EncodePosition::Data,
            ..EncodeOptions::default()
        },
    )?;
    let (media, datum, body) = match output {
        Output::Text(text) => (CallArgumentMedia::Text, Datum::String(text.clone()), text),
        Output::Bytes(bytes) => (
            CallArgumentMedia::Bytes,
            Datum::Bytes(bytes.clone()),
            hex_text(&bytes),
        ),
    };
    let content_id = datum.content_id()?;
    let fence = InjectionFence::for_content(&content_id);
    Ok(BridgeCallArgument::new(
        symbol_from_name(name),
        codec.clone(),
        media,
        content_id_string(&content_id),
        fence.wrap(name, &body),
    ))
}

fn answer_packet(
    cx: &mut Cx,
    book: &BridgeBook,
    parent: &BridgePacket,
    response: &ModelResponse,
) -> Result<std::result::Result<BridgePacket, AskFailure>> {
    let contract = output_contract_for_packet(parent)?;
    let answer = match decode_terminal_answer(cx, response, &contract)? {
        Ok(answer) => answer,
        Err(failure) => return Ok(Err(failure)),
    };
    if let Err(failure) = validate_answer(cx, &contract, &answer)? {
        return Ok(Err(failure));
    }
    let packet = stamp_packet_cid(&BridgePacket {
        header: BridgeHeader {
            cid: None,
            move_kind: Symbol::new("reply"),
            from: parent
                .header
                .to
                .first()
                .cloned()
                .unwrap_or_else(|| "model".to_owned()),
            to: vec![parent.header.from.clone()],
            role: Symbol::new("implementer"),
            parents: parent_token(parent).into_iter().collect(),
            task: Symbol::new("A1"),
            output: Symbol::new("A1"),
            ceiling: Vec::new(),
            context: Vec::new(),
            provenance: BridgeProvenance::default(),
        },
        body: vec![BridgePart {
            id: Symbol::new("A1"),
            kind: Symbol::qualified("bridge", "Return"),
            payload: answer,
        }],
        warrant: None,
    })?;
    let report = rx_check(cx, book, &packet, Some(parent))?;
    if !report.accepted() {
        return Err(sim_kernel::Error::Eval(format!(
            "bridge ask reply failed rx check: {:?}",
            report.obligations
        )));
    }
    Ok(Ok(packet))
}

fn decode_terminal_answer(
    cx: &mut Cx,
    response: &ModelResponse,
    contract: &OutputContract,
) -> Result<std::result::Result<Expr, AskFailure>> {
    let input = match terminal_model_content(response) {
        Ok(Expr::String(text)) => Input::Text(text.clone()),
        Ok(Expr::Bytes(bytes)) => Input::Bytes(bytes.clone()),
        Ok(Expr::Map(_)) => match field(terminal_model_content(response)?, "text") {
            Some(Expr::String(text)) => Input::Text(text.clone()),
            _ => {
                return Ok(Err(AskFailure::Decode {
                    codec: contract.codec.clone(),
                    message: "terminal content map must carry text".to_owned(),
                }));
            }
        },
        Ok(other) => {
            return Ok(Err(AskFailure::Decode {
                codec: contract.codec.clone(),
                message: format!("terminal content must be text or bytes, found {other:?}"),
            }));
        }
        Err(err) => {
            return Ok(Err(AskFailure::Decode {
                codec: contract.codec.clone(),
                message: err.to_string(),
            }));
        }
    };
    if let Input::Text(text) = &input
        && let Some(failure) = grammar_check_failure(cx, contract, text)?
    {
        return Ok(Err(failure));
    }
    match decode_with_codec(cx, &contract.codec, input, ReadPolicy::default()) {
        Ok(answer) => Ok(Ok(answer)),
        Err(err) => Ok(Err(AskFailure::Decode {
            codec: contract.codec.clone(),
            message: err.to_string(),
        })),
    }
}

fn grammar_check_failure(
    cx: &mut Cx,
    contract: &OutputContract,
    text: &str,
) -> Result<Option<AskFailure>> {
    if contract.grammar.is_none()
        && contract.grammar_dialect.is_none()
        && contract.grammar_graph.is_none()
    {
        return Ok(None);
    }
    let Some(shape) = shape_from_contract_expr(&contract.shape_expr) else {
        return Ok(Some(AskFailure::Shape {
            expected: format!("{:?}", contract.shape_expr),
            diagnostics: vec!["unsupported return Shape expression".to_owned()],
        }));
    };
    let decoded = match decode_with_codec(
        cx,
        &contract.codec,
        Input::Text(text.to_owned()),
        ReadPolicy::default(),
    ) {
        Ok(decoded) => decoded,
        Err(err) => {
            return Ok(Some(AskFailure::Decode {
                codec: contract.codec.clone(),
                message: err.to_string(),
            }));
        }
    };
    let matched = shape.check_expr(cx, &decoded)?;
    if matched.accepted {
        Ok(None)
    } else {
        Ok(Some(AskFailure::Shape {
            expected: format!("{:?}", contract.shape_expr),
            diagnostics: matched
                .diagnostics
                .iter()
                .map(|diagnostic| diagnostic.message.clone())
                .collect(),
        }))
    }
}

fn validate_answer(
    cx: &mut Cx,
    contract: &OutputContract,
    answer: &Expr,
) -> Result<std::result::Result<(), AskFailure>> {
    let Some(shape) = shape_from_contract_expr(&contract.shape_expr) else {
        return Ok(Err(AskFailure::Shape {
            expected: format!("{:?}", contract.shape_expr),
            diagnostics: vec!["unsupported return Shape expression".to_owned()],
        }));
    };
    let shape_ref = shape_value(Symbol::qualified("bridge", "AskReturn"), shape);
    let value = cx.factory().expr(answer.clone())?;
    let matched = check_value_report(cx, &shape_ref, value)?;
    if matched.accepted {
        Ok(Ok(()))
    } else {
        Ok(Err(AskFailure::Shape {
            expected: format!("{:?}", contract.shape_expr),
            diagnostics: matched
                .diagnostics
                .iter()
                .map(|diagnostic| diagnostic.message.clone())
                .collect(),
        }))
    }
}

fn repair_packet_for_failure(
    cx: &mut Cx,
    packet: &BridgePacket,
    failure: &AskFailure,
    attempt: u8,
) -> Result<BridgePacket> {
    let mut repaired = packet.canonicalized();
    for part in &mut repaired.body {
        if part.kind != Symbol::qualified("bridge", "Call") {
            continue;
        }
        let payload = BridgeCallPayload::from_expr(&part.payload)?.with_arg(pack_argument(
            cx,
            &format!("repair-{attempt}"),
            &ask_default_codec(),
            &failure.to_expr(),
        )?);
        part.payload = payload.to_expr();
        return Ok(repaired);
    }
    Err(sim_kernel::Error::Eval(
        "bridge ask repair requires a Call part".to_owned(),
    ))
}

fn symbol_from_name(name: &str) -> Symbol {
    match name.split_once('/') {
        Some((namespace, name)) if !namespace.is_empty() && !name.is_empty() => {
            Symbol::qualified(namespace, name)
        }
        _ => Symbol::new(name),
    }
}

fn hex_text(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>()
}
