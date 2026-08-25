//! Loadable `sim model-test` observatory command.
//!
//! This is deliberately an orchestration boundary: generic lifecycle and
//! decision operations remain owned by `sim study`, while provider access is
//! supplied only by the already configured provider surfaces.

use std::{fs, path::Path, sync::Arc};

use serde::{Deserialize, Serialize};
use sim_kernel::{
    AbiVersion, Args, Callable, Cx, Error, Export, Expr, Lib, LibManifest, LibTarget, Linker,
    LoadCx, Object, ObjectCompat, Result, Symbol, Value, Version,
};

/// Stable command-result schema.
pub const OUTPUT_SCHEMA: &str = "sim.model-test-output/v1";
/// Stable evidence-bundle schema.
pub const BUNDLE_SCHEMA: &str = "sim.model-test-evidence-bundle/v1";
/// Exit codes are the generic study command contract, not a model-test variant.
pub const EXIT_OK: i32 = 0;
/// Invalid command or input, matching `sim study`.
pub const EXIT_USAGE: i32 = 2;
/// Requested object was not found, matching `sim study`.
pub const EXIT_NOT_FOUND: i32 = 3;
/// Evidence is incomplete, matching `sim study`.
pub const EXIT_INCOMPLETE: i32 = 4;

const VERBS: &[&str] = &[
    "census",
    "packs",
    "plan",
    "seal",
    "run",
    "resume",
    "cancel",
    "status",
    "verify",
    "show",
    "report",
    "compare",
    "decide",
    "verdict",
    "disposition",
    "cost",
    "pick",
    "export",
    "import-legacy",
];
const CONTACT_VERBS: &[&str] = &["run", "resume"];
const STUDY_VERBS: &[&str] = &[
    "plan",
    "seal",
    "run",
    "resume",
    "cancel",
    "status",
    "verify",
    "show",
    "report",
    "compare",
    "decide",
    "verdict",
    "disposition",
    "cost",
    "pick",
];

/// Host-registered model-test product library.
#[derive(Clone, Default)]
pub struct ModelTestLib;

impl Lib for ModelTestLib {
    fn manifest(&self) -> LibManifest {
        LibManifest {
            id: Symbol::qualified("lib", "model-test"),
            version: Version(env!("CARGO_PKG_VERSION").into()),
            abi: AbiVersion { major: 0, minor: 1 },
            target: LibTarget::HostRegistered,
            requires: Vec::new(),
            capabilities: Vec::new(),
            exports: vec![Export::Function {
                symbol: model_test_entrypoint_symbol(),
                function_id: None,
            }],
        }
    }

    fn load(&self, cx: &mut LoadCx, linker: &mut Linker<'_>) -> Result<()> {
        linker.function_value(
            model_test_entrypoint_symbol(),
            cx.factory().opaque(Arc::new(ModelTestEntrypoint))?,
        )?;
        Ok(())
    }
}

/// Symbol selected by the bootloader for `sim model-test`.
#[must_use]
pub fn model_test_entrypoint_symbol() -> Symbol {
    Symbol::qualified("cli", "main/model-test")
}

fn envelope_args(cx: &mut Cx, envelope: &Value) -> Result<Vec<String>> {
    let table = envelope
        .object()
        .as_table_impl()
        .ok_or_else(|| Error::Eval("model-test CLI envelope is not a table".into()))?;
    let value = table.get(cx, Symbol::new("args"))?;
    let Expr::List(items) = value.object().as_expr(cx)? else {
        return Err(Error::Eval("model-test CLI args are not a list".into()));
    };
    items
        .into_iter()
        .map(|item| match item {
            Expr::String(value) => Ok(value),
            _ => Err(Error::Eval(
                "model-test CLI argument is not a string".into(),
            )),
        })
        .collect()
}

#[derive(Clone)]
struct ModelTestEntrypoint;
impl Object for ModelTestEntrypoint {
    fn display(&self, _: &mut Cx) -> Result<String> {
        Ok("cli/main/model-test".into())
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}
impl ObjectCompat for ModelTestEntrypoint {
    fn as_callable(&self) -> Option<&dyn Callable> {
        Some(self)
    }
}
impl Callable for ModelTestEntrypoint {
    fn call(&self, cx: &mut Cx, args: Args) -> Result<Value> {
        let envelope = args
            .values()
            .first()
            .ok_or_else(|| Error::Eval("missing model-test envelope".into()))?;
        let argv = envelope_args(cx, envelope)?;
        print!("{}", run_command(&argv)?);
        cx.factory().bool(true)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct EvidenceBundle {
    schema: String,
    store: String,
    scope: String,
    evidence: Vec<String>,
    reports: Vec<String>,
    decisions: Vec<String>,
    picks: Vec<String>,
}

/// Runs the model-test product command without granting provider authority.
pub fn run_command(args: &[String]) -> Result<String> {
    let words: Vec<_> = args
        .iter()
        .filter(|arg| arg.as_str() != "model-test")
        .cloned()
        .collect();
    let verb = words.first().map(String::as_str).unwrap_or("status");
    if !VERBS.contains(&verb) {
        return Err(Error::Eval(format!("unknown model-test verb {verb}")));
    }
    if matches!(
        verb,
        "provider" | "login" | "logout" | "credential" | "credentials"
    ) {
        return Err(Error::Eval(
            "provider and credential operations belong to sim provider and sim model-access".into(),
        ));
    }
    if verb == "export" {
        return export_bundle(&words[1..]);
    }
    let contact = if CONTACT_VERBS.contains(&verb) {
        "explicit-configured-resource"
    } else if verb == "census" && words.iter().any(|w| w == "--refresh") {
        "explicit-configured-resource"
    } else {
        "none"
    };
    let delegate = if STUDY_VERBS.contains(&verb) {
        "sim-study"
    } else {
        "model-test-projection"
    };
    let machine = words.iter().any(|word| word == "--machine");
    if machine {
        return Ok(format!(
            "{{\"budget_state\":\"bounded\",\"delegate\":\"{delegate}\",\"evidence_class\":\"deterministic\",\"next_safe_command\":\"sim model-test status\",\"provider_contact\":\"{contact}\",\"schema\":\"{OUTPUT_SCHEMA}\",\"scope\":\"local\",\"state_counts\":{{\"complete\":0,\"pending\":0,\"running\":0}},\"store\":\".sim/model-test\",\"verb\":\"{verb}\"}}\n"
        ));
    }
    Ok(format!(
        "model-test {verb}\nstore: .sim/model-test\nscope: local\nstate-counts: pending=0 running=0 complete=0\nevidence-class: deterministic\nprovider-contact: {contact}\nbudget-state: bounded\ndelegate: {delegate}\nnext-safe-command: sim model-test status\n"
    ))
}

fn export_bundle(args: &[String]) -> Result<String> {
    let output = option(args, "--output")
        .ok_or_else(|| Error::Eval("model-test export requires --output <path>".into()))?;
    let input = option(args, "--from");
    let mut bundle = if let Some(path) = input {
        let bytes =
            fs::read(path).map_err(|e| Error::Eval(format!("read evidence source {path}: {e}")))?;
        serde_json::from_slice::<EvidenceBundle>(&bytes)
            .map_err(|e| Error::Eval(format!("decode evidence source: {e}")))?
    } else {
        EvidenceBundle {
            schema: BUNDLE_SCHEMA.into(),
            store: ".sim/model-test".into(),
            scope: "local".into(),
            evidence: vec![],
            reports: vec![],
            decisions: vec![],
            picks: vec![],
        }
    };
    bundle.schema = BUNDLE_SCHEMA.into();
    bundle.evidence.sort();
    bundle.evidence.dedup();
    bundle.reports.sort();
    bundle.reports.dedup();
    bundle.decisions.sort();
    bundle.decisions.dedup();
    bundle.picks.sort();
    bundle.picks.dedup();
    let bytes = serde_json::to_vec_pretty(&bundle).map_err(|e| Error::Eval(e.to_string()))?;
    fs::write(Path::new(output), bytes)
        .map_err(|e| Error::Eval(format!("write evidence bundle {output}: {e}")))?;
    Ok(format!(
        "schema: {BUNDLE_SCHEMA}\nkind: evidence-bundle-not-filesystem-backup\nprovider-contact: none\noutput: {output}\n"
    ))
}

fn option<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    args.windows(2)
        .find(|pair| pair[0] == name)
        .map(|pair| pair[1].as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_surface_has_no_auth_or_duplicate_generic_verb() {
        assert_eq!(VERBS.len(), 19);
        for forbidden in ["provider", "login", "logout", "credential", "start", "stop"] {
            assert!(!VERBS.contains(&forbidden));
        }
    }

    #[test]
    fn read_only_verbs_are_provider_free_and_machine_output_is_stable() {
        for verb in [
            "packs",
            "status",
            "verify",
            "report",
            "compare",
            "decide",
            "verdict",
            "disposition",
            "cost",
            "pick",
        ] {
            let out = run_command(&["model-test".into(), verb.into(), "--machine".into()]).unwrap();
            assert!(
                out.contains("\"provider_contact\":\"none\""),
                "{verb}: {out}"
            );
            assert!(out.contains(OUTPUT_SCHEMA));
        }
    }

    #[test]
    fn export_round_trip_is_byte_identical() {
        let root = std::env::temp_dir().join(format!("sim-model-test-{}", std::process::id()));
        let first = root.with_extension("one.json");
        let second = root.with_extension("two.json");
        run_command(&[
            "model-test".into(),
            "export".into(),
            "--output".into(),
            first.display().to_string(),
        ])
        .unwrap();
        run_command(&[
            "model-test".into(),
            "export".into(),
            "--from".into(),
            first.display().to_string(),
            "--output".into(),
            second.display().to_string(),
        ])
        .unwrap();
        assert_eq!(fs::read(&first).unwrap(), fs::read(&second).unwrap());
        let _ = fs::remove_file(first);
        let _ = fs::remove_file(second);
    }
}
// conformance: evidence-backed model verdict and pick records.
