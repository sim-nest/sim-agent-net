use sim_kernel::{Error, Result, Symbol};
use sim_lib_agent_runner_http::{
    EndpointCandidate, ProbeHttpRequest, ProbeHttpResponse, ProbeStatus, ProbeTransport,
    ProviderAuth, ProviderConfig, ProviderProfile, lemonade_candidates, parse_ollama_tags,
    probe_provider, provider_profiles,
};
use sim_lib_net_core::HttpHead;
use std::{sync::Mutex, time::Duration};

const OPENAI_STYLE_MODELS: &str = r#"{"data":[{"id":"model-a"},{"id":"model-b"}]}"#;
const OLLAMA_MODELS: &str = r#"{"models":[{"name":"llama3:8b"},{"name":"qwen3:4b"}]}"#;
const LEMONADE_MODELS: &str = r#"{"data":[{"id":"lemonade-text","modalities":["text"],"input_modalities":["text","image"],"output_modalities":["text","audio"]}]}"#;
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
            ProviderAuth::BearerEnv { .. } | ProviderAuth::OptionalBearerEnv { .. } => assert!(
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
        let reason = report.reason.as_deref().unwrap_or_default();
        if provider == Symbol::new("ollama") {
            assert!(
                reason.contains("ollama tags invalid json"),
                "{provider}: {reason}"
            );
        } else {
            assert!(
                reason.contains("malformed model list json"),
                "{provider}: {reason}"
            );
        }
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

#[test]
fn parse_ollama_tags_extracts_model_names() {
    let models = parse_ollama_tags(OLLAMA_MODELS.as_bytes()).unwrap();

    assert_eq!(models, vec!["llama3:8b", "qwen3:4b"]);
}

#[test]
fn lemonade_candidates_try_legacy_and_api_bases_by_default() {
    assert_eq!(
        lemonade_candidates(None),
        vec![
            EndpointCandidate {
                endpoint: "http://127.0.0.1:13305/v1".to_owned(),
                models_path: "/models"
            },
            EndpointCandidate {
                endpoint: "http://127.0.0.1:13305/api/v1".to_owned(),
                models_path: "/models"
            },
        ]
    );
    assert_eq!(
        lemonade_candidates(Some("http://localhost:13305/custom".to_owned())),
        vec![EndpointCandidate {
            endpoint: "http://localhost:13305/custom".to_owned(),
            models_path: "/models"
        }]
    );
}

#[test]
fn lemonade_probe_keeps_first_healthy_candidate_and_model_modalities() {
    let transport = SequenceTransport::new(vec![
        MockResult::Response(ProbeHttpResponse {
            head: HttpHead {
                status: 404,
                reason: "Not Found".to_owned(),
                headers: Vec::new(),
            },
            body: b"not found".to_vec(),
        }),
        MockResult::Response(ProbeHttpResponse {
            head: HttpHead {
                status: 200,
                reason: "OK".to_owned(),
                headers: Vec::new(),
            },
            body: LEMONADE_MODELS.as_bytes().to_vec(),
        }),
    ]);
    let config = config_for(provider_profiles::lemonade(), None);

    let report = probe_provider(&transport, &config).unwrap();

    assert_eq!(report.status, ProbeStatus::Available);
    assert_eq!(report.endpoint, "http://127.0.0.1:13305/api/v1");
    assert_eq!(report.models, vec!["lemonade-text"]);
    assert_eq!(report.model_cards.len(), 1);
    let card = &report.model_cards[0];
    assert_eq!(card.runner, Symbol::qualified("runner", "lemonade"));
    assert_eq!(card.provider, Symbol::new("lemonade"));
    assert_eq!(card.locality, Symbol::new("local"));
    assert_eq!(symbol_list(card_field(card, "modalities")), vec!["text"]);
    assert_eq!(
        symbol_list(card_field(card, "modalities-in")),
        vec!["text", "image"]
    );
    assert_eq!(
        symbol_list(card_field(card, "modalities-out")),
        vec!["text", "audio"]
    );
    let requests = transport.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].endpoint, "http://127.0.0.1:13305/v1");
    assert_eq!(requests[1].endpoint, "http://127.0.0.1:13305/api/v1");
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

enum MockResult {
    Response(ProbeHttpResponse),
}

struct SequenceTransport {
    results: Mutex<Vec<MockResult>>,
    requests: Mutex<Vec<SeenRequest>>,
}

impl SequenceTransport {
    fn new(results: Vec<MockResult>) -> Self {
        Self {
            results: Mutex::new(results.into_iter().rev().collect()),
            requests: Mutex::new(Vec::new()),
        }
    }

    fn requests(&self) -> Vec<SeenRequest> {
        self.requests.lock().unwrap().clone()
    }
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

impl ProbeTransport for SequenceTransport {
    fn get(&self, request: ProbeHttpRequest<'_>) -> Result<ProbeHttpResponse> {
        self.requests.lock().unwrap().push(SeenRequest {
            endpoint: request.endpoint.to_owned(),
            scheme: request.endpoint_parts.scheme,
            path: request.path.to_owned(),
            headers: request.headers,
            timeout: request.timeout,
            max_response_bytes: request.max_response_bytes,
        });
        match self.results.lock().unwrap().pop() {
            Some(MockResult::Response(response)) => Ok(response),
            None => panic!("unexpected extra probe request"),
        }
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
        ProviderAuth::BearerEnv { .. }
        | ProviderAuth::OptionalBearerEnv { .. }
        | ProviderAuth::HeaderEnv { .. } => Some("PATH"),
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

fn card_field<'a>(
    card: &'a sim_lib_agent_runner_core::ModelCard,
    name: &str,
) -> &'a sim_kernel::Expr {
    card.extra
        .iter()
        .find_map(|(key, value)| match key {
            sim_kernel::Expr::Symbol(symbol) if symbol.name.as_ref() == name => Some(value),
            _ => None,
        })
        .unwrap_or_else(|| panic!("missing card field {name}"))
}

fn symbol_list(expr: &sim_kernel::Expr) -> Vec<&str> {
    match expr {
        sim_kernel::Expr::List(items) => items
            .iter()
            .map(|item| match item {
                sim_kernel::Expr::Symbol(symbol) => symbol.name.as_ref(),
                other => panic!("expected symbol, found {other:?}"),
            })
            .collect(),
        other => panic!("expected symbol list, found {other:?}"),
    }
}
