use crate::{ModelBid, ModelCard, ModelEvent, ModelRequest, ModelResponse, ModelUsage};
use sim_codec_chat::{
    is_model_request_expr, model_card_expr, model_request_messages_expr, model_response_expr,
    validate_chat_transcript,
};
use sim_kernel::{Error, Expr, Result, Symbol};

impl From<ModelRequest> for Expr {
    fn from(value: ModelRequest) -> Self {
        let mut entries = vec![
            key_bool("model-request", true),
            key_expr("task", value.task),
            key_expr("messages", Expr::List(value.messages)),
        ];
        entries.extend(value.extra);
        Expr::Map(entries)
    }
}

impl TryFrom<Expr> for ModelRequest {
    type Error = Error;

    fn try_from(value: Expr) -> Result<Self> {
        if !is_model_request_expr(&value) {
            return Err(Error::Eval(
                "model request transcript must set model-request true".to_owned(),
            ));
        }
        validate_chat_transcript(&value)?;
        Ok(Self {
            task: require_field(&value, "task")?.clone(),
            messages: model_request_messages_expr(&value)?.to_vec(),
            extra: extra_fields(&value, &["model-request", "task", "messages"]),
        })
    }
}

impl From<ModelResponse> for Expr {
    fn from(value: ModelResponse) -> Self {
        let mut expr = match model_response_expr(
            value.runner,
            value.model,
            value.content,
            value.stop_reason,
        ) {
            Expr::Map(entries) => entries,
            _ => unreachable!("model_response_expr always returns a map"),
        };
        if let Some(usage) = value.usage {
            expr.push(key_expr("usage", usage.into()));
        }
        expr.extend(value.extra);
        Expr::Map(expr)
    }
}

impl TryFrom<Expr> for ModelResponse {
    type Error = Error;

    fn try_from(value: Expr) -> Result<Self> {
        validate_chat_transcript(&value)?;
        expect_marker(&value, "model-response")?;
        Ok(Self {
            runner: require_symbol_field(&value, "runner")?.clone(),
            model: require_string_field(&value, "model")?.to_owned(),
            content: require_list_field(&value, "content")?.to_vec(),
            stop_reason: require_symbol_field(&value, "stop-reason")?.clone(),
            usage: field(&value, "usage")
                .map(ModelUsage::from_expr)
                .transpose()?,
            extra: extra_fields(
                &value,
                &[
                    "model-response",
                    "runner",
                    "model",
                    "content",
                    "stop-reason",
                    "usage",
                ],
            ),
        })
    }
}

impl From<ModelUsage> for Expr {
    fn from(value: ModelUsage) -> Self {
        let mut entries = Vec::new();
        if let Some(tokens) = value.input_tokens {
            entries.push(key_expr("input-tokens", number_expr(tokens)));
        }
        if let Some(tokens) = value.output_tokens {
            entries.push(key_expr("output-tokens", number_expr(tokens)));
        }
        if let Some(latency) = value.latency_ms {
            entries.push(key_expr("latency-ms", number_expr(latency)));
        }
        if let Some(cost) = value.cost_usd {
            entries.push(key_expr("cost-usd", float_expr(cost)));
        }
        entries.extend(value.extra);
        Expr::Map(entries)
    }
}

impl ModelUsage {
    fn from_expr(value: &Expr) -> Result<Self> {
        let Expr::Map(_) = value else {
            return Err(Error::Eval("model usage must be a map".to_owned()));
        };
        Ok(Self {
            input_tokens: field(value, "input-tokens").map(parse_u64).transpose()?,
            output_tokens: field(value, "output-tokens").map(parse_u64).transpose()?,
            latency_ms: field(value, "latency-ms").map(parse_u64).transpose()?,
            cost_usd: field(value, "cost-usd").map(parse_f64).transpose()?,
            extra: extra_fields(
                value,
                &["input-tokens", "output-tokens", "latency-ms", "cost-usd"],
            ),
        })
    }
}

impl From<ModelEvent> for Expr {
    fn from(value: ModelEvent) -> Self {
        let mut entries = vec![
            key_bool("model-event", true),
            key_expr("event", Expr::Symbol(value.event)),
            key_expr("runner", Expr::Symbol(value.runner)),
            key_expr("model", Expr::String(value.model)),
            key_expr("span-id", value.span_id),
        ];
        if let Some(response) = value.response {
            entries.push(key_expr("response", response.into()));
        }
        entries.extend(value.extra);
        Expr::Map(entries)
    }
}

impl TryFrom<Expr> for ModelEvent {
    type Error = Error;

    fn try_from(value: Expr) -> Result<Self> {
        validate_chat_transcript(&value)?;
        expect_marker(&value, "model-event")?;
        Ok(Self {
            event: require_symbol_field(&value, "event")?.clone(),
            runner: require_symbol_field(&value, "runner")?.clone(),
            model: require_string_field(&value, "model")?.to_owned(),
            span_id: require_field(&value, "span-id")?.clone(),
            response: field(&value, "response")
                .cloned()
                .map(ModelResponse::try_from)
                .transpose()?,
            extra: extra_fields(
                &value,
                &[
                    "model-event",
                    "event",
                    "runner",
                    "model",
                    "span-id",
                    "response",
                ],
            ),
        })
    }
}

impl From<ModelCard> for Expr {
    fn from(value: ModelCard) -> Self {
        let mut expr =
            match model_card_expr(value.runner, value.model, value.provider, value.locality) {
                Expr::Map(entries) => entries,
                _ => unreachable!("model_card_expr always returns a map"),
            };
        expr.extend(value.extra);
        Expr::Map(expr)
    }
}

impl TryFrom<Expr> for ModelCard {
    type Error = Error;

    fn try_from(value: Expr) -> Result<Self> {
        validate_chat_transcript(&value)?;
        expect_marker(&value, "model-card")?;
        Ok(Self {
            runner: require_symbol_field(&value, "runner")?.clone(),
            model: require_string_field(&value, "model")?.to_owned(),
            provider: require_symbol_field(&value, "provider")?.clone(),
            locality: require_symbol_field(&value, "locality")?.clone(),
            extra: extra_fields(
                &value,
                &["model-card", "runner", "model", "provider", "locality"],
            ),
        })
    }
}

impl From<ModelBid> for Expr {
    fn from(value: ModelBid) -> Self {
        let mut entries = vec![key_bool("model-bid", value.available)];
        if let Some(reason) = value.reason {
            entries.push(key_expr("reason", Expr::String(reason)));
        }
        if let Some(score) = value.score {
            entries.push(key_expr("score", float_expr(score)));
        }
        if let Some(model) = value.model {
            entries.push(key_expr("model", Expr::String(model)));
        }
        entries.extend(value.extra);
        Expr::Map(entries)
    }
}

impl TryFrom<Expr> for ModelBid {
    type Error = Error;

    fn try_from(value: Expr) -> Result<Self> {
        Ok(Self {
            available: match field(&value, "model-bid") {
                Some(Expr::Bool(flag)) => *flag,
                _ => {
                    return Err(Error::Eval(
                        "chat transcript must set model-bid to a bool".to_owned(),
                    ));
                }
            },
            reason: field(&value, "reason")
                .map(require_string)
                .transpose()?
                .map(str::to_owned),
            score: field(&value, "score").map(parse_f64).transpose()?,
            model: field(&value, "model")
                .map(require_string)
                .transpose()?
                .map(str::to_owned),
            extra: extra_fields(&value, &["model-bid", "reason", "score", "model"]),
        })
    }
}

use sim_value::access::field;

fn require_field<'a>(expr: &'a Expr, name: &str) -> Result<&'a Expr> {
    field(expr, name).ok_or_else(|| Error::Eval(format!("chat transcript missing {name} field")))
}

fn require_list_field<'a>(expr: &'a Expr, name: &str) -> Result<&'a [Expr]> {
    match require_field(expr, name)? {
        Expr::List(items) => Ok(items),
        _ => Err(Error::Eval(format!(
            "chat transcript {name} field must be a list"
        ))),
    }
}

fn require_symbol_field<'a>(expr: &'a Expr, name: &str) -> Result<&'a Symbol> {
    match require_field(expr, name)? {
        Expr::Symbol(symbol) => Ok(symbol),
        _ => Err(Error::Eval(format!(
            "chat transcript {name} field must be a symbol"
        ))),
    }
}

fn require_string_field<'a>(expr: &'a Expr, name: &str) -> Result<&'a str> {
    require_string(require_field(expr, name)?)
}

fn require_string(expr: &Expr) -> Result<&str> {
    match expr {
        Expr::String(text) => Ok(text),
        _ => Err(Error::Eval(
            "chat transcript field must be a string".to_owned(),
        )),
    }
}

fn expect_marker(expr: &Expr, name: &str) -> Result<()> {
    if matches!(field(expr, name), Some(Expr::Bool(true))) {
        Ok(())
    } else {
        Err(Error::Eval(format!("chat transcript must set {name} true")))
    }
}

fn extra_fields(expr: &Expr, known: &[&str]) -> Vec<(Expr, Expr)> {
    let Expr::Map(entries) = expr else {
        return Vec::new();
    };
    entries
        .iter()
        .filter(|(key, _)| !is_known_field(key, known))
        .cloned()
        .collect()
}

fn is_known_field(key: &Expr, known: &[&str]) -> bool {
    match key {
        Expr::Symbol(symbol) if symbol.namespace.is_none() => {
            known.iter().any(|name| symbol.name.as_ref() == *name)
        }
        _ => false,
    }
}

fn key_bool(name: &str, value: bool) -> (Expr, Expr) {
    key_expr(name, Expr::Bool(value))
}

use sim_value::build::entry as key_expr;

fn number_expr(value: u64) -> Expr {
    sim_value::build::num_q(Some("numbers"), "f64", &value.to_string())
}

fn float_expr(value: f64) -> Expr {
    sim_value::build::num_q(Some("numbers"), "f64", &value.to_string())
}

fn parse_u64(expr: &Expr) -> Result<u64> {
    match expr {
        Expr::Number(number) => number
            .canonical
            .parse()
            .map_err(|_| Error::Eval("numeric transcript field must be an integer".to_owned())),
        _ => Err(Error::Eval(
            "numeric transcript field must be a number literal".to_owned(),
        )),
    }
}

fn parse_f64(expr: &Expr) -> Result<f64> {
    match expr {
        Expr::Number(number) => number
            .canonical
            .parse()
            .map_err(|_| Error::Eval("numeric transcript field must be numeric".to_owned())),
        _ => Err(Error::Eval(
            "numeric transcript field must be a number literal".to_owned(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use crate::{ModelBid, ModelCard, ModelEvent, ModelRequest, ModelResponse, ModelUsage};
    use sim_codec_chat::validate_chat_transcript;
    use sim_kernel::{Expr, Symbol};

    #[test]
    fn model_request_round_trips() {
        let mut request = ModelRequest::new(
            Expr::String("summarize".to_owned()),
            vec![Expr::Map(vec![
                (
                    Expr::Symbol(Symbol::new("role")),
                    Expr::Symbol(Symbol::new("user")),
                ),
                (
                    Expr::Symbol(Symbol::new("content")),
                    Expr::List(vec![Expr::Map(vec![
                        (
                            Expr::Symbol(Symbol::new("type")),
                            Expr::Symbol(Symbol::new("text")),
                        ),
                        (
                            Expr::Symbol(Symbol::new("text")),
                            Expr::String("hello".to_owned()),
                        ),
                    ])]),
                ),
            ])],
        );
        request.extra.push((
            Expr::Symbol(Symbol::new("metadata")),
            Expr::Map(vec![(
                Expr::Symbol(Symbol::new("ticket")),
                Expr::String("A5".to_owned()),
            )]),
        ));

        let expr: Expr = request.clone().into();
        validate_chat_transcript(&expr).unwrap();
        let decoded = ModelRequest::try_from(expr).unwrap();
        assert_eq!(decoded, request);
    }

    #[test]
    fn model_response_usage_round_trips() {
        let mut response = ModelResponse::new(
            Symbol::new("runner"),
            "fake/model",
            vec![Expr::Map(vec![
                (
                    Expr::Symbol(Symbol::new("type")),
                    Expr::Symbol(Symbol::new("text")),
                ),
                (
                    Expr::Symbol(Symbol::new("text")),
                    Expr::String("done".to_owned()),
                ),
            ])],
            Symbol::new("stop"),
        );
        response.usage = Some(ModelUsage {
            input_tokens: Some(10),
            output_tokens: Some(2),
            latency_ms: Some(5),
            cost_usd: Some(0.0),
            extra: Vec::new(),
        });
        response
            .extra
            .push((Expr::Symbol(Symbol::new("shape-ok")), Expr::Bool(true)));

        let expr: Expr = response.clone().into();
        validate_chat_transcript(&expr).unwrap();
        let decoded = ModelResponse::try_from(expr).unwrap();
        assert_eq!(decoded, response);
    }

    #[test]
    fn model_event_card_and_bid_round_trip() {
        let mut event = ModelEvent::new(
            Symbol::new("usage"),
            Symbol::new("runner"),
            "fake/model",
            Expr::String("span-1".to_owned()),
        );
        event.extra.push((
            Expr::Symbol(Symbol::new("detail")),
            Expr::String("tracked".to_owned()),
        ));
        let expr: Expr = event.clone().into();
        validate_chat_transcript(&expr).unwrap();
        assert_eq!(ModelEvent::try_from(expr).unwrap(), event);

        let mut card = ModelCard::new(
            Symbol::new("runner"),
            "fake/model",
            Symbol::new("fake"),
            Symbol::new("local"),
        );
        card.extra.push((
            Expr::Symbol(Symbol::new("supports-shape")),
            Expr::Bool(true),
        ));
        let expr: Expr = card.into();
        validate_chat_transcript(&expr).unwrap();
        let decoded = ModelCard::try_from(expr).unwrap();
        assert_eq!(decoded.runner, Symbol::new("runner"));
        assert_eq!(decoded.model, "fake/model");
        assert!(decoded.extra.iter().any(|(key, value)| {
            *key == Expr::Symbol(Symbol::new("supports-shape")) && *value == Expr::Bool(true)
        }));

        let bid = ModelBid::unavailable("busy");
        let expr: Expr = bid.clone().into();
        assert_eq!(ModelBid::try_from(expr).unwrap().reason, bid.reason);
    }
}
