use crate::{BrokerProcessSpec, ProcessProtocol, ProcessRunner, run_broker_process};
use serde_json::Value;
use sim_kernel::{Cx, Error, Expr, Result, Symbol};
use sim_lib_agent_runner_core::{ModelRequest, ModelResponse, ModelRunner};
use sim_lib_exec::{
    ArgAtom, BindingValue, PrivateArtifactRef, ProcessCancellation, ProgramRef, ProjectRootRef,
    SealedBindings,
};
use sim_lib_provider::{
    AuthMethod, CodexCliConfigHome, CodexCliProbe, ProviderAdapter, ProviderFamilyCard,
    ProviderRegistry, ProviderSeatCard, SessionStatus, codex_cli_family,
};
use std::{sync::Arc, time::Duration};

/// Codex CLI provider adapter over explicitly configured, opaque config homes.
#[derive(Clone, Debug)]
pub struct CodexCliAdapter {
    program: ProgramRef,
    homes: Vec<CodexCliConfigHome>,
    timeout: Duration,
    max_output_bytes: usize,
}

impl CodexCliAdapter {
    /// Creates an adapter. Discovery is limited to these host-declared homes.
    pub fn new(
        program: ProgramRef,
        homes: Vec<CodexCliConfigHome>,
        timeout: Duration,
        max_output_bytes: usize,
    ) -> Result<Self> {
        if homes.is_empty() {
            return Err(Error::Eval(
                "Codex CLI requires at least one configured home".into(),
            ));
        }
        let mut labels = homes
            .iter()
            .map(|home| home.label.as_str())
            .collect::<Vec<_>>();
        labels.sort_unstable();
        if labels.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err(Error::Eval(
                "Codex CLI config-home labels must be unique".into(),
            ));
        }
        Ok(Self {
            program,
            homes,
            timeout,
            max_output_bytes,
        })
    }

    fn spec(
        &self,
        home: &CodexCliConfigHome,
        argv: &[&str],
        label: &str,
    ) -> Result<BrokerProcessSpec> {
        let artifact = PrivateArtifactRef::new(&home.artifact)?;
        BrokerProcessSpec::new(
            self.program.clone(),
            argv.iter()
                .map(|arg| ArgAtom::new(*arg))
                .collect::<Result<Vec<_>>>()?,
            ProjectRootRef::new(&home.workspace_posture)?,
            SealedBindings::try_from_entries([(
                "CODEX_HOME".into(),
                BindingValue::PrivateArtifact(artifact.clone()),
            )])?,
            vec![artifact],
            label,
            self.timeout,
            self.max_output_bytes,
        )
    }

    fn probe(&self, cx: &Cx, home: &CodexCliConfigHome) -> Result<CodexCliProbe> {
        let version = run_text(cx, &self.spec(home, &["--version"], "codex-version")?)?;
        if version != home.expected_version {
            return Err(Error::Eval(format!(
                "Codex CLI version drifted: expected {}, observed {version}",
                home.expected_version
            )));
        }
        let exec_help = run_text(
            cx,
            &self.spec(home, &["exec", "--help"], "codex-exec-probe")?,
        )?;
        if !["--json", "--model", "--sandbox"]
            .iter()
            .all(|flag| exec_help.contains(flag))
        {
            return Err(Error::Eval(
                "Codex CLI non-interactive JSON mode is unsupported or drifted".into(),
            ));
        }
        let auth = run_text(
            cx,
            &self.spec(home, &["login", "status"], "codex-auth-probe")?,
        )?;
        let (auth_method, principal_label) =
            if auth.contains("ChatGPT") || auth.contains("subscription") {
                (AuthMethod::Subscription, Some("codex-subscription".into()))
            } else if auth.contains("API key") || auth.contains("api-key") {
                (AuthMethod::ApiKey, Some("codex-api-key".into()))
            } else if auth.contains("not logged in") || auth.contains("Login required") {
                (AuthMethod::BrokerOwned, None)
            } else {
                return Err(Error::Eval("Codex CLI auth status output drifted".into()));
            };
        Ok(CodexCliProbe {
            version,
            machine_mode: "exec-jsonl".into(),
            auth_method,
            output_schema: "codex-exec-jsonl/1".into(),
            principal_label,
        })
    }
}

impl ProviderAdapter for CodexCliAdapter {
    fn family(&self) -> ProviderFamilyCard {
        codex_cli_family()
    }
    fn discover(&self, cx: &mut Cx, _hint: Expr) -> Result<Vec<ProviderSeatCard>> {
        self.homes
            .iter()
            .map(|home| home.seat_card(&self.probe(cx, home)?))
            .collect()
    }
    fn open(
        &self,
        _cx: &mut Cx,
        seat: &ProviderSeatCard,
        _options: Expr,
    ) -> Result<Arc<dyn ModelRunner>> {
        let home = self
            .homes
            .iter()
            .find(|home| seat.seat.label == home.label)
            .ok_or_else(|| Error::Eval("Codex CLI seat no longer has a configured home".into()))?;
        if seat.principal.kind == Symbol::new("broker-owned") {
            return Err(Error::Eval("Codex CLI login required".into()));
        }
        let model = seat
            .model
            .clone()
            .ok_or_else(|| Error::Eval("Codex CLI seat has no model selection".into()))?;
        let spec = self.spec(
            home,
            &[
                "exec",
                "--json",
                "--model",
                &model,
                "--sandbox",
                &home.sandbox_mode,
                "--skip-git-repo-check",
                "-",
            ],
            "codex-exec",
        )?;
        Ok(Arc::new(CodexRunner {
            model: model.clone(),
            inner: ProcessRunner::new(
                Symbol::qualified("runner", "codex-cli"),
                model,
                spec,
                ProcessProtocol::LineText,
            ),
        }))
    }
    fn auth_methods(&self, _cx: &mut Cx) -> Result<Vec<AuthMethod>> {
        Ok(vec![AuthMethod::Subscription, AuthMethod::ApiKey])
    }
    fn status(&self, cx: &mut Cx, seat: &ProviderSeatCard) -> Result<SessionStatus> {
        let home = self
            .homes
            .iter()
            .find(|home| seat.seat.label == home.label)
            .ok_or_else(|| Error::Eval("unknown Codex CLI seat".into()))?;
        let probe = self.probe(cx, home)?;
        Ok(if probe.auth_method == AuthMethod::BrokerOwned {
            SessionStatus::LoginRequired
        } else {
            SessionStatus::Authenticated {
                principal_label: probe.principal_label,
            }
        })
    }
}

/// Registers Codex CLI as one ordinary provider family.
pub fn register_codex_cli(registry: &mut ProviderRegistry, adapter: CodexCliAdapter) -> Result<()> {
    registry.register(Arc::new(adapter))
}

#[derive(Clone, Debug)]
struct CodexRunner {
    model: String,
    inner: ProcessRunner,
}
impl ModelRunner for CodexRunner {
    fn card(&self) -> sim_lib_agent_runner_core::ModelCard {
        self.inner.card()
    }
    fn infer(&self, cx: &mut Cx, request: ModelRequest) -> Result<ModelResponse> {
        let raw = self.inner.infer_inner(cx, request)?;
        let text = raw
            .extra
            .iter()
            .find_map(|(key, value)| (key == &Expr::Symbol(Symbol::new("text"))).then_some(value))
            .and_then(|value| match value {
                Expr::String(text) => Some(text),
                _ => None,
            })
            .ok_or_else(|| Error::Eval("Codex CLI response lacks JSONL output".into()))?;
        decode_exec_jsonl(text, &self.model)
    }
}

fn decode_exec_jsonl(stdout: &str, model: &str) -> Result<ModelResponse> {
    let mut answer = None;
    let mut completed = false;
    for line in stdout.lines().filter(|line| !line.trim().is_empty()) {
        let event: Value = serde_json::from_str(line)
            .map_err(|error| Error::Eval(format!("Codex CLI malformed JSON output: {error}")))?;
        match event.get("type").and_then(Value::as_str) {
            Some("item.completed")
                if event.pointer("/item/type").and_then(Value::as_str) == Some("agent_message") =>
            {
                answer = event
                    .pointer("/item/text")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            }
            Some("turn.completed") => completed = true,
            Some("error") => {
                return Err(Error::Eval(format!(
                    "Codex CLI refused task: {}",
                    event
                        .get("message")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown refusal")
                )));
            }
            Some(_) => {}
            None => return Err(Error::Eval("Codex CLI event lacks type".into())),
        }
    }
    if !completed {
        return Err(Error::Eval(
            "Codex CLI output ended before turn.completed".into(),
        ));
    }
    let answer =
        answer.ok_or_else(|| Error::Eval("Codex CLI completed without an agent message".into()))?;
    Ok(ModelResponse::new(
        Symbol::qualified("runner", "codex-cli"),
        model,
        vec![Expr::String(answer)],
        Symbol::new("stop"),
    ))
}

fn run_text(cx: &Cx, spec: &BrokerProcessSpec) -> Result<String> {
    String::from_utf8(run_broker_process(
        cx,
        spec,
        Vec::new(),
        &ProcessCancellation::default(),
    )?)
    .map(|value| value.trim().to_owned())
    .map_err(|_| Error::Eval("Codex CLI probe returned non-UTF-8 output".into()))
}
#[cfg(test)]
mod tests;
