use super::*;

pub(super) fn recognized_legacy_probe(kind: &str, error: &BindingError) -> bool {
    match (kind, error) {
        ("stdio", BindingError::Rpc { code: -32601, .. }) => true,
        ("http", BindingError::Http { status: 400, body }) => exact_method_not_found(body),
        _ => false,
    }
}

fn exact_method_not_found(body: &[u8]) -> bool {
    serde_json::from_slice::<Value>(body)
        .ok()
        .is_some_and(|value| {
            value.pointer("/error/code").and_then(Value::as_i64) == Some(-32601)
                && value.pointer("/error/message").and_then(Value::as_str)
                    == Some("Method not found")
        })
}

pub(super) fn is_unrecognized_probe(error: &BindingError) -> bool {
    matches!(error, BindingError::Rpc { .. } | BindingError::Http { .. })
}

pub(super) fn legacy_initialize() -> Value {
    json!({"protocolVersion":"2025-03-26","clientInfo":{"name":"sim-lib-mcp-client","version":env!("CARGO_PKG_VERSION")},"capabilities":{}})
}

pub(super) fn parse_discovery(era: Era, value: &Value) -> Result<Discovery, ClientError> {
    let object = value
        .as_object()
        .ok_or(ClientError::UnrecognizedDiscovery)?;
    let (version, extensions, info, ttl_ms) = if era == Era::Modern {
        if object.get("resultType").and_then(Value::as_str) != Some("complete") {
            return Err(ClientError::UnrecognizedDiscovery);
        }
        let versions = object
            .get("supportedVersions")
            .and_then(Value::as_array)
            .ok_or(ClientError::UnrecognizedDiscovery)?;
        let version = versions
            .first()
            .and_then(Value::as_str)
            .ok_or(ClientError::UnrecognizedDiscovery)?
            .to_owned();
        let extensions = strings(object.get("extensions"))?;
        (
            version,
            extensions,
            object.get("serverInfo"),
            object.get("ttlMs").and_then(Value::as_u64).or_else(|| {
                object
                    .get("ttl")
                    .and_then(Value::as_u64)
                    .map(|v| v.saturating_mul(1000))
            }),
        )
    } else {
        (
            string(object, "protocolVersion")?,
            strings(object.get("extensions")).unwrap_or_default(),
            object.get("serverInfo"),
            Some(60_000),
        )
    };
    if version.is_empty() {
        return Err(ClientError::Schema("empty protocol version".into()));
    }
    let mut unique = BTreeSet::new();
    if extensions
        .iter()
        .any(|value| value.is_empty() || !unique.insert(value))
    {
        return Err(ClientError::Schema("invalid duplicate extension".into()));
    }
    let info = info
        .and_then(Value::as_object)
        .ok_or_else(|| ClientError::Schema("missing serverInfo".into()))?;
    Ok(Discovery {
        era,
        version,
        extensions,
        server_name: string(info, "name")?,
        server_version: string(info, "version")?,
        ttl: Duration::from_millis(ttl_ms.unwrap_or(60_000)),
    })
}

pub(super) fn cache_key(
    endpoint: EndpointIdentity,
    discovery: &Discovery,
    invocation: &Invocation,
    parameters: &Value,
    context: &CallContext<'_>,
) -> Result<CacheKey, ClientError> {
    Ok(CacheKey {
        endpoint,
        era: discovery.era,
        version: discovery.version.clone(),
        principal_scope: context.principal_scope.to_owned(),
        extensions: discovery.extensions.clone(),
        method: invocation.method.clone(),
        canonical_parameters: serde_json::to_string(&canonical(parameters))
            .map_err(|e| ClientError::Cache(e.to_string()))?,
        pagination_cursor: context.pagination_cursor.map(str::to_owned),
    })
}

fn canonical(value: &Value) -> Value {
    match value {
        Value::Object(values) => Value::Object(
            values
                .iter()
                .map(|(k, v)| (k.clone(), canonical(v)))
                .collect::<BTreeMap<_, _>>()
                .into_iter()
                .collect(),
        ),
        Value::Array(values) => Value::Array(values.iter().map(canonical).collect()),
        other => other.clone(),
    }
}
