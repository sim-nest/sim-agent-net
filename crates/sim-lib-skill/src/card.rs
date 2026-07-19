use sim_citizen_derive::non_citizen;
use sim_kernel::{CapabilityName, Cx, Expr, Object, ObjectCompat, Result, ShapeRef, Symbol, Value};

/// Role a skill plays for an agent.
///
/// A skill may carry more than one role; the role set drives how the skill is
/// presented to tool, model, and resource surfaces.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SkillRole {
    /// Callable tool the agent can invoke.
    Tool,
    /// Language or inference model.
    Model,
    /// Readable resource exposed to the agent.
    Resource,
    /// Reusable prompt template.
    Prompt,
    /// Memory store the agent can read from or write to.
    Memory,
    /// Retriever that fetches relevant context.
    Retriever,
    /// Judge that scores or evaluates candidate outputs.
    Judge,
    /// Router that dispatches to other skills.
    Router,
}

impl SkillRole {
    /// Returns the canonical symbol naming this role.
    pub fn as_symbol(&self) -> Symbol {
        Symbol::new(match self {
            Self::Tool => "tool",
            Self::Model => "model",
            Self::Resource => "resource",
            Self::Prompt => "prompt",
            Self::Memory => "memory",
            Self::Retriever => "retriever",
            Self::Judge => "judge",
            Self::Router => "router",
        })
    }
}

/// How much of a skill's raw payload may leave the local boundary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SkillPrivacyPolicy {
    /// Only metadata may be exposed; raw inputs and outputs stay private.
    MetadataOnly,
    /// Raw payloads must not be recorded or forwarded.
    NoRaw,
    /// Raw payloads may be used locally but never leave the host.
    LocalOnly,
    /// Raw payloads may be exposed and forwarded.
    AllowRaw,
}

impl SkillPrivacyPolicy {
    /// Returns the canonical symbol naming this privacy policy.
    pub fn as_symbol(&self) -> Symbol {
        Symbol::new(match self {
            Self::MetadataOnly => "metadata-only",
            Self::NoRaw => "no-raw",
            Self::LocalOnly => "local-only",
            Self::AllowRaw => "allow-raw",
        })
    }
}

/// Caching behavior for a skill's results.
///
/// Caching only applies to skills marked idempotent (see
/// [`SkillPolicy::idempotent`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SkillCacheMode {
    /// No caching; every call reaches the transport.
    Disabled,
    /// Read from the cache on a hit, otherwise call and store the result.
    ReadThrough,
    /// Read from the cache but never store new results.
    ReadOnly,
    /// Store results but never serve from the cache.
    WriteOnly,
    /// Bypass cached results and refresh the stored entry from a live call.
    Refresh,
}

impl SkillCacheMode {
    /// Returns the canonical symbol naming this cache mode.
    pub fn as_symbol(&self) -> Symbol {
        Symbol::new(match self {
            Self::Disabled => "disabled",
            Self::ReadThrough => "read-through",
            Self::ReadOnly => "read-only",
            Self::WriteOnly => "write-only",
            Self::Refresh => "refresh",
        })
    }
}

/// Cassette (record/replay) behavior for a skill's calls.
///
/// Cassettes capture deterministic recordings of skill calls for replay in
/// tests and offline runs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SkillCassetteMode {
    /// No recording or replay.
    Disabled,
    /// Replay a recorded result on a hit, otherwise call and record it.
    RecordReplay,
    /// Replay only; a missing recording is an error.
    ReplayOnly,
    /// Record live calls without replaying existing recordings.
    RecordOnly,
}

impl SkillCassetteMode {
    /// Returns the canonical symbol naming this cassette mode.
    pub fn as_symbol(&self) -> Symbol {
        Symbol::new(match self {
            Self::Disabled => "disabled",
            Self::RecordReplay => "record-replay",
            Self::ReplayOnly => "replay-only",
            Self::RecordOnly => "record-only",
        })
    }
}

/// Privacy, caching, and recording policy attached to a [`SkillCard`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SkillPolicy {
    /// How much of the raw payload may leave the local boundary.
    pub privacy: SkillPrivacyPolicy,
    /// Caching behavior for results (effective only when idempotent).
    pub cache: SkillCacheMode,
    /// Cassette record/replay behavior for calls.
    pub cassette: SkillCassetteMode,
    /// Whether repeated calls with the same arguments yield the same result,
    /// which is what makes caching sound.
    pub idempotent: bool,
    /// Optional explicit key for deriving the cache/cassette identity for a call
    /// instead of the default argument-derived key.
    pub semantic_key: Option<String>,
}

impl Default for SkillPolicy {
    fn default() -> Self {
        Self {
            privacy: SkillPrivacyPolicy::NoRaw,
            cache: SkillCacheMode::Disabled,
            cassette: SkillCassetteMode::Disabled,
            idempotent: false,
            semantic_key: None,
        }
    }
}

/// Full runtime description of a single skill.
///
/// A card carries the skill's identity and symbol, its input and output shape
/// contracts, the roles it plays, the capabilities required to call it, its
/// [`SkillPolicy`], and the transport coordinates (id, kind, and operation)
/// that dispatch a call. It is a shape-bearing runtime handle; its
/// serializable projection is the [`SkillCardDescriptor`] (`skill/Card`).
///
/// [`SkillCardDescriptor`]: crate::SkillCardDescriptor
#[derive(Clone)]
#[non_citizen(
    reason = "shape-bearing runtime skill card; serializable projection is skill/Card descriptor",
    kind = "handle",
    descriptor = "skill/Card"
)]
pub struct SkillCard {
    /// Stable string identifier for the skill.
    pub id: String,
    /// Symbol the skill is registered and called under.
    pub symbol: Symbol,
    /// Additional symbols that also resolve to this skill.
    pub aliases: Vec<Symbol>,
    /// Symbol naming where the card came from (for example `fixture`).
    pub origin: Symbol,
    /// Human-readable title.
    pub title: String,
    /// Human-readable description of what the skill does.
    pub description: String,
    /// Shape contract the call arguments must satisfy.
    pub input_shape: ShapeRef,
    /// Shape contract the call result must satisfy.
    pub output_shape: ShapeRef,
    /// Roles this skill plays for an agent.
    pub roles: Vec<SkillRole>,
    /// Capabilities a caller must hold to invoke the skill.
    pub capabilities: Vec<CapabilityName>,
    /// Privacy, caching, and recording policy.
    pub policy: SkillPolicy,
    /// Identifier of the transport that runs the skill.
    pub transport_id: String,
    /// Kind of the transport (for example `fixture`, `mcp`, `http`).
    pub transport_kind: String,
    /// Operation name the transport dispatches for this skill.
    pub operation: String,
}

/// Inputs for building a fixture [`SkillCard`] via [`SkillCard::fixture`].
pub struct FixtureSkillSpec {
    /// Stable string identifier for the skill.
    pub id: String,
    /// Symbol the skill is registered and called under.
    pub symbol: Symbol,
    /// Human-readable title.
    pub title: String,
    /// Human-readable description.
    pub description: String,
    /// Shape contract for the call arguments.
    pub input_shape: ShapeRef,
    /// Shape contract for the call result.
    pub output_shape: ShapeRef,
    /// Identifier of the fixture transport that runs the skill.
    pub transport_id: String,
    /// Operation name the fixture transport dispatches.
    pub operation: String,
}

impl SkillCard {
    /// Builds a fixture skill card from `spec`.
    ///
    /// The card is given the `fixture` origin, the [`SkillRole::Tool`] role, a
    /// default [`SkillPolicy`], and a single skill-specific call capability.
    pub fn fixture(spec: FixtureSkillSpec) -> Self {
        let id = spec.id;
        Self {
            capabilities: vec![crate::skill_specific_call_capability(&id)],
            id,
            symbol: spec.symbol,
            aliases: Vec::new(),
            origin: Symbol::new("fixture"),
            title: spec.title,
            description: spec.description,
            input_shape: spec.input_shape,
            output_shape: spec.output_shape,
            roles: vec![SkillRole::Tool],
            policy: SkillPolicy::default(),
            transport_id: spec.transport_id,
            transport_kind: "fixture".to_owned(),
            operation: spec.operation,
        }
    }

    /// Returns the card with `capability` appended to its required capabilities.
    pub fn with_capability(mut self, capability: CapabilityName) -> Self {
        self.capabilities.push(capability);
        self
    }

    /// Returns the card with `role` added if it is not already present.
    pub fn with_role(mut self, role: SkillRole) -> Self {
        if !self.roles.contains(&role) {
            self.roles.push(role);
        }
        self
    }

    /// Returns the card with its policy replaced by `policy`.
    pub fn with_policy(mut self, policy: SkillPolicy) -> Self {
        self.policy = policy;
        self
    }

    /// Returns the card with its cache mode set to `cache`.
    pub fn with_cache_mode(mut self, cache: SkillCacheMode) -> Self {
        self.policy.cache = cache;
        self
    }

    /// Returns the card with its cassette mode set to `cassette`.
    pub fn with_cassette_mode(mut self, cassette: SkillCassetteMode) -> Self {
        self.policy.cassette = cassette;
        self
    }

    /// Returns the card with its idempotency flag set to `idempotent`.
    pub fn with_idempotent(mut self, idempotent: bool) -> Self {
        self.policy.idempotent = idempotent;
        self
    }

    /// Returns the card with its semantic key set to `semantic_key`.
    pub fn with_semantic_key(mut self, semantic_key: impl Into<String>) -> Self {
        self.policy.semantic_key = Some(semantic_key.into());
        self
    }

    /// Returns the card with its privacy policy set to `privacy`.
    pub fn with_privacy(mut self, privacy: SkillPrivacyPolicy) -> Self {
        self.policy.privacy = privacy;
        self
    }

    /// Wraps the card in an opaque runtime [`Value`].
    pub fn value(&self, cx: &mut Cx) -> Result<Value> {
        cx.factory().opaque(std::sync::Arc::new(self.clone()))
    }

    /// Projects the card into a table [`Value`] describing its fields.
    pub fn table_value(&self, cx: &mut Cx) -> Result<Value> {
        let aliases = cx.factory().list(
            self.aliases
                .iter()
                .map(|alias| cx.factory().symbol(alias.clone()))
                .collect::<Result<Vec<_>>>()?,
        )?;
        let roles = cx.factory().list(
            self.roles
                .iter()
                .map(|role| cx.factory().symbol(role.as_symbol()))
                .collect::<Result<Vec<_>>>()?,
        )?;
        let capabilities = cx.factory().list(
            self.capabilities
                .iter()
                .map(|capability| cx.factory().string(capability.as_str().to_owned()))
                .collect::<Result<Vec<_>>>()?,
        )?;
        let transport = cx.factory().table(vec![
            (
                Symbol::new("id"),
                cx.factory().string(self.transport_id.clone())?,
            ),
            (
                Symbol::new("kind"),
                cx.factory()
                    .symbol(Symbol::new(self.transport_kind.clone()))?,
            ),
            (
                Symbol::new("operation"),
                cx.factory().string(self.operation.clone())?,
            ),
        ])?;
        let mut policy = vec![
            (
                Symbol::new("privacy"),
                cx.factory().symbol(self.policy.privacy.as_symbol())?,
            ),
            (
                Symbol::new("cache"),
                cx.factory().symbol(self.policy.cache.as_symbol())?,
            ),
            (
                Symbol::new("cassette"),
                cx.factory().symbol(self.policy.cassette.as_symbol())?,
            ),
            (
                Symbol::new("idempotent"),
                cx.factory().bool(self.policy.idempotent)?,
            ),
        ];
        if let Some(semantic_key) = &self.policy.semantic_key {
            policy.push((
                Symbol::new("semantic-key"),
                cx.factory().string(semantic_key.clone())?,
            ));
        }
        let policy = cx.factory().table(policy)?;
        cx.factory().table(vec![
            (
                Symbol::new("kind"),
                cx.factory().symbol(Symbol::qualified("skill", "card"))?,
            ),
            (Symbol::new("id"), cx.factory().string(self.id.clone())?),
            (
                Symbol::new("symbol"),
                cx.factory().symbol(self.symbol.clone())?,
            ),
            (Symbol::new("aliases"), aliases),
            (
                Symbol::new("origin"),
                cx.factory().symbol(self.origin.clone())?,
            ),
            (
                Symbol::new("title"),
                cx.factory().string(self.title.clone())?,
            ),
            (
                Symbol::new("description"),
                cx.factory().string(self.description.clone())?,
            ),
            (Symbol::new("input-shape"), self.input_shape.clone()),
            (Symbol::new("output-shape"), self.output_shape.clone()),
            (Symbol::new("roles"), roles),
            (Symbol::new("capabilities"), capabilities),
            (Symbol::new("policy"), policy),
            (Symbol::new("transport"), transport),
        ])
    }
}

impl Object for SkillCard {
    fn display(&self, _cx: &mut Cx) -> Result<String> {
        Ok(format!("#<skill-card {}>", self.id))
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl ObjectCompat for SkillCard {
    fn as_expr(&self, cx: &mut Cx) -> Result<Expr> {
        self.to_expr(cx)
    }

    fn as_table(&self, cx: &mut Cx) -> Result<Value> {
        self.table_value(cx)
    }
}
