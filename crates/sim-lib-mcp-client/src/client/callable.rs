/// The single imported runtime callable path.
#[derive(Clone)]
pub struct McpCallable {
    card: SkillCard,
    invocation: Invocation,
    icons: Vec<IconDescriptor>,
}
impl McpCallable {
    /// Canonical runtime Card/SkillCard projection.
    pub fn card(&self) -> &SkillCard {
        &self.card
    }
    /// Validated invocation metadata.
    pub fn invocation(&self) -> &Invocation {
        &self.invocation
    }
    /// Validated inert icon descriptors. The client never dereferences them.
    pub fn icons(&self) -> &[IconDescriptor] {
        &self.icons
    }
}

/// Bounded inert icon metadata imported from a Card.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IconDescriptor {
    /// Absolute or relative descriptor URI retained verbatim without fetching.
    pub src: String,
    /// Declared media type, when present.
    pub media_type: Option<String>,
}

fn unwrap_complete(value: Value) -> Result<(Value, Option<u64>), ClientError> {
    if value
        .get("resultType")
        .and_then(Value::as_str)
        .is_some_and(|kind| kind != "complete")
    {
        return Err(ClientError::Policy(
            "only complete outcomes may publish".into(),
        ));
    }
    let ttl = value.get("ttlMs").and_then(Value::as_u64);
    Ok((value.get("value").cloned().unwrap_or(value), ttl))
}
fn complete(reply: PeerReply) -> Result<Value, ClientError> {
    match reply {
        PeerReply::Complete(value) => Ok(value),
        _ => Err(ClientError::Schema("expected complete response".into())),
    }
}
fn strings(value: Option<&Value>) -> Result<Vec<String>, ClientError> {
    value.map_or(Ok(Vec::new()), |value| {
        value
            .as_array()
            .ok_or_else(|| ClientError::Schema("expected string array".into()))?
            .iter()
            .map(|value| {
                value
                    .as_str()
                    .map(str::to_owned)
                    .ok_or_else(|| ClientError::Schema("expected string array".into()))
            })
            .collect()
    })
}
fn string(object: &Map<String, Value>, name: &str) -> Result<String, ClientError> {
    object
        .get(name)
        .and_then(Value::as_str)
        .filter(|v| !v.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| ClientError::Schema(format!("missing string {name}")))
}
fn validate_keys(object: &Map<String, Value>, allowed: &[&str]) -> Result<(), ClientError> {
    if let Some(key) = object.keys().find(|key| !allowed.contains(&key.as_str())) {
        Err(ClientError::Schema(format!("unknown Card field {key}")))
    } else {
        Ok(())
    }
}
fn validate_icons(
    value: Option<&Value>,
    policy: &ClientPolicy,
) -> Result<Vec<IconDescriptor>, ClientError> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let icons = value
        .as_array()
        .ok_or_else(|| ClientError::Schema("icons must be an array".into()))?;
    if icons.len() > policy.maximum_icons {
        return Err(ClientError::Schema("icon descriptor count exceeded".into()));
    }
    let bytes: usize = icons
        .iter()
        .map(|icon| {
            icon.get("src").and_then(Value::as_str).map_or(0, str::len)
                + icon
                    .get("mediaType")
                    .and_then(Value::as_str)
                    .map_or(0, str::len)
        })
        .sum();
    if bytes > policy.maximum_icon_metadata_bytes
        || icons
            .iter()
            .any(|icon| icon.get("src").and_then(Value::as_str).is_none())
    {
        return Err(ClientError::Schema(
            "invalid or oversized icon metadata".into(),
        ));
    }
    icons
        .iter()
        .map(|icon| {
            let object = icon
                .as_object()
                .ok_or_else(|| ClientError::Schema("icon descriptor must be an object".into()))?;
            validate_keys(object, &["src", "mediaType"])?;
            Ok(IconDescriptor {
                src: string(object, "src")?,
                media_type: object
                    .get("mediaType")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
            })
        })
        .collect()
}
fn stable_name(value: &str) -> Result<String, ClientError> {
    if value.is_empty()
        || !value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    {
        Err(ClientError::Schema("unsafe callable name".into()))
    } else {
        Ok(value.to_owned())
    }
}
fn duration_ms(value: Duration) -> u64 {
    u64::try_from(value.as_millis()).unwrap_or(u64::MAX)
}

fn validate_subscription(
    frames: Vec<Value>,
    cancellation: &Cancellation,
) -> Result<Subscription, ClientError> {
    let first = frames
        .first()
        .and_then(Value::as_object)
        .ok_or_else(|| ClientError::Subscription("missing acknowledgement".into()))?;
    if first.get("type").and_then(Value::as_str) != Some("acknowledged") {
        return Err(ClientError::Subscription(
            "first frame must acknowledge".into(),
        ));
    }
    let id = first
        .get("subscriptionId")
        .and_then(Value::as_str)
        .ok_or_else(|| ClientError::Subscription("acknowledgement missing id".into()))?
        .to_owned();
    let mut events = vec![ClientEvent::Acknowledged(id.clone())];
    for (index, frame) in frames.iter().enumerate().skip(1) {
        let object = frame
            .as_object()
            .ok_or_else(|| ClientError::Subscription("stream frame must be an object".into()))?;
        if object.get("subscriptionId").and_then(Value::as_str) != Some(&id) {
            return Err(ClientError::Subscription("subscription id mismatch".into()));
        }
        match object.get("type").and_then(Value::as_str) {
            Some("event") if index + 1 < frames.len() => events.push(ClientEvent::Event(
                object
                    .get("event")
                    .cloned()
                    .ok_or_else(|| ClientError::Subscription("event body missing".into()))?,
            )),
            Some("complete") if index + 1 == frames.len() => events.push(ClientEvent::Complete {
                completed_at_ms: object
                    .get("completedAtMs")
                    .and_then(Value::as_u64)
                    .ok_or_else(|| ClientError::Subscription("dated terminal required".into()))?,
                cancelled: object
                    .get("cancelled")
                    .and_then(Value::as_bool)
                    .unwrap_or_else(|| cancellation.is_cancelled()),
            }),
            _ => {
                return Err(ClientError::Subscription(
                    "invalid acknowledgement/event/terminal sequence".into(),
                ));
            }
        }
    }
    if !matches!(events.last(), Some(ClientEvent::Complete { .. })) {
        return Err(ClientError::Subscription("dated terminal required".into()));
    }
    Ok(Subscription { id, events })
}
