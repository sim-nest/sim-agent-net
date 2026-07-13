//! Open HTTP-provider profile records.
//!
//! Profiles are ordinary data owned by the HTTP runner crate. Provider identity
//! stays outside the kernel so new providers can be described without adding a
//! closed enum to `sim-kernel`.

use sim_kernel::Symbol;
use std::time::Duration;

/// Maximum decoded provider response size used by default.
pub const DEFAULT_MAX_OUTPUT_BYTES: usize = 1024 * 1024;

/// Authentication shape used by a provider profile.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProviderAuth {
    /// The provider does not need authentication by default.
    None,
    /// The provider expects an `Authorization: Bearer <secret>` value read from
    /// an environment variable.
    BearerEnv {
        /// Default environment variable name.
        env: String,
    },
    /// The provider may accept an `Authorization: Bearer <secret>` value, but
    /// does not require one by default.
    OptionalBearerEnv {
        /// Suggested environment variable name.
        env: String,
    },
    /// The provider expects a named header value read from an environment
    /// variable.
    HeaderEnv {
        /// Header name to send.
        header: String,
        /// Default environment variable name.
        env: String,
    },
}

impl ProviderAuth {
    /// Returns the default environment variable named by this auth profile.
    pub fn default_env(&self) -> Option<&str> {
        match self {
            Self::None => None,
            Self::BearerEnv { env } | Self::HeaderEnv { env, .. } => Some(env),
            Self::OptionalBearerEnv { .. } => None,
        }
    }

    /// Returns the documented environment variable for optional auth shapes.
    pub fn env_hint(&self) -> Option<&str> {
        match self {
            Self::None => None,
            Self::BearerEnv { env }
            | Self::OptionalBearerEnv { env }
            | Self::HeaderEnv { env, .. } => Some(env),
        }
    }
}

/// Data profile for a concrete HTTP model provider.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderProfile {
    /// Open provider id such as `openai`, `anthropic`, or `ollama`.
    pub provider: Symbol,
    /// Default runner symbol exposed by the agent library.
    pub runner_symbol: Symbol,
    /// Runtime codec symbol used to encode/decode provider payloads.
    pub codec: Symbol,
    /// Default base endpoint. An empty value means the caller must supply one.
    pub default_endpoint: String,
    /// Provider path that lists models relative to the base endpoint.
    pub models_path: &'static str,
    /// Provider path that performs chat inference relative to the base endpoint.
    pub chat_path: &'static str,
    /// Provider authentication profile.
    pub auth: ProviderAuth,
    /// Default runner locality, either `local` or `network`.
    pub default_locality: Symbol,
    /// Default model name used when the option map does not provide `model`.
    pub default_model: String,
    /// Default HTTP timeout.
    pub default_timeout: Duration,
    /// Default streaming flag.
    pub default_stream: bool,
    /// Default tool-use flag.
    pub default_tools: bool,
    /// Default response byte limit.
    pub default_max_output_bytes: usize,
}

impl ProviderProfile {
    /// Returns the native OpenAI profile.
    pub fn openai() -> Self {
        provider_profiles::openai()
    }

    /// Returns the native Anthropic profile.
    pub fn anthropic() -> Self {
        provider_profiles::anthropic()
    }

    /// Returns the native Ollama profile.
    pub fn ollama() -> Self {
        provider_profiles::ollama()
    }

    /// Returns the native LM Studio profile.
    pub fn lm_studio() -> Self {
        provider_profiles::lm_studio()
    }

    /// Returns the native Lemonade profile.
    pub fn lemonade() -> Self {
        provider_profiles::lemonade()
    }

    /// Returns the generic OpenAI-compatible fallback profile.
    pub fn openai_compatible() -> Self {
        provider_profiles::openai_compatible()
    }
}

/// Provider profile constructors.
pub mod provider_profiles {
    use super::{DEFAULT_MAX_OUTPUT_BYTES, ProviderAuth, ProviderProfile};
    use sim_kernel::Symbol;
    use std::time::Duration;

    /// Returns every built-in provider profile.
    pub fn all() -> Vec<ProviderProfile> {
        vec![
            openai(),
            anthropic(),
            ollama(),
            lm_studio(),
            lemonade(),
            openai_compatible(),
        ]
    }

    /// Native OpenAI provider profile.
    pub fn openai() -> ProviderProfile {
        ProviderProfile {
            provider: Symbol::new("openai"),
            runner_symbol: Symbol::qualified("runner", "openai"),
            codec: Symbol::qualified("codec", "openai"),
            default_endpoint: "https://api.openai.com/v1".to_owned(),
            models_path: "/models",
            chat_path: "/chat/completions",
            auth: ProviderAuth::BearerEnv {
                env: "OPENAI_API_KEY".to_owned(),
            },
            default_locality: Symbol::new("network"),
            default_model: "gpt-5-mini".to_owned(),
            default_timeout: Duration::from_secs(60),
            default_stream: false,
            default_tools: true,
            default_max_output_bytes: DEFAULT_MAX_OUTPUT_BYTES,
        }
    }

    /// Native Anthropic provider profile.
    pub fn anthropic() -> ProviderProfile {
        ProviderProfile {
            provider: Symbol::new("anthropic"),
            runner_symbol: Symbol::qualified("runner", "anthropic"),
            codec: Symbol::qualified("codec", "anthropic"),
            default_endpoint: "https://api.anthropic.com/v1".to_owned(),
            models_path: "/models",
            chat_path: "/messages",
            auth: ProviderAuth::HeaderEnv {
                header: "x-api-key".to_owned(),
                env: "ANTHROPIC_API_KEY".to_owned(),
            },
            default_locality: Symbol::new("network"),
            default_model: "claude-sonnet-latest".to_owned(),
            default_timeout: Duration::from_secs(60),
            default_stream: false,
            default_tools: true,
            default_max_output_bytes: DEFAULT_MAX_OUTPUT_BYTES,
        }
    }

    /// Native Ollama provider profile.
    pub fn ollama() -> ProviderProfile {
        ProviderProfile {
            provider: Symbol::new("ollama"),
            runner_symbol: Symbol::qualified("runner", "ollama"),
            codec: Symbol::qualified("codec", "ollama"),
            default_endpoint: "http://127.0.0.1:11434".to_owned(),
            models_path: "/api/tags",
            chat_path: "/api/chat",
            auth: ProviderAuth::None,
            default_locality: Symbol::new("local"),
            default_model: "qwen3.5:4b".to_owned(),
            default_timeout: Duration::from_secs(120),
            default_stream: true,
            default_tools: false,
            default_max_output_bytes: DEFAULT_MAX_OUTPUT_BYTES,
        }
    }

    /// Native LM Studio provider profile.
    pub fn lm_studio() -> ProviderProfile {
        ProviderProfile {
            provider: Symbol::new("lm-studio"),
            runner_symbol: Symbol::qualified("runner", "lm-studio"),
            codec: Symbol::qualified("codec", "lm-studio"),
            default_endpoint: "http://127.0.0.1:1234/v1".to_owned(),
            models_path: "/models",
            chat_path: "/chat/completions",
            auth: ProviderAuth::OptionalBearerEnv {
                env: "LM_STUDIO_API_KEY".to_owned(),
            },
            default_locality: Symbol::new("local"),
            default_model: "local/default".to_owned(),
            default_timeout: Duration::from_secs(120),
            default_stream: false,
            default_tools: true,
            default_max_output_bytes: DEFAULT_MAX_OUTPUT_BYTES,
        }
    }

    /// Native Lemonade provider profile.
    pub fn lemonade() -> ProviderProfile {
        ProviderProfile {
            provider: Symbol::new("lemonade"),
            runner_symbol: Symbol::qualified("runner", "lemonade"),
            codec: Symbol::qualified("codec", "lemonade"),
            default_endpoint: "http://127.0.0.1:13305/v1".to_owned(),
            models_path: "/models",
            chat_path: "/chat/completions",
            auth: ProviderAuth::None,
            default_locality: Symbol::new("local"),
            default_model: "Qwen3-Coder".to_owned(),
            default_timeout: Duration::from_secs(120),
            default_stream: false,
            default_tools: true,
            default_max_output_bytes: DEFAULT_MAX_OUTPUT_BYTES,
        }
    }

    /// Generic OpenAI-compatible fallback profile.
    pub fn openai_compatible() -> ProviderProfile {
        ProviderProfile {
            provider: Symbol::new("openai-compatible"),
            runner_symbol: Symbol::qualified("runner", "openai-compatible"),
            codec: Symbol::qualified("codec", "openai"),
            default_endpoint: String::new(),
            models_path: "/models",
            chat_path: "/chat/completions",
            auth: ProviderAuth::BearerEnv {
                env: "OPENAI_API_KEY".to_owned(),
            },
            default_locality: Symbol::new("network"),
            default_model: "gpt-4.1-mini".to_owned(),
            default_timeout: Duration::from_secs(60),
            default_stream: false,
            default_tools: true,
            default_max_output_bytes: DEFAULT_MAX_OUTPUT_BYTES,
        }
    }
}

/// Native OpenAI provider profile.
pub fn openai_profile() -> ProviderProfile {
    provider_profiles::openai()
}

/// Native Anthropic provider profile.
pub fn anthropic_profile() -> ProviderProfile {
    provider_profiles::anthropic()
}

/// Native Ollama provider profile.
pub fn ollama_profile() -> ProviderProfile {
    provider_profiles::ollama()
}

/// Native LM Studio provider profile.
pub fn lm_studio_profile() -> ProviderProfile {
    provider_profiles::lm_studio()
}

/// Native Lemonade provider profile.
pub fn lemonade_profile() -> ProviderProfile {
    provider_profiles::lemonade()
}

/// Generic OpenAI-compatible fallback provider profile.
pub fn openai_compatible_profile() -> ProviderProfile {
    provider_profiles::openai_compatible()
}

#[cfg(test)]
mod tests {
    use super::{ProviderAuth, ProviderProfile, provider_profiles};
    use sim_kernel::Symbol;

    #[test]
    fn profiles_cover_known_providers_and_generic_fallback() {
        let providers = provider_profiles::all()
            .into_iter()
            .map(|profile| profile.provider.to_string())
            .collect::<Vec<_>>();
        assert_eq!(
            providers,
            vec![
                "openai",
                "anthropic",
                "ollama",
                "lm-studio",
                "lemonade",
                "openai-compatible"
            ]
        );
    }

    #[test]
    fn anthropic_profile_records_header_auth_and_messages_path() {
        let profile = ProviderProfile::anthropic();
        assert_eq!(profile.provider, Symbol::new("anthropic"));
        assert_eq!(
            profile.runner_symbol,
            Symbol::qualified("runner", "anthropic")
        );
        assert_eq!(profile.codec, Symbol::qualified("codec", "anthropic"));
        assert_eq!(profile.default_endpoint, "https://api.anthropic.com/v1");
        assert_eq!(profile.models_path, "/models");
        assert_eq!(profile.chat_path, "/messages");
        assert_eq!(
            profile.auth,
            ProviderAuth::HeaderEnv {
                header: "x-api-key".to_owned(),
                env: "ANTHROPIC_API_KEY".to_owned()
            }
        );
        assert_eq!(profile.default_locality, Symbol::new("network"));
    }

    #[test]
    fn local_profiles_default_to_loopback_without_auth() {
        for profile in [ProviderProfile::ollama(), ProviderProfile::lemonade()] {
            assert_eq!(profile.auth, ProviderAuth::None);
            assert_eq!(profile.default_locality, Symbol::new("local"));
            assert!(profile.default_endpoint.starts_with("http://127.0.0.1:"));
        }
    }

    #[test]
    fn lm_studio_profile_records_optional_bearer_auth() {
        let profile = ProviderProfile::lm_studio();

        assert_eq!(
            profile.auth,
            ProviderAuth::OptionalBearerEnv {
                env: "LM_STUDIO_API_KEY".to_owned()
            }
        );
        assert_eq!(profile.auth.default_env(), None);
        assert_eq!(profile.auth.env_hint(), Some("LM_STUDIO_API_KEY"));
        assert_eq!(profile.default_locality, Symbol::new("local"));
        assert!(profile.default_endpoint.starts_with("http://127.0.0.1:"));
    }
}
