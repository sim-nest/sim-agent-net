impl RoadmapSpec {
    pub fn admit(&self) -> Result<AdmittedRoadmap, Failure> {
        AdmittedRoadmap::compile(self)
    }
}

fn validate_spec(spec: &RoadmapSpec) -> Result<(), Failure> {
    let l = spec.limits;
    bounded("imports", spec.imports.len(), l.imports)?;
    validate_prose("charter title", &spec.charter.title, l.prose_bytes)?;
    validate_prose("charter intent", &spec.charter.intent, l.document_bytes)?;
    if !spec.phases.contains_key(&spec.root) {
        return Err(Failure::Missing {
            kind: "root phase",
            id: spec.root.to_string(),
        });
    }
    AdmittedRoadmap::compile(spec)?;
    for (key, phase) in &spec.phases {
        if key != &phase.id {
            return Err(Failure::Duplicate {
                kind: "phase id",
                id: phase.id.to_string(),
            });
        }
        validate_phase(phase, spec)?;
    }
    for (id, pin) in &spec.imports {
        if pin.revision.0.bytes == [0; 32] || pin.root_content.bytes == [0; 32] {
            return Err(Failure::UnpinnedImport(id.clone()));
        }
    }
    Ok(())
}

fn validate_phase(phase: &PhaseSpec, spec: &RoadmapSpec) -> Result<(), Failure> {
    let l = spec.limits;
    validate_prose("phase title", &phase.title, l.prose_bytes)?;
    validate_prose("phase intent", &phase.intent, l.prose_bytes)?;
    bounded(
        "outputs_per_phase",
        phase.outputs.len(),
        l.outputs_per_phase,
    )?;
    match &phase.body {
        PhaseBody::Leaf { checkpoints } => {
            bounded(
                "checkpoints_per_phase",
                checkpoints.len(),
                l.checkpoints_per_phase,
            )?;
            unique(checkpoints.iter().map(|x| x.id.to_string()), "checkpoint")?;
            for c in checkpoints {
                validate_prose("checkpoint", &c.statement, l.prose_bytes)?;
            }
        }
        PhaseBody::Composite { children } => {
            bounded("children_per_phase", children.len(), l.children_per_phase)?;
            unique(children.iter().map(ToString::to_string), "child")?;
        }
    }
    validate_guide(&phase.guide, l)?;
    for dep in &phase.dependencies {
        validate_dependency(dep, spec)?;
    }
    if let PhaseOrigin::Imported { import, .. } = &phase.origin
        && !spec.imports.contains_key(import)
    {
        return Err(Failure::UnpinnedImport(import.clone()));
    }
    Ok(())
}

fn validate_dependency(dep: &PhaseDependency, spec: &RoadmapSpec) -> Result<(), Failure> {
    let r = match dep {
        PhaseDependency::Requires(r) | PhaseDependency::PrefersAfter(r) => r,
        PhaseDependency::Consumes(o) => &o.phase,
    };
    match r {
        PhaseRef::Local(id) if !spec.phases.contains_key(id) => Err(Failure::Missing {
            kind: "local phase",
            id: id.to_string(),
        }),
        PhaseRef::Imported { import, .. } if !spec.imports.contains_key(import) => {
            Err(Failure::UnpinnedImport(import.clone()))
        }
        _ => Ok(()),
    }
}

fn validate_guide(g: &ImplementationGuide, l: Limits) -> Result<(), Failure> {
    bounded("guide_queries", g.uses.len(), l.guide_queries)?;
    bounded("guide_targets", g.change_targets.len(), l.guide_targets)?;
    bounded("guide_promises", g.promises.len(), l.guide_promises)?;
    bounded("guide_sketches", g.sketches.len(), l.guide_sketches)?;
    unique(g.promises.iter().map(|p| p.id().to_string()), "promise")?;
    unique(g.sketches.iter().map(|s| s.id.to_string()), "sketch")?;
    let promise_ids: BTreeSet<_> = g.promises.iter().map(Promise::id).collect();
    let mut bound = BTreeSet::new();
    for s in &g.sketches {
        validate_prose("sketch", &s.body, l.sketch_bytes)?;
        bounded("sketch_bindings", s.bindings.len(), l.sketch_bindings)?;
        unique(
            s.bindings.iter().map(|b| match b {
                SketchBinding::Uses { label, .. } | SketchBinding::Produces { label, .. } => {
                    label.clone()
                }
            }),
            "binding label",
        )?;
        for b in &s.bindings {
            match b {
                SketchBinding::Uses { label, query } if !g.uses.contains(query) => {
                    return Err(Failure::InvalidBinding {
                        sketch: s.id.clone(),
                        label: label.clone(),
                    });
                }
                SketchBinding::Produces { label, promise } if !promise_ids.contains(promise) => {
                    return Err(Failure::InvalidBinding {
                        sketch: s.id.clone(),
                        label: label.clone(),
                    });
                }
                SketchBinding::Produces { promise, .. } => {
                    bound.insert(promise.clone());
                }
                _ => {}
            }
        }
    }
    for p in promise_ids {
        if !bound.contains(p) {
            return Err(Failure::UnboundPromise(p.clone()));
        }
    }
    Ok(())
}

fn validate_prose(kind: &'static str, s: &str, max: usize) -> Result<(), Failure> {
    if s.trim().is_empty() {
        return Err(Failure::InvalidText {
            kind,
            reason: "empty",
        });
    }
    if s.chars().any(|c| c == '\0') {
        return Err(Failure::InvalidText {
            kind,
            reason: "NUL",
        });
    }
    bounded(kind, s.len(), max)
}
fn bounded(limit: &'static str, actual: usize, maximum: usize) -> Result<(), Failure> {
    if actual > maximum {
        Err(Failure::OverLimit {
            limit,
            actual,
            maximum,
        })
    } else {
        Ok(())
    }
}
fn unique<I: IntoIterator<Item = String>>(items: I, kind: &'static str) -> Result<(), Failure> {
    let mut seen = BTreeSet::new();
    for id in items {
        if !seen.insert(id.clone()) {
            return Err(Failure::Duplicate { kind, id });
        }
    }
    Ok(())
}
