/// Validates every input and mints the deck only after exact grounding succeeds.
pub fn build(input: DeckInput<'_>) -> Result<SourceDeck, Failure> {
    check_count(
        "repositories",
        input.repositories.len(),
        input.limits.repositories,
    )?;
    check_count("fragments", input.fragments.len(), input.limits.fragments)?;
    check_count("files", input.files.len(), input.limits.files)?;
    check_count("excerpts", input.excerpts.len(), input.limits.excerpts)?;
    check_count("specimens", input.specimens.len(), input.limits.specimens)?;
    check_count("queries", input.queries.len(), input.limits.queries)?;
    let total_bytes = input
        .fragments
        .iter()
        .map(|v| v.bytes.len())
        .chain(input.files.iter().map(|v| v.bytes.len()))
        .chain(input.specimens.iter().map(|v| v.bytes.len()))
        .sum();
    check_count("total-bytes", total_bytes, input.limits.total_bytes)?;

    unique(input.repositories.iter().map(|v| (&v.owner, "repository")))?;
    let repo_owners: BTreeSet<_> = input
        .repositories
        .iter()
        .map(|v| v.owner.as_str())
        .collect();
    for repo in &input.repositories {
        required(&repo.owner)?;
        required(&repo.repository)?;
        required(&repo.revision)?;
    }

    let mut decoded = Vec::new();
    for pin in &input.fragments {
        require_owner(&repo_owners, &pin.owner, "fragment")?;
        verify_content("fragment", &pin.bytes, &pin.content_id)?;
        let fragment = input.decoder.decode(&pin.bytes)?;
        if fragment.owner != pin.owner {
            return Err(Failure::OwnerMismatch {
                subject: pin.owner.clone(),
            });
        }
        decoded.push((pin, fragment));
    }
    let mut anchors: BTreeMap<&str, Vec<(&FragmentPin, &IndexAnchor)>> = BTreeMap::new();
    let mut index_specimens: BTreeMap<&str, Vec<&IndexSpecimen>> = BTreeMap::new();
    for (pin, fragment) in &decoded {
        for anchor in &fragment.anchors {
            if anchor.owner != fragment.owner {
                return Err(Failure::OwnerMismatch {
                    subject: anchor.id.clone(),
                });
            }
            if let Some(path) = &anchor.source_path {
                valid_path(path)?;
            }
            anchors.entry(&anchor.id).or_default().push((pin, anchor));
        }
        for specimen in &fragment.specimens {
            if specimen.owner != fragment.owner {
                return Err(Failure::OwnerMismatch {
                    subject: specimen.id.clone(),
                });
            }
            index_specimens
                .entry(&specimen.id)
                .or_default()
                .push(specimen);
        }
    }
    unique(
        input
            .certificates
            .iter()
            .map(|v| (&v.anchor, "certificate")),
    )?;
    for (id, rows) in &anchors {
        if rows.len() > 1 {
            return Err(Failure::MultiplyClaimed((*id).into()));
        }
        let certs: Vec<_> = input
            .certificates
            .iter()
            .filter(|v| v.anchor == *id)
            .collect();
        if certs.is_empty() {
            return Err(Failure::Unclaimed((*id).into()));
        }
        if certs.len() != 1 {
            return Err(Failure::MultiplyClaimed((*id).into()));
        }
        let (pin, anchor) = rows[0];
        let cert = certs[0];
        if cert.owner != anchor.owner || cert.fragment_id != pin.content_id {
            return Err(Failure::Substituted((*id).into()));
        }
        if cert.digest != cert.expected_digest()? {
            return Err(Failure::CertificateDigestMismatch((*id).into()));
        }
    }
    for cert in &input.certificates {
        if !anchors.contains_key(cert.anchor.as_str()) {
            return Err(Failure::DanglingAnchor(cert.anchor.clone()));
        }
    }

    let mut files = BTreeMap::new();
    for file in &input.files {
        require_owner(&repo_owners, &file.owner, &file.path)?;
        valid_path(&file.path)?;
        verify_content(&file.path, &file.bytes, &file.content_id)?;
        if files
            .insert((file.owner.as_str(), file.path.as_str()), file)
            .is_some()
        {
            return Err(Failure::Duplicate {
                kind: "file",
                id: file.path.clone(),
            });
        }
    }
    for rows in anchors.values() {
        for (_, anchor) in rows {
            if let Some(path) = &anchor.source_path
                && !files.contains_key(&(anchor.owner.as_str(), path.as_str()))
            {
                return Err(Failure::Unresolved {
                    kind: "anchor source",
                    id: format!("{}:{path}", anchor.owner),
                });
            }
        }
    }
    let mut excerpts = BTreeMap::new();
    for excerpt in &input.excerpts {
        valid_path(&excerpt.path)?;
        let file = files
            .get(&(excerpt.owner.as_str(), excerpt.path.as_str()))
            .ok_or_else(|| Failure::Unresolved {
                kind: "file",
                id: excerpt.path.clone(),
            })?;
        if excerpt.end < excerpt.start || excerpt.end > file.bytes.len() {
            return Err(Failure::Truncated {
                subject: excerpt.id.clone(),
            });
        }
        if file.bytes[excerpt.start..excerpt.end] != excerpt.bytes {
            return Err(Failure::ExcerptForgery(excerpt.id.clone()));
        }
        if excerpts.insert(excerpt.id.as_str(), excerpt).is_some() {
            return Err(Failure::Duplicate {
                kind: "excerpt",
                id: excerpt.id.clone(),
            });
        }
    }
    let mut specimens = BTreeMap::new();
    for specimen in &input.specimens {
        require_owner(&repo_owners, &specimen.owner, &specimen.id)?;
        verify_content(&specimen.id, &specimen.bytes, &specimen.content_id)?;
        match index_specimens.get(specimen.id.as_str()).map(Vec::as_slice) {
            None => {
                return Err(Failure::Unresolved {
                    kind: "specimen",
                    id: specimen.id.clone(),
                });
            }
            Some([row]) if row.owner == specimen.owner => {}
            Some([_]) => {
                return Err(Failure::OwnerMismatch {
                    subject: specimen.id.clone(),
                });
            }
            Some(_) => {
                return Err(Failure::Ambiguous {
                    kind: "specimen",
                    id: specimen.id.clone(),
                });
            }
        }
        if specimens.insert(specimen.id.as_str(), specimen).is_some() {
            return Err(Failure::Duplicate {
                kind: "specimen",
                id: specimen.id.clone(),
            });
        }
    }
    let mut evidence = Vec::new();
    for query in &input.queries {
        match query {
            SourceQuery::Anchor(id) => match anchors.get(id.as_str()).map(Vec::as_slice) {
                None => {
                    return Err(Failure::Unresolved {
                        kind: "anchor",
                        id: id.clone(),
                    });
                }
                Some([(_, row)]) => evidence.push(GroundedEvidence::Anchor((*row).clone())),
                Some(_) => {
                    return Err(Failure::Ambiguous {
                        kind: "anchor",
                        id: id.clone(),
                    });
                }
            },
            SourceQuery::Excerpt(id) => evidence.push(GroundedEvidence::Excerpt(
                (*excerpts
                    .get(id.as_str())
                    .ok_or_else(|| Failure::Unresolved {
                        kind: "excerpt",
                        id: id.clone(),
                    })?)
                .clone(),
            )),
            SourceQuery::Specimen(id) => evidence.push(GroundedEvidence::Specimen(
                (*specimens
                    .get(id.as_str())
                    .ok_or_else(|| Failure::Unresolved {
                        kind: "specimen",
                        id: id.clone(),
                    })?)
                .clone(),
            )),
        }
    }
    if input.limitations.iter().any(|v| matches!(v, Limitation::SyntaxBound { detail, .. } | Limitation::ScannerEvidence { detail, .. } if detail.is_empty())) { return Err(Failure::MissingLimitation("empty limitation evidence".into())); }

    let datum = canonical(
        &input.repositories,
        &input.fragments,
        &input.files,
        &evidence,
        &input.limitations,
    );
    let id = SourceDeckId(
        datum
            .content_id()
            .map_err(|e| Failure::Canonical(e.to_string()))?,
    );
    Ok(SourceDeck {
        id,
        repositories: input.repositories,
        fragments: input.fragments,
        files: input.files,
        evidence,
        limitations: input.limitations,
    })
}
