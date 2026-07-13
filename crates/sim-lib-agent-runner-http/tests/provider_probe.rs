use sim_kernel::{Error, Result, Symbol};
use sim_lib_agent_runner_http::{
    ProbeHttpRequest, ProbeHttpResponse, ProbeStatus, ProbeTransport, ProviderAuth, ProviderConfig,
    ProviderProfile, probe_provider, provider_profiles,
};
use sim_lib_net_core::HttpHead;
use std::{sync::Mutex, time::Duration};

const OPENAI_STYLE_MODELS: &str = r#"{"data":[{"id":"model-a"},{"id":"model-b"}]}"#;
const OLLAMA_MODELS: &str = r#"{"models":[{"name":"llama3:8b"},{"name":"qwen3:4b"}]}"#;
const MISSING_SECRET_ENV: &str = "SIM_AGENT_NET_PROVIDER_PROBE_MISSING_SECRET";

#[test]
fn provider_probe_discovers_models_for_every_provider() {
    let secret = std::env::var("PATH").expect("PATH must be set for auth probe tests");
    for profile in provider_profiles::all() {
        let provider = profile.provider.clone();
        let auth = profile.auth.clone();
        let models_path = profile.models_path;
        let body = model_body_for(&provider);
        let transport = MockTransport::ok(200, body);
        let config = config_for(profile, auth_env_for(&auth));

        let report = probe_provider(&transport, &config).unwrap();

        assert_eq!(report.status, ProbeStatus::Available, "{provider}");
        assert_eq!(report.models, expected_models_for(&provider), "{provider}");
        assert_eq!(report.redacted, auth != ProviderAuth::None, "{provider}");
        assert!(report.reason.is_none(), "{provider}: {:?}", report.reason);
        assert!(
            !format!("{report:?}").contains(&secret),
            "{provider} report leaked auth secret"
        );

        let requests = transport.requests();
        assert_eq!(requests.len(), 1, "{provider}");
        let request = &requests[0];
        assert_eq!(request.endpoint, config.endpoint, "{provider}");
        assert!(!request.scheme.is_empty(), "{provider}");
        assert_eq!(request.path, models_path, "{provider}");
        assert_eq!(request.timeout, Duration::from_secs(1), "{provider}");
        assert_eq!(request.max_response_bytes, 4096, "{provider}");
        match auth {
            ProviderAuth::None => assert!(
                request.headers.is_empty() || only_anthropic_version(&request.headers),
                "{provider}"
            ),
            ProviderAuth::BearerEnv { .. } => assert!(
                request
                    .headers
                    .iter()
                    .any(|(name, value)| name == "Authorization"
                        && value == &format!("Bearer {secret}")),
                "{provider}"
            ),
            ProviderAuth::HeaderEnv { ref header, .. } => assert!(
                request
                    .headers
                    .iter()
                    .any(|(name, value)| name == header && value == &secret),
                "{provider}"
            ),
        }
    }
}

#[test]
fn provider_probe_reports_unavailable_status_for_every_provider() {
    for profile in provider_profiles::all() {
        let provider = profile.provider.clone();
        let auth = profile.auth.clone();
        let transport = MockTransport::ok(503, "service unavailable");
        let config = config_for(profile, auth_env_for(&auth));

        let report = probe_provider(&transport, &config).unwrap();

        assert_eq!(report.status, ProbeStatus::Unavailable, "{provider}");
        assert!(report.models.is_empty(), "{provider}");
        assert_eq!(
            report.reason.as_deref(),
            Some("provider probe returned HTTP 503"),
            "{provider}"
        );
    }
}

#[test]
fn provider_probe_reports_malformed_json_for_every_provider() {
    for profile in provider_profiles::all() {
        let provider = profile.provider.clone();
        let auth = profile.auth.clone();
        let transport = MockTransport::ok(200, "{not-json");
        let config = config_for(profile, auth_env_for(&auth));

        let report = probe_provider(&transport, &config).unwrap();

        assert_eq!(report.status, ProbeStatus::Unavailable, "{provider}");
        assert!(report.models.is_empty(), "{provider}");
        assert!(
            report
                .reason
                .as_deref()
                .is_some_and(|reason| reason.contains("malformed model list json")),
            "{provider}: {:?}",
            report.reason
        );
    }
}

#[test]
fn provider_probe_skips_secret_providers_when_env_is_missing() {
    for profile in provider_profiles::all()
        .into_iter()
        .filter(|profile| profile.auth != ProviderAuth::None)
    {
        let provider = profile.provider.clone();
        let transport = MockTransport::ok(200, model_body_for(&provider));
        let mut config = config_for(profile, Some(MISSING_SECRET_ENV));
        config.api_key_env = Some(MISSING_SECRET_ENV.to_owned());

        let report = probe_provider(&transport, &config).unwrap();

        assert_eq!(report.status, ProbeStatus::Skipped, "{provider}");
        assert!(report.models.is_empty(), "{provider}");
        assert!(report.redacted, "{provider}");
        assert_eq!(
            report.reason.as_deref(),
            Some("missing secret env SIM_AGENT_NET_PROVIDER_PROBE_MISSING_SECRET"),
            "{provider}"
        );
        assert!(transport.requests().is_empty(), "{provider}");
    }
}

#[test]
fn provider_probe_redacts_transport_errors() {
    let secret = std::env::var("PATH").expect("PATH must be set for auth probe tests");
    let transport = MockTransport::err(format!("socket failed with {secret}"));
    let config = config_for(provider_profiles::openai(), Some("PATH"));

    let report = probe_provider(&transport, &config).unwrap();

    assert_eq!(report.status, ProbeStatus::Unavailable);
    let reason = report.reason.unwrap();
    assert!(!reason.contains(&secret));
    assert!(reason.contains("[REDACTED]"));
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct SeenRequest {
    endpoint: String,
    scheme: String,
    path: String,
    headers: Vec<(String, String)>,
    timeout: Duration,
    max_response_bytes: usize,
}

struct MockTransport {
    status: u16,
    body: String,
    error: Option<String>,
    requests: Mutex<Vec<SeenRequest>>,
}

impl MockTransport {
    fn ok(status: u16, body: &str) -> Self {
        Self {
            status,
            body: body.to_owned(),
            error: None,
            requests: Mutex::new(Vec::new()),
        }
    }

    fn err(message: String) -> Self {
        Self {
            status: 0,
            body: String::new(),
            error: Some(message),
            requests: Mutex::new(Vec::new()),
        }
    }

    fn requests(&self) -> Vec<SeenRequest> {
        self.requests.lock().unwrap().clone()
    }
}

impl ProbeTransport for MockTransport {
    fn get(&self, request: ProbeHttpRequest<'_>) -> Result<ProbeHttpResponse> {
        self.requests.lock().unwrap().push(SeenRequest {
            endpoint: request.endpoint.to_owned(),
            scheme: request.endpoint_parts.scheme,
            path: request.path.to_owned(),
            headers: request.headers,
            timeout: request.timeout,
            max_response_bytes: request.max_response_bytes,
        });
        if let Some(error) = &self.error {
            return Err(Error::HostError(error.clone()));
        }
        Ok(ProbeHttpResponse {
            head: HttpHead {
                status: self.status,
                reason: if self.status == 200 {
                    "OK".to_owned()
                } else {
                    "Service Unavailable".to_owned()
                },
                headers: vec![("Content-Type".to_owned(), "application/json".to_owned())],
            },
            body: self.body.as_bytes().to_vec(),
        })
    }
}

fn config_for(profile: ProviderProfile, api_key_env: Option<&str>) -> ProviderConfig {
    let runner = profile.runner_symbol.clone();
    let codec = profile.codec.clone();
    let endpoint = if profile.default_endpoint.is_empty() {
        "https://models.example/v1".to_owned()
    } else {
        profile.default_endpoint.clone()
    };
    let model = profile.default_model.clone();
    let locality = profile.default_locality.clone();
    let stream = profile.default_stream;
    let tools = profile.default_tools;
    ProviderConfig {
        profile,
        runner,
        codec,
        endpoint,
        model,
        api_key_env: api_key_env.map(str::to_owned),
        locality,
        timeout: Duration::from_secs(1),
        stream,
        tools,
        max_output_bytes: 4096,
    }
}

fn auth_env_for(auth: &ProviderAuth) -> Option<&'static str> {
    match auth {
        ProviderAuth::None => None,
        ProviderAuth::BearerEnv { .. } | ProviderAuth::HeaderEnv { .. } => Some("PATH"),
    }
}

fn model_body_for(provider: &Symbol) -> &'static str {
    if provider == &Symbol::new("ollama") {
        OLLAMA_MODELS
    } else {
        OPENAI_STYLE_MODELS
    }
}

fn expected_models_for(provider: &Symbol) -> Vec<String> {
    if provider == &Symbol::new("ollama") {
        vec!["llama3:8b".to_owned(), "qwen3:4b".to_owned()]
    } else {
        vec!["model-a".to_owned(), "model-b".to_owned()]
    }
}

fn only_anthropic_version(headers: &[(String, String)]) -> bool {
    headers
        .iter()
        .all(|(name, value)| name == "anthropic-version" && value == "2023-06-01")
}
