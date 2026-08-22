use crate::{BrokerProcessSpec, run_broker_process};
use serde_json::{Value, json};
use sim_kernel::{Cx, Error, Result};
use sim_lib_exec::ProcessCancellation;
use sim_lib_provider::{AuthMethod, BrokerRevision, ProviderControlResult, SessionStatus};

/// Machine-readable session controller for a sealed broker process.
#[derive(Clone, Debug)]
pub struct BrokerSessionController {
    process: BrokerProcessSpec,
    expected: BrokerRevision,
}

impl BrokerSessionController {
    /// Creates a controller pinned to an exact broker compatibility declaration.
    pub fn new(process: BrokerProcessSpec, expected: BrokerRevision) -> Result<Self> {
        if process.request().program.as_str() != expected.executable_path {
            return Err(Error::Eval(
                "broker executable path does not match its revision declaration".into(),
            ));
        }
        if expected.machine_mode.is_empty() || expected.event_schema.is_empty() {
            return Err(Error::Eval(
                "broker machine mode and event schema must be declared".into(),
            ));
        }
        Ok(Self { process, expected })
    }

    /// Lists authentication methods through `provider/auth-methods`.
    pub fn auth_methods(&self, cx: &Cx) -> Result<Vec<AuthMethod>> {
        match self.control(cx, "provider/auth-methods", None)? {
            ProviderControlResult::AuthMethods(methods) => Ok(methods),
            _ => Err(Error::Eval(
                "broker returned the wrong auth-methods result".into(),
            )),
        }
    }

    /// Starts a typed login flow through `provider/login`.
    pub fn login(&self, cx: &Cx, method: AuthMethod) -> Result<SessionStatus> {
        match self.control(cx, "provider/login", Some(method))? {
            ProviderControlResult::Session(status) => Ok(status),
            _ => Err(Error::Eval("broker returned the wrong login result".into())),
        }
    }

    /// Queries the broker session through `provider/status`.
    pub fn status(&self, cx: &Cx) -> Result<SessionStatus> {
        match self.control(cx, "provider/status", None)? {
            ProviderControlResult::Session(status) => Ok(status),
            _ => Err(Error::Eval(
                "broker returned the wrong status result".into(),
            )),
        }
    }

    /// Ends the broker session through `provider/logout`.
    pub fn logout(&self, cx: &Cx) -> Result<()> {
        match self.control(cx, "provider/logout", None)? {
            ProviderControlResult::LoggedOut => Ok(()),
            _ => Err(Error::Eval(
                "broker returned the wrong logout result".into(),
            )),
        }
    }

    fn control(
        &self,
        cx: &Cx,
        operation: &str,
        method: Option<AuthMethod>,
    ) -> Result<ProviderControlResult> {
        let stdin = serde_json::to_vec(&json!({
            "operation": operation,
            "auth_method": method.map(|value| value.symbol().to_string()),
        }))
        .map_err(|error| Error::Eval(format!("cannot encode broker control request: {error}")))?;
        let stdout = run_broker_process(cx, &self.process, stdin, &ProcessCancellation::default())?;
        let event: Value = serde_json::from_slice(&stdout)
            .map_err(|error| Error::Eval(format!("malformed broker event: {error}")))?;
        self.check_revision(&event)?;
        decode_result(operation, &event)
    }

    fn check_revision(&self, event: &Value) -> Result<()> {
        let broker = event
            .get("broker")
            .and_then(Value::as_object)
            .ok_or_else(|| Error::Eval("broker event lacks a revision object".into()))?;
        check_string(broker.get("version"), &self.expected.version, "version")?;
        check_string(
            broker.get("machine_mode"),
            &self.expected.machine_mode,
            "machine mode",
        )?;
        check_string(
            broker.get("event_schema"),
            &self.expected.event_schema,
            "event schema",
        )?;
        let methods = broker
            .get("auth_methods")
            .and_then(Value::as_array)
            .ok_or_else(|| Error::Eval("broker event lacks supported auth methods".into()))?
            .iter()
            .map(parse_method)
            .collect::<Result<Vec<_>>>()?;
        if methods != self.expected.auth_methods {
            return Err(Error::Eval("broker supported auth methods drifted".into()));
        }
        Ok(())
    }
}

fn check_string(value: Option<&Value>, expected: &str, label: &str) -> Result<()> {
    if value.and_then(Value::as_str) == Some(expected) {
        Ok(())
    } else {
        Err(Error::Eval(format!("broker {label} drifted")))
    }
}

fn parse_method(value: &Value) -> Result<AuthMethod> {
    let value = value
        .as_str()
        .ok_or_else(|| Error::Eval("broker auth method is not a string".into()))?;
    AuthMethod::from_symbol(&sim_kernel::Symbol::new(value))
}

fn decode_result(operation: &str, event: &Value) -> Result<ProviderControlResult> {
    let result = event
        .get("result")
        .and_then(Value::as_object)
        .ok_or_else(|| Error::Eval("broker event lacks a typed result".into()))?;
    match operation {
        "provider/auth-methods" => Ok(ProviderControlResult::AuthMethods(
            result
                .get("methods")
                .and_then(Value::as_array)
                .ok_or_else(|| Error::Eval("auth-methods result is malformed".into()))?
                .iter()
                .map(parse_method)
                .collect::<Result<Vec<_>>>()?,
        )),
        "provider/login" | "provider/status" => {
            Ok(ProviderControlResult::Session(decode_status(result)?))
        }
        "provider/logout" if result.get("logged_out").and_then(Value::as_bool) == Some(true) => {
            Ok(ProviderControlResult::LoggedOut)
        }
        "provider/logout" => Err(Error::Eval("broker did not confirm logout".into())),
        _ => Err(Error::Eval("unknown broker control operation".into())),
    }
}

fn decode_status(result: &serde_json::Map<String, Value>) -> Result<SessionStatus> {
    match result.get("status").and_then(Value::as_str) {
        Some("logged-out") => Ok(SessionStatus::LoggedOut),
        Some("login-required") => Ok(SessionStatus::LoginRequired),
        Some("browser-handoff") => Ok(SessionStatus::BrowserHandoff {
            url: required_string(result, "url")?,
        }),
        Some("device-handoff") => Ok(SessionStatus::DeviceHandoff {
            url: required_string(result, "url")?,
            user_code: required_string(result, "user_code")?,
        }),
        Some("authenticated") => Ok(SessionStatus::Authenticated {
            principal_label: result
                .get("principal_label")
                .and_then(Value::as_str)
                .map(str::to_owned),
        }),
        Some(other) => Err(Error::Eval(format!(
            "unknown broker session status {other}"
        ))),
        None => Err(Error::Eval("broker session result lacks status".into())),
    }
}

fn required_string(result: &serde_json::Map<String, Value>, key: &str) -> Result<String> {
    result
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| Error::Eval(format!("broker session result lacks {key}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bind_process_port;
    use sim_lib_exec::{
        ArgAtom, BindingValue, PrivateArtifactRef, ProcResult, ProcessAttempt, ProcessPort,
        ProcessReceipt, ProcessRequest, ProgramRef, ProjectRootRef, SealedBindings,
    };
    use std::{
        sync::{Arc, Mutex},
        time::Duration,
    };

    struct FixturePort {
        outputs: Mutex<Vec<String>>,
    }
    impl ProcessPort for FixturePort {
        fn run(
            &self,
            _request: &ProcessRequest,
            _cancellation: &ProcessCancellation,
        ) -> ProcessAttempt {
            let stdout = self.outputs.lock().unwrap().pop().unwrap();
            ProcessAttempt::Completed {
                receipt: ProcessReceipt {
                    provider: "fixture".into(),
                    elapsed_mono_ns: 1,
                    result: ProcResult {
                        stdout,
                        stderr: String::new(),
                        exit_code: 0,
                        truncated: false,
                    },
                },
            }
        }
    }

    fn controller(outputs: Vec<String>) -> (Cx, BrokerSessionController) {
        let artifact = PrivateArtifactRef::new("broker-config").unwrap();
        let spec = BrokerProcessSpec::new(
            ProgramRef::new("provider-cli").unwrap(),
            vec![ArgAtom::new("--machine").unwrap()],
            ProjectRootRef::new("broker-seat").unwrap(),
            SealedBindings::try_from_entries([(
                "CONFIG".into(),
                BindingValue::PrivateArtifact(artifact.clone()),
            )])
            .unwrap(),
            vec![artifact],
            "broker-control",
            Duration::from_secs(1),
            4096,
        )
        .unwrap();
        let expected = BrokerRevision {
            executable_path: "provider-cli".into(),
            version: "1.2.3".into(),
            machine_mode: "json-events".into(),
            auth_methods: vec![AuthMethod::OauthBrowser, AuthMethod::BrokerOwned],
            event_schema: "provider-events/1".into(),
        };
        let mut cx = Cx::new(
            Arc::new(sim_kernel::eval::NoopEvalPolicy),
            Arc::new(sim_kernel::DefaultFactory),
            sim_kernel::HandleSeed::new(0x4252_4f4b),
        );
        bind_process_port(
            &mut cx,
            Arc::new(FixturePort {
                outputs: Mutex::new(outputs.into_iter().rev().collect()),
            }),
        )
        .unwrap();
        (cx, BrokerSessionController::new(spec, expected).unwrap())
    }

    fn event(result: Value) -> String {
        json!({ "broker": { "version": "1.2.3", "machine_mode": "json-events", "auth_methods": ["oauth-browser", "broker-owned"], "event_schema": "provider-events/1" }, "result": result }).to_string()
    }

    #[test]
    fn version_drift_and_malformed_events_fail_closed() {
        let drift = json!({ "broker": { "version": "2.0.0", "machine_mode": "json-events", "auth_methods": ["oauth-browser", "broker-owned"], "event_schema": "provider-events/1" }, "result": {"status":"login-required"} }).to_string();
        let (cx, broker) = controller(vec![drift]);
        assert!(
            broker
                .status(&cx)
                .unwrap_err()
                .to_string()
                .contains("version drifted")
        );
        let (cx, broker) = controller(vec!["welcome to provider cli\n> ".into()]);
        assert!(
            broker
                .status(&cx)
                .unwrap_err()
                .to_string()
                .contains("malformed broker event")
        );
    }

    #[test]
    fn login_browser_status_and_logout_are_typed_machine_events() {
        let outputs = vec![
            event(json!({"methods":["oauth-browser","broker-owned"]})),
            event(json!({"status":"login-required"})),
            event(json!({"status":"browser-handoff","url":"https://login.example/device"})),
            event(json!({"status":"authenticated","principal_label":"paid-seat"})),
            event(json!({"logged_out":true})),
        ];
        let (cx, broker) = controller(outputs);
        assert_eq!(
            broker.auth_methods(&cx).unwrap(),
            vec![AuthMethod::OauthBrowser, AuthMethod::BrokerOwned]
        );
        assert_eq!(broker.status(&cx).unwrap(), SessionStatus::LoginRequired);
        assert_eq!(
            broker.login(&cx, AuthMethod::OauthBrowser).unwrap(),
            SessionStatus::BrowserHandoff {
                url: "https://login.example/device".into()
            }
        );
        assert_eq!(
            broker.status(&cx).unwrap(),
            SessionStatus::Authenticated {
                principal_label: Some("paid-seat".into())
            }
        );
        broker.logout(&cx).unwrap();
    }
}
