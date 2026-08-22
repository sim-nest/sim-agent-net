use sim_kernel::{Error, Expr, Result, Symbol};

/// Authentication method advertised by a provider family.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum AuthMethod {
    /// A SIM-owned API key resolved through a secret provider.
    ApiKey,
    /// OAuth completed by handing a browser URL to the caller.
    OauthBrowser,
    /// OAuth completed with a device code and verification URL.
    OauthDevice,
    /// Authentication inherited from a paid subscription.
    Subscription,
    /// Authentication state owned entirely by the broker.
    BrokerOwned,
    /// The provider requires no authentication.
    None,
}

impl AuthMethod {
    /// Returns the stable provider vocabulary symbol.
    pub fn symbol(self) -> Symbol {
        Symbol::new(match self {
            Self::ApiKey => "api-key",
            Self::OauthBrowser => "oauth-browser",
            Self::OauthDevice => "oauth-device",
            Self::Subscription => "subscription",
            Self::BrokerOwned => "broker-owned",
            Self::None => "none",
        })
    }

    /// Parses one stable provider vocabulary symbol, rejecting extensions by default.
    pub fn from_symbol(symbol: &Symbol) -> Result<Self> {
        match symbol.to_string().as_str() {
            "api-key" => Ok(Self::ApiKey),
            "oauth-browser" => Ok(Self::OauthBrowser),
            "oauth-device" => Ok(Self::OauthDevice),
            "subscription" => Ok(Self::Subscription),
            "broker-owned" => Ok(Self::BrokerOwned),
            "none" => Ok(Self::None),
            other => Err(Error::Eval(format!("unknown provider auth method {other}"))),
        }
    }
}

/// Redaction-safe owner of provider authentication state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AuthOwner {
    /// SIM owns authentication through an opaque secret-provider reference.
    Sim,
    /// The external broker owns its authentication artifacts.
    Broker,
    /// No authentication state exists.
    None,
}

/// Current redaction-safe authentication session state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SessionStatus {
    /// No login has been established.
    LoggedOut,
    /// A login flow must be completed by the caller.
    LoginRequired,
    /// A browser handoff is pending at this non-secret URL.
    BrowserHandoff {
        /// Public handoff URL; never a cookie, token, or browser profile.
        url: String,
    },
    /// A device flow is pending at the URL with its public user code.
    DeviceHandoff {
        /// Public verification URL.
        url: String,
        /// Public device-flow user code.
        user_code: String,
    },
    /// The seat has an authenticated broker session.
    Authenticated {
        /// Optional redaction-safe principal label.
        principal_label: Option<String>,
    },
}

/// Versioned acknowledgement of provider terms, containing no browser artifacts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TermsAcknowledgement {
    /// Stable terms identifier supplied by the provider family.
    pub terms_id: String,
    /// Exact revision acknowledged by the operator.
    pub revision: String,
    /// Redaction-safe actor label or automation identity.
    pub acknowledged_by: String,
}

/// Authentication and terms metadata attached to a family or seat card.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthMetadata {
    /// Owner of authentication artifacts.
    pub owner: AuthOwner,
    /// Current session state.
    pub session: SessionStatus,
    /// Terms revision required before login or inference, if any.
    pub required_terms: Option<(String, String)>,
    /// Recorded terms acknowledgement, if any.
    pub acknowledgement: Option<TermsAcknowledgement>,
}

impl AuthMetadata {
    /// Refuses use when the exact required terms revision has not been acknowledged.
    pub fn require_terms(&self) -> Result<()> {
        let Some((terms_id, revision)) = &self.required_terms else {
            return Ok(());
        };
        let accepted = self
            .acknowledgement
            .as_ref()
            .is_some_and(|ack| ack.terms_id == *terms_id && ack.revision == *revision);
        if accepted {
            Ok(())
        } else {
            Err(Error::Eval(format!(
                "provider terms {terms_id} revision {revision} must be acknowledged"
            )))
        }
    }

    /// Encodes the redaction-safe record into open card metadata.
    pub fn to_expr(&self) -> Expr {
        let owner = match self.owner {
            AuthOwner::Sim => "sim",
            AuthOwner::Broker => "broker",
            AuthOwner::None => "none",
        };
        let session = match &self.session {
            SessionStatus::LoggedOut => vec![Expr::Symbol(Symbol::new("logged-out"))],
            SessionStatus::LoginRequired => vec![Expr::Symbol(Symbol::new("login-required"))],
            SessionStatus::BrowserHandoff { url } => vec![
                Expr::Symbol(Symbol::new("browser-handoff")),
                Expr::String(url.clone()),
            ],
            SessionStatus::DeviceHandoff { url, user_code } => vec![
                Expr::Symbol(Symbol::new("device-handoff")),
                Expr::String(url.clone()),
                Expr::String(user_code.clone()),
            ],
            SessionStatus::Authenticated { principal_label } => vec![
                Expr::Symbol(Symbol::new("authenticated")),
                Expr::String(principal_label.clone().unwrap_or_default()),
            ],
        };
        let required = self
            .required_terms
            .as_ref()
            .map_or(Expr::Nil, |(id, revision)| {
                Expr::List(vec![
                    Expr::String(id.clone()),
                    Expr::String(revision.clone()),
                ])
            });
        let acknowledgement = self.acknowledgement.as_ref().map_or(Expr::Nil, |ack| {
            Expr::List(vec![
                Expr::String(ack.terms_id.clone()),
                Expr::String(ack.revision.clone()),
                Expr::String(ack.acknowledged_by.clone()),
            ])
        });
        Expr::List(vec![
            Expr::Symbol(Symbol::new(owner)),
            Expr::List(session),
            required,
            acknowledgement,
        ])
    }

    /// Decodes card metadata, rejecting malformed or unknown values.
    pub fn from_expr(expr: &Expr) -> Result<Self> {
        let Expr::List(fields) = expr else {
            return Err(Error::Eval("provider auth metadata must be a list".into()));
        };
        let [
            Expr::Symbol(owner),
            Expr::List(session),
            required,
            acknowledgement,
        ] = fields.as_slice()
        else {
            return Err(Error::Eval(
                "provider auth metadata has the wrong shape".into(),
            ));
        };
        let owner = match owner.to_string().as_str() {
            "sim" => AuthOwner::Sim,
            "broker" => AuthOwner::Broker,
            "none" => AuthOwner::None,
            other => return Err(Error::Eval(format!("unknown provider auth owner {other}"))),
        };
        let session = match session.as_slice() {
            [Expr::Symbol(kind)] if kind.to_string() == "logged-out" => SessionStatus::LoggedOut,
            [Expr::Symbol(kind)] if kind.to_string() == "login-required" => {
                SessionStatus::LoginRequired
            }
            [Expr::Symbol(kind), Expr::String(url)] if kind.to_string() == "browser-handoff" => {
                SessionStatus::BrowserHandoff { url: url.clone() }
            }
            [
                Expr::Symbol(kind),
                Expr::String(url),
                Expr::String(user_code),
            ] if kind.to_string() == "device-handoff" => SessionStatus::DeviceHandoff {
                url: url.clone(),
                user_code: user_code.clone(),
            },
            [Expr::Symbol(kind), Expr::String(label)] if kind.to_string() == "authenticated" => {
                SessionStatus::Authenticated {
                    principal_label: (!label.is_empty()).then(|| label.clone()),
                }
            }
            _ => {
                return Err(Error::Eval(
                    "provider session metadata has the wrong shape".into(),
                ));
            }
        };
        let required_terms = decode_pair(required, "required terms")?;
        let acknowledgement = match acknowledgement {
            Expr::Nil => None,
            Expr::List(values) => match values.as_slice() {
                [
                    Expr::String(terms_id),
                    Expr::String(revision),
                    Expr::String(acknowledged_by),
                ] => Some(TermsAcknowledgement {
                    terms_id: terms_id.clone(),
                    revision: revision.clone(),
                    acknowledged_by: acknowledged_by.clone(),
                }),
                _ => {
                    return Err(Error::Eval(
                        "terms acknowledgement has the wrong shape".into(),
                    ));
                }
            },
            _ => {
                return Err(Error::Eval(
                    "terms acknowledgement has the wrong shape".into(),
                ));
            }
        };
        Ok(Self {
            owner,
            session,
            required_terms,
            acknowledgement,
        })
    }
}

fn decode_pair(expr: &Expr, label: &str) -> Result<Option<(String, String)>> {
    match expr {
        Expr::Nil => Ok(None),
        Expr::List(values) => match values.as_slice() {
            [Expr::String(left), Expr::String(right)] => Ok(Some((left.clone(), right.clone()))),
            _ => Err(Error::Eval(format!("{label} has the wrong shape"))),
        },
        _ => Err(Error::Eval(format!("{label} has the wrong shape"))),
    }
}

/// Stable metadata key used on family and seat card extension fields.
pub fn auth_metadata_key() -> Expr {
    Expr::Symbol(Symbol::qualified("provider", "auth"))
}
