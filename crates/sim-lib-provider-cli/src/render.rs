fn render_fan_out(inventory: &ProviderInventory, target: Option<&String>) -> Result<String> {
    let target = target.map(String::as_str).unwrap_or("all");
    let seats: Vec<_> = if target == "all" {
        inventory.seats.iter().collect()
    } else {
        vec![
            inventory
                .seats
                .iter()
                .find(|seat| seat.id == target)
                .ok_or_else(|| Error::Eval(format!("unknown provider seat {target}")))?,
        ]
    };
    Ok(seats.into_iter().map(|seat| format!(
        "operation=fan-out seat-id={} family=provider/{} status=planned mode=explicit provider-organ=sim-lib-provider\n",
        seat.id, seat.family
    )).collect())
}

fn read_inventory(path: &str) -> Result<ProviderInventory> {
    let source = fs::read_to_string(Path::new(path))
        .map_err(|err| Error::Eval(format!("read provider inventory {path}: {err}")))?;
    ProviderInventory::from_toml(&source)
}

fn split_inventory(args: &[String]) -> Result<(String, Vec<String>)> {
    let mut path = None;
    let mut words = Vec::new();
    let mut iter = args.iter().filter(|arg| arg.as_str() != "provider");
    while let Some(arg) = iter.next() {
        if arg == "--inventory" {
            path = iter.next().cloned();
        } else {
            words.push(arg.clone());
        }
    }
    Ok((
        path.ok_or_else(|| Error::Eval("provider requires --inventory <path>".into()))?,
        words,
    ))
}

fn render_families(inventory: &ProviderInventory) -> Result<String> {
    let mut families: Vec<_> = inventory
        .seats
        .iter()
        .map(|seat| seat.family.as_str())
        .collect();
    families.sort_unstable();
    families.dedup();
    Ok(families
        .into_iter()
        .map(|family| format!("family=provider/{family}\n"))
        .collect())
}

fn render_seats(inventory: &ProviderInventory) -> Result<String> {
    inventory.seats.iter().map(render_seat).collect()
}

fn render_show(inventory: &ProviderInventory, id: Option<&String>) -> Result<String> {
    let id = id.ok_or_else(|| Error::Eval("provider show requires a seat id".into()))?;
    let seat = inventory
        .seats
        .iter()
        .find(|seat| &seat.id == id)
        .ok_or_else(|| Error::Eval(format!("unknown provider seat {id}")))?;
    render_seat(seat)
}

fn render_status(inventory: &ProviderInventory) -> Result<String> {
    inventory
        .census(&[])?
        .into_iter()
        .map(|row| {
            Ok(format!(
                "seat-id={} family=provider/{} readiness={} reason={} next-action={}\n",
                row.seat_id,
                row.family,
                state_name(row.state),
                row.reason,
                row.next_action
            ))
        })
        .collect()
}

fn render_seat(seat: &ProviderSeatConfig) -> Result<String> {
    let transport = if seat.config_home_ref.is_some() {
        "broker-process"
    } else if matches!(seat.family.as_str(), "ollama" | "lemonade" | "lm-studio") {
        "local-http"
    } else {
        "https"
    };
    let semantics = if seat.config_home_ref.is_some() {
        "agent-task"
    } else {
        "model-turn"
    };
    let identity = seat
        .config_home_ref
        .as_deref()
        .unwrap_or(&seat.endpoint_label);
    let wire = match seat.family.as_str() {
        "anthropic-api" | "claude-cli" => "anthropic",
        "ollama" => "ollama",
        _ => "openai-compatible",
    };
    let terms = seat
        .terms_acknowledgement
        .as_deref()
        .unwrap_or("not-required");
    Ok(format!(
        "seat-id={} family=provider/{} transport={} semantics={} principal-kind={} identity={} wire={} readiness=cached-unknown terms={}\n",
        seat.id, seat.family, transport, semantics, seat.auth, identity, wire, terms
    ))
}

fn state_name(state: CensusState) -> &'static str {
    match state {
        CensusState::Ready => "ready",
        CensusState::Unavailable => "unavailable",
        CensusState::Expired => "expired",
        CensusState::Refused => "refused",
    }
}
