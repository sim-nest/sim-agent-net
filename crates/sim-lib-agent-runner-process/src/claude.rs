use crate::{BrokerProcessSpec, ProcessProtocol, ProcessRunner, run_broker_process};
use serde_json::Value;
use sim_kernel::{Cx, Error, Expr, Result, Symbol};
use sim_lib_agent_runner_core::{ModelRequest, ModelResponse, ModelRunner};
use sim_lib_exec::{
    ArgAtom, BindingValue, PrivateArtifactRef, ProcessCancellation, ProgramRef, ProjectRootRef,
    SealedBindings,
};
use sim_lib_provider::{
    AuthMethod, ClaudeCliConfigHome, ClaudeCliProbe, ProviderAdapter, ProviderFamilyCard,
    ProviderRegistry, ProviderSeatCard, SessionStatus, claude_cli_family,
};
use std::{sync::Arc, time::Duration};

const OUTPUT_SCHEMA: &str = "claude-stream-json/1";

/// Claude CLI provider adapter over explicitly configured, opaque config homes.
#[derive(Clone, Debug)]
pub struct ClaudeCliAdapter {
    program: ProgramRef,
    homes: Vec<ClaudeCliConfigHome>,
    timeout: Duration,
    max_output_bytes: usize,
}

impl ClaudeCliAdapter {
    /// Creates an adapter. Discovery is limited to these host-declared homes.
    pub fn new(
        program: ProgramRef,
        homes: Vec<ClaudeCliConfigHome>,
        timeout: Duration,
        max_output_bytes: usize,
    ) -> Result<Self> {
        if homes.is_empty() {
            return Err(Error::Eval(
                "Claude CLI requires at least one configured home".into(),
            ));
        }
        let mut labels = homes
            .iter()
            .map(|home| home.label.as_str())
            .collect::<Vec<_>>();
        labels.sort_unstable();
        if labels.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err(Error::Eval(
                "Claude CLI config-home labels must be unique".into(),
            ));
        }
        if homes.iter().any(|home| home.max_turns == 0) {
            return Err(Error::Eval("Claude CLI max-turns must be non-zero".into()));
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
        home: &ClaudeCliConfigHome,
        argv: &[String],
        label: &str,
    ) -> Result<BrokerProcessSpec> {
        let artifact = PrivateArtifactRef::new(&home.artifact)?;
        BrokerProcessSpec::new(
            self.program.clone(),
            argv.iter().map(ArgAtom::new).collect::<Result<Vec<_>>>()?,
            ProjectRootRef::new(&home.label)?,
            SealedBindings::try_from_entries([(
                "CLAUDE_CONFIG_DIR".into(),
                BindingValue::PrivateArtifact(artifact.clone()),
            )])?,
            vec![artifact],
            label,
            self.timeout,
            self.max_output_bytes,
        )
    }

    fn probe(&self, cx: &Cx, home: &ClaudeCliConfigHome) -> Result<ClaudeCliProbe> {
        home.terms_policy.enforce()?;
        let version = run_text(
            cx,
            &self.spec(home, &["--version".into()], "claude-version")?,
        )?;
        if version != home.expected_version {
            return Err(Error::Eval(format!(
                "Claude CLI version drifted: expected {}, observed {version}",
                home.expected_version
            )));
        }
        let help = run_text(
            cx,
            &self.spec(home, &["--help".into()], "claude-mode-probe")?,
        )?;
        for flag in [
            "--print",
            "--output-format",
            "stream-json",
            "--model",
            "--permission-mode",
            "--max-turns",
        ] {
            if !help.contains(flag) {
                return Err(Error::Eval(format!(
                    "Claude CLI machine-readable mode or required flag {flag} is unsupported"
                )));
            }
        }
        if home.max_budget_usd.is_some() && !help.contains("--max-budget-usd") {
            return Err(Error::Eval("Claude CLI spend bound is unsupported".into()));
        }
        let status = run_text(
            cx,
            &self.spec(
                home,
                &["auth".into(), "status".into(), "--json".into()],
                "claude-auth-probe",
            )?,
        )?;
        decode_status(&status, version)
    }

    fn home_for<'a>(&'a self, seat: &ProviderSeatCard) -> Result<&'a ClaudeCliConfigHome> {
        self.homes
            .iter()
            .find(|home| seat.seat.label == home.label)
            .ok_or_else(|| Error::Eval("Claude CLI seat no longer has a configured home".into()))
    }
}

impl ProviderAdapter for ClaudeCliAdapter {
    fn family(&self) -> ProviderFamilyCard {
        claude_cli_family()
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
        let home = self.home_for(seat)?;
        home.terms_policy.enforce()?;
        seat.auth_metadata()?
            .ok_or_else(|| Error::Eval("Claude CLI seat lacks terms metadata".into()))?
            .require_terms()?;
        if !matches!(
            seat.auth_metadata()?.map(|metadata| metadata.session),
            Some(SessionStatus::Authenticated { .. })
        ) {
            return Err(Error::Eval(
                "Claude CLI login required; run the vendor CLI login flow".into(),
            ));
        }
        let model = seat
            .model
            .clone()
            .ok_or_else(|| Error::Eval("Claude CLI seat has no model selection".into()))?;
        let mut argv = vec![
            "--print".into(),
            "--output-format".into(),
            "stream-json".into(),
            "--model".into(),
            model.clone(),
            "--permission-mode".into(),
            home.permission_mode.clone(),
            "--max-turns".into(),
            home.max_turns.to_string(),
        ];
        if let Some(budget) = &home.max_budget_usd {
            argv.extend(["--max-budget-usd".into(), budget.clone()]);
        }
        argv.push("-".into());
        let spec = self.spec(home, &argv, "claude-print")?;
        Ok(Arc::new(ClaudeRunner {
            model: model.clone(),
            inner: ProcessRunner::new(
                Symbol::qualified("runner", "claude-cli"),
                model,
                spec,
                ProcessProtocol::LineText,
            ),
        }))
    }

    fn auth_methods(&self, _cx: &mut Cx) -> Result<Vec<AuthMethod>> {
        Ok(vec![AuthMethod::Subscription, AuthMethod::OauthBrowser])
    }

    fn status(&self, cx: &mut Cx, seat: &ProviderSeatCard) -> Result<SessionStatus> {
        Ok(self.probe(cx, self.home_for(seat)?)?.session)
    }
}

/// Registers Claude CLI as one ordinary provider family.
pub fn register_claude_cli(
    registry: &mut ProviderRegistry,
    adapter: ClaudeCliAdapter,
) -> Result<()> {
    registry.register(Arc::new(adapter))
}

#[derive(Clone, Debug)]
struct ClaudeRunner {
    model: String,
    inner: ProcessRunner,
}

impl ModelRunner for ClaudeRunner {
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
            .ok_or_else(|| Error::Eval("Claude CLI response lacks stream-json output".into()))?;
        decode_stream_json(text, &self.model)
    }
}

fn decode_status(stdout: &str, version: String) -> Result<ClaudeCliProbe> {
    let value: Value = serde_json::from_str(stdout)
        .map_err(|error| Error::Eval(format!("Claude CLI malformed auth output: {error}")))?;
    let schema = value
        .get("schema")
        .and_then(Value::as_str)
        .ok_or_else(|| Error::Eval("Claude CLI auth output lacks schema".into()))?;
    if schema != OUTPUT_SCHEMA {
        return Err(Error::Eval(format!(
            "Claude CLI unknown output schema {schema}"
        )));
    }
    let status = match value.get("status").and_then(Value::as_str) {
        Some("authenticated") => SessionStatus::Authenticated {
            principal_label: value
                .get("principal")
                .and_then(Value::as_str)
                .map(str::to_owned),
        },
        Some("login-required") | Some("expired-token") => SessionStatus::LoginRequired,
        Some("browser-handoff") => SessionStatus::BrowserHandoff {
            url: value
                .get("url")
                .and_then(Value::as_str)
                .ok_or_else(|| Error::Eval("Claude CLI browser handoff lacks URL".into()))?
                .to_owned(),
        },
        Some(other) => {
            return Err(Error::Eval(format!(
                "Claude CLI unknown auth status {other}"
            )));
        }
        None => return Err(Error::Eval("Claude CLI auth output lacks status".into())),
    };
    let principal_label = match &status {
        SessionStatus::Authenticated { principal_label } => principal_label.clone(),
        _ => None,
    };
    Ok(ClaudeCliProbe {
        version,
        machine_mode: "print-stream-json".into(),
        output_schema: schema.into(),
        session: status,
        principal_label,
    })
}

fn decode_stream_json(stdout: &str, model: &str) -> Result<ModelResponse> {
    let mut answer = None;
    let mut completed = false;
    for line in stdout.lines().filter(|line| !line.trim().is_empty()) {
        let event: Value = serde_json::from_str(line).map_err(|error| {
            Error::Eval(format!("Claude CLI malformed stream-json output: {error}"))
        })?;
        if event.get("schema").and_then(Value::as_str) != Some(OUTPUT_SCHEMA) {
            return Err(Error::Eval(
                "Claude CLI stream event has unknown schema".into(),
            ));
        }
        match event.get("type").and_then(Value::as_str) {
            Some("assistant") => {
                answer = event.get("text").and_then(Value::as_str).map(str::to_owned)
            }
            Some("result") if event.get("subtype").and_then(Value::as_str) == Some("success") => {
                completed = true
            }
            Some("result") => {
                return Err(Error::Eval(format!(
                    "Claude CLI refused task: {}",
                    event
                        .get("error")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown refusal")
                )));
            }
            Some(_) => {}
            None => return Err(Error::Eval("Claude CLI stream event lacks type".into())),
        }
    }
    if !completed {
        return Err(Error::Eval(
            "Claude CLI output ended before successful result".into(),
        ));
    }
    let answer = answer
        .ok_or_else(|| Error::Eval("Claude CLI completed without an assistant message".into()))?;
    Ok(ModelResponse::new(
        Symbol::qualified("runner", "claude-cli"),
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
    .map_err(|_| Error::Eval("Claude CLI probe returned non-UTF-8 output".into()))
}

#[cfg(test)]
mod tests;
