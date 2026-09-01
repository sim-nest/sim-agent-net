fn canonical(
    repos: &[RepositorySnapshot],
    fragments: &[FragmentPin],
    files: &[SourceFile],
    evidence: &[GroundedEvidence],
    limitations: &[Limitation],
) -> Datum {
    let mut rows = Vec::new();
    for v in repos {
        rows.push((
            format!("repo:{}", v.owner),
            format!("{}:{}", v.repository, v.revision),
        ));
    }
    for v in fragments {
        rows.push((format!("fragment:{}", v.owner), cid_text(&v.content_id.0)));
    }
    for v in files {
        rows.push((
            format!("file:{}:{}", v.owner, v.path),
            cid_text(&v.content_id.0),
        ));
    }
    for v in evidence {
        rows.push((format!("evidence:{v:?}"), format!("{v:?}")));
    }
    for v in limitations {
        rows.push((format!("limitation:{v:?}"), format!("{v:?}")));
    }
    rows.sort();
    Datum::Node {
        tag: tag("source-deck", "deck-v1"),
        fields: vec![(
            Symbol::new("entries"),
            Datum::Map(
                rows.into_iter()
                    .map(|(k, v)| (Datum::String(k), Datum::String(v)))
                    .collect(),
            ),
        )],
    }
}

fn required(value: &str) -> Result<(), Failure> {
    if value.is_empty() {
        Err(Failure::Stale {
            subject: "empty identity field".into(),
        })
    } else {
        Ok(())
    }
}
fn valid_path(path: &str) -> Result<(), Failure> {
    if path.is_empty()
        || path.starts_with('/')
        || path
            .split('/')
            .any(|p| p.is_empty() || p == "." || p == "..")
        || path.contains('\\')
    {
        Err(Failure::InvalidPath(path.into()))
    } else {
        Ok(())
    }
}
fn check_count(name: &'static str, actual: usize, maximum: usize) -> Result<(), Failure> {
    if actual > maximum {
        Err(Failure::OverLimit {
            limit: name,
            actual,
            maximum,
        })
    } else {
        Ok(())
    }
}
fn require_owner(owners: &BTreeSet<&str>, owner: &str, subject: &str) -> Result<(), Failure> {
    if owners.contains(owner) {
        Ok(())
    } else {
        Err(Failure::OwnerMismatch {
            subject: subject.into(),
        })
    }
}
fn verify_content(subject: &str, bytes: &[u8], expected: &ByteContentId) -> Result<(), Failure> {
    if &ByteContentId::of(bytes)? == expected {
        Ok(())
    } else {
        Err(Failure::ContentMismatch(subject.into()))
    }
}
fn unique<'a>(items: impl Iterator<Item = (&'a String, &'static str)>) -> Result<(), Failure> {
    let mut seen = BTreeSet::new();
    for (id, kind) in items {
        if !seen.insert(id) {
            return Err(Failure::Duplicate {
                kind,
                id: id.clone(),
            });
        }
    }
    Ok(())
}
fn tag(namespace: &str, name: &str) -> Symbol {
    Symbol::qualified(namespace, name)
}
fn field(name: &str, value: &str) -> (Symbol, Datum) {
    (Symbol::new(name), Datum::String(value.into()))
}
fn cid_field(name: &str, id: &ContentId) -> (Symbol, Datum) {
    (Symbol::new(name), Datum::String(cid_text(id)))
}
fn cid_text(id: &ContentId) -> String {
    format!(
        "{}:{}",
        id.algorithm,
        id.bytes
            .iter()
            .map(|v| format!("{v:02x}"))
            .collect::<String>()
    )
}
