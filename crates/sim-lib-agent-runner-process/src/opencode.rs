use crate::{BrokerProcessSpec, ProcessProtocol, ProcessRunner, run_broker_process};
use serde_json::Value;
use sim_kernel::{Cx, Error, Expr, Result, Symbol};
use sim_lib_agent_runner_core::{ModelRequest, ModelResponse, ModelRunner};
use sim_lib_exec::{
    ArgAtom, BindingValue, PrivateArtifactRef, ProcessCancellation, ProgramRef, ProjectRootRef,
    SealedBindings,
};
use sim_lib_provider::{
    AuthMethod, OpenCodeConfig, OpenCodeProbe, OpenCodeTransport, ProviderAdapter,
    ProviderFamilyCard, ProviderRegistry, ProviderSeatCard, SessionStatus, opencode_cli_family,
};
use std::{sync::Arc, time::Duration};

/// OpenCode adapter over an explicit set of config homes and server endpoints.
#[derive(Clone, Debug)]
pub struct OpenCodeCliAdapter {
    program: ProgramRef,
    configs: Vec<OpenCodeConfig>,
    timeout: Duration,
    max_output_bytes: usize,
}

impl OpenCodeCliAdapter {
    /// Creates an adapter without consulting OpenCode config or credential files.
    pub fn new(
        program: ProgramRef,
        configs: Vec<OpenCodeConfig>,
        timeout: Duration,
        max_output_bytes: usize,
    ) -> Result<Self> {
        if configs.is_empty() {
            return Err(Error::Eval(
                "OpenCode requires at least one declared config or endpoint".into(),
            ));
        }
        let mut labels = configs.iter().map(|c| c.label.as_str()).collect::<Vec<_>>();
        labels.sort_unstable();
        if labels.windows(2).any(|p| p[0] == p[1]) {
            return Err(Error::Eval("OpenCode seat labels must be unique".into()));
        }
        Ok(Self {
            program,
            configs,
            timeout,
            max_output_bytes,
        })
    }

    fn spec(
        &self,
        config: &OpenCodeConfig,
        argv: &[&str],
        label: &str,
    ) -> Result<BrokerProcessSpec> {
        let artifact = PrivateArtifactRef::new(&config.artifact)?;
        BrokerProcessSpec::new(
            self.program.clone(),
            argv.iter()
                .map(|a| ArgAtom::new(*a))
                .collect::<Result<Vec<_>>>()?,
            ProjectRootRef::new(&config.workspace)?,
            SealedBindings::try_from_entries([(
                "OPENCODE_CONFIG_DIR".into(),
                BindingValue::PrivateArtifact(artifact.clone()),
            )])?,
            vec![artifact],
            label,
            self.timeout,
            self.max_output_bytes,
        )
    }

    fn probe(&self, cx: &Cx, config: &OpenCodeConfig) -> Result<OpenCodeProbe> {
        config.terms_policy.enforce()?;
        if matches!(config.transport, OpenCodeTransport::LocalServer { .. }) {
            return Ok(OpenCodeProbe {
                version: config.expected_version.clone(),
                output_schema: "opencode-server-events/1".into(),
                session: SessionStatus::Authenticated {
                    principal_label: Some(config.provider.clone()),
                },
                observed_catalog_digest: "not-probed".into(),
            });
        }
        let version = run_text(cx, &self.spec(config, &["--version"], "opencode-version")?)?;
        if version != config.expected_version {
            return Err(Error::Eval(format!(
                "OpenCode version drifted: expected {}, observed {version}",
                config.expected_version
            )));
        }
        let identity = run_text(
            cx,
            &self.spec(
                config,
                &["models", "--format", "json"],
                "opencode-catalog-probe",
            )?,
        )?;
        let parsed: Value = serde_json::from_str(&identity)
            .map_err(|e| Error::Eval(format!("OpenCode malformed catalog output: {e}")))?;
        let digest = parsed
            .get("digest")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::Eval("OpenCode catalog evidence lacks digest".into()))?;
        Ok(OpenCodeProbe {
            version,
            output_schema: "opencode-json-events/1".into(),
            session: SessionStatus::Authenticated {
                principal_label: Some(config.provider.clone()),
            },
            observed_catalog_digest: digest.into(),
        })
    }
}

impl ProviderAdapter for OpenCodeCliAdapter {
    fn family(&self) -> ProviderFamilyCard {
        opencode_cli_family()
    }
    fn discover(&self, cx: &mut Cx, _hint: Expr) -> Result<Vec<ProviderSeatCard>> {
        self.configs
            .iter()
            .map(|c| c.seat_card(&self.probe(cx, c)?))
            .collect()
    }
    fn open(
        &self,
        _cx: &mut Cx,
        seat: &ProviderSeatCard,
        _options: Expr,
    ) -> Result<Arc<dyn ModelRunner>> {
        let config = self
            .configs
            .iter()
            .find(|c| c.label == seat.seat.label)
            .ok_or_else(|| Error::Eval("OpenCode seat is no longer declared".into()))?;
        config.terms_policy.enforce()?;
        if seat.endpoint.transport.as_str() != config.transport.name() {
            return Err(Error::Eval(
                "OpenCode transport mismatch; implicit fallback is forbidden".into(),
            ));
        }
        if let OpenCodeTransport::LocalServer { password_ref, .. } = &config.transport {
            if password_ref.is_empty() {
                return Err(Error::Eval(
                    "OpenCode local server requires an opaque password reference".into(),
                ));
            }
            return Err(Error::Eval("OpenCode local-server seat requires the server transport adapter; process fallback is forbidden".into()));
        }
        let spec = self.spec(
            config,
            &[
                "run",
                "--format",
                "json",
                "--provider",
                &config.provider,
                "--model",
                &config.model,
                "--agent",
                &config.agent,
                "-",
            ],
            "opencode-run",
        )?;
        Ok(Arc::new(OpenCodeRunner {
            model: config.model.clone(),
            inner: ProcessRunner::new(
                Symbol::qualified("runner", "opencode-cli"),
                config.model.clone(),
                spec,
                ProcessProtocol::LineText,
            ),
        }))
    }
    fn auth_methods(&self, _cx: &mut Cx) -> Result<Vec<AuthMethod>> {
        Ok(vec![
            AuthMethod::BrokerOwned,
            AuthMethod::Subscription,
            AuthMethod::ApiKey,
        ])
    }
}

/// Registers OpenCode as one ordinary, removable provider family.
pub fn register_opencode_cli(
    registry: &mut ProviderRegistry,
    adapter: OpenCodeCliAdapter,
) -> Result<()> {
    registry.register(Arc::new(adapter))
}

#[derive(Clone, Debug)]
struct OpenCodeRunner {
    model: String,
    inner: ProcessRunner,
}
impl ModelRunner for OpenCodeRunner {
    fn card(&self) -> sim_lib_agent_runner_core::ModelCard {
        self.inner.card()
    }
    fn infer(&self, cx: &mut Cx, request: ModelRequest) -> Result<ModelResponse> {
        let raw = self.inner.infer_inner(cx, request)?;
        let text = raw
            .extra
            .iter()
            .find_map(|(k, v)| (k == &Expr::Symbol(Symbol::new("text"))).then_some(v))
            .and_then(|v| {
                if let Expr::String(s) = v {
                    Some(s)
                } else {
                    None
                }
            })
            .ok_or_else(|| Error::Eval("OpenCode response lacks event output".into()))?;
        decode_events(text, &self.model)
    }
}

fn decode_events(stdout: &str, model: &str) -> Result<ModelResponse> {
    let values: Vec<Value> = if stdout.trim_start().starts_with('[') {
        serde_json::from_str(stdout)
            .map_err(|e| Error::Eval(format!("OpenCode malformed framed event stream: {e}")))?
    } else {
        stdout
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| {
                serde_json::from_str(l)
                    .map_err(|e| Error::Eval(format!("OpenCode malformed JSON event: {e}")))
            })
            .collect::<Result<_>>()?
    };
    let mut answer = None;
    let mut done = false;
    for event in values {
        match event.get("type").and_then(Value::as_str) {
            Some("text") => answer = event.get("text").and_then(Value::as_str).map(str::to_owned),
            Some("done") => done = true,
            Some("error") => {
                return Err(Error::Eval(format!(
                    "OpenCode refused task: {}",
                    event
                        .get("message")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown error")
                )));
            }
            Some(_) => {}
            None => return Err(Error::Eval("OpenCode event lacks type".into())),
        }
    }
    if !done {
        return Err(Error::Eval(
            "OpenCode event stream ended before done".into(),
        ));
    }
    let answer = answer.ok_or_else(|| Error::Eval("OpenCode completed without text".into()))?;
    Ok(ModelResponse::new(
        Symbol::qualified("runner", "opencode-cli"),
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
    .map(|s| s.trim().into())
    .map_err(|_| Error::Eval("OpenCode probe returned non-UTF-8 output".into()))
}

#[cfg(test)]
mod tests;
