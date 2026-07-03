use std::sync::Arc;

use sim_kernel::{
    CapabilityName, CapabilitySet, Cx, DefaultFactory, Env, Error, Expr, Factory, Registry, Result,
    Symbol, Value,
};

use crate::{EvalSite, ServerAddress, ServerFrame};

#[derive(Clone, Debug, PartialEq, Eq)]
/// How one isolation axis (env, libs, factory, registry, capabilities) is
/// derived for an evaluation.
pub enum ShareMode {
    /// Reuse the caller's value for this axis directly.
    Share,
    /// Derive a child scoped to the caller's value where applicable.
    Child,
    /// Start fresh with a default, sharing nothing from the caller.
    Isolate,
    /// Reconstruct the axis from a named registry snapshot symbol.
    Import(Symbol),
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// Per-axis [`ShareMode`] selection controlling how an evaluation is isolated
/// from its caller.
pub struct IsolationPolicy {
    /// Share mode for the lexical environment.
    pub env: ShareMode,
    /// Share mode for the loaded library set.
    pub libs: ShareMode,
    /// Share mode for the object factory.
    pub factory: ShareMode,
    /// Share mode for the registry.
    pub registry: ShareMode,
    /// Share mode for the capability set.
    pub capabilities: ShareMode,
}

impl Default for IsolationPolicy {
    fn default() -> Self {
        Self {
            env: ShareMode::Share,
            libs: ShareMode::Share,
            factory: ShareMode::Share,
            registry: ShareMode::Share,
            capabilities: ShareMode::Share,
        }
    }
}

impl IsolationPolicy {
    /// Parses an isolation policy from a key/value-pair expression.
    ///
    /// Nil or the `all-shared` symbol yields the default (all axes shared);
    /// otherwise a list or vector of `:axis mode` pairs sets each named axis.
    pub fn from_expr(expr: &Expr) -> Result<Self> {
        match expr {
            Expr::Nil => Ok(Self::default()),
            Expr::Symbol(symbol) if symbol.name.as_ref() == "all-shared" => Ok(Self::default()),
            Expr::List(items) | Expr::Vector(items) => {
                let mut policy = Self::default();
                if !items.len().is_multiple_of(2) {
                    return Err(Error::Eval(
                        "isolation policy must use key/value pairs".to_owned(),
                    ));
                }
                for pair in items.chunks(2) {
                    let Expr::Symbol(symbol) = &pair[0] else {
                        return Err(Error::TypeMismatch {
                            expected: "keyword symbol",
                            found: "non-symbol",
                        });
                    };
                    let key = symbol
                        .name
                        .strip_prefix(':')
                        .unwrap_or(symbol.name.as_ref());
                    let mode = parse_share_mode(&pair[1])?;
                    match key {
                        "env" => policy.env = mode,
                        "libs" => policy.libs = mode,
                        "factory" => policy.factory = mode,
                        "registry" => policy.registry = mode,
                        "capabilities" => policy.capabilities = mode,
                        other => {
                            return Err(Error::Eval(format!(
                                "unknown isolation policy axis :{other}"
                            )));
                        }
                    }
                }
                Ok(policy)
            }
            _ => Err(Error::TypeMismatch {
                expected: "isolation policy expression",
                found: "non-policy",
            }),
        }
    }

    /// Reflects the policy as a table keyed by axis name.
    pub fn as_value(&self, cx: &mut Cx) -> Result<Value> {
        let env = share_mode_value(cx, &self.env)?;
        let libs = share_mode_value(cx, &self.libs)?;
        let factory = share_mode_value(cx, &self.factory)?;
        let registry = share_mode_value(cx, &self.registry)?;
        let capabilities = share_mode_value(cx, &self.capabilities)?;
        cx.factory().table(vec![
            (Symbol::new("env"), env),
            (Symbol::new("libs"), libs),
            (Symbol::new("factory"), factory),
            (Symbol::new("registry"), registry),
            (Symbol::new("capabilities"), capabilities),
        ])
    }

    /// Runs `f` under a context derived per this policy's axes.
    ///
    /// Derives the env, capabilities, factory, and registry according to each
    /// axis and installs them for the duration of the call.
    pub fn apply<T>(&self, cx: &mut Cx, f: impl FnOnce(&mut Cx) -> Result<T>) -> Result<T> {
        let env = derive_env(cx, &self.env)?;
        let capabilities = derive_capabilities(cx, &self.capabilities)?;
        let factory = derive_factory(cx, &self.factory);
        let registry = derive_registry(cx, &self.registry, &self.libs)?;
        cx.with_registry(registry, |cx| {
            cx.with_factory(factory, |cx| {
                cx.with_capabilities(capabilities, |cx| cx.with_env(env, f))
            })
        })
    }
}

fn parse_share_mode(expr: &Expr) -> Result<ShareMode> {
    match expr {
        Expr::Symbol(symbol) => match symbol.name.as_ref() {
            "share" => Ok(ShareMode::Share),
            "child" => Ok(ShareMode::Child),
            "isolate" => Ok(ShareMode::Isolate),
            other => Err(Error::Eval(format!("unsupported share mode {other}"))),
        },
        Expr::List(items) | Expr::Vector(items) => match items.as_slice() {
            [Expr::Symbol(head), value] if head.name.as_ref() == "import" => match value {
                Expr::Symbol(symbol) => Ok(ShareMode::Import(symbol.clone())),
                _ => Err(Error::TypeMismatch {
                    expected: "import symbol",
                    found: "non-symbol",
                }),
            },
            _ => Err(Error::Eval("unsupported share mode form".to_owned())),
        },
        _ => Err(Error::TypeMismatch {
            expected: "share mode",
            found: "non-share-mode",
        }),
    }
}

fn share_mode_value(cx: &mut Cx, mode: &ShareMode) -> Result<Value> {
    match mode {
        ShareMode::Share => cx.factory().symbol(Symbol::new("share")),
        ShareMode::Child => cx.factory().symbol(Symbol::new("child")),
        ShareMode::Isolate => cx.factory().symbol(Symbol::new("isolate")),
        ShareMode::Import(symbol) => cx.factory().table(vec![
            (
                Symbol::new("kind"),
                cx.factory().symbol(Symbol::new("import"))?,
            ),
            (Symbol::new("value"), cx.factory().symbol(symbol.clone())?),
        ]),
    }
}

fn derive_env(cx: &mut Cx, mode: &ShareMode) -> Result<Env> {
    match mode {
        ShareMode::Share => Ok(cx.env().clone()),
        ShareMode::Child => Ok(Env::child(Arc::new(cx.env().clone()))),
        ShareMode::Isolate => Ok(Env::default()),
        ShareMode::Import(symbol) => env_from_snapshot_expr(&snapshot_expr(cx, symbol)?, cx),
    }
}

fn derive_capabilities(cx: &mut Cx, mode: &ShareMode) -> Result<CapabilitySet> {
    match mode {
        ShareMode::Share | ShareMode::Child => Ok(cx.capabilities().clone()),
        ShareMode::Isolate => Ok(CapabilitySet::default()),
        ShareMode::Import(symbol) => capabilities_from_snapshot_expr(&snapshot_expr(cx, symbol)?),
    }
}

fn derive_factory(cx: &Cx, mode: &ShareMode) -> Arc<dyn Factory> {
    match mode {
        ShareMode::Share | ShareMode::Child => cx.factory_ref(),
        ShareMode::Isolate | ShareMode::Import(_) => Arc::new(DefaultFactory),
    }
}

fn derive_registry(
    cx: &mut Cx,
    registry_mode: &ShareMode,
    libs_mode: &ShareMode,
) -> Result<Registry> {
    let base = match registry_mode {
        ShareMode::Share | ShareMode::Child => cx.registry().clone(),
        ShareMode::Isolate => Registry::default(),
        ShareMode::Import(symbol) => registry_from_snapshot_expr(&snapshot_expr(cx, symbol)?, cx)?,
    };
    apply_libs_mode(cx, base, libs_mode)
}

fn apply_libs_mode(cx: &mut Cx, registry: Registry, mode: &ShareMode) -> Result<Registry> {
    match mode {
        ShareMode::Share | ShareMode::Child => Ok(registry),
        ShareMode::Isolate => Ok(Registry::default()),
        ShareMode::Import(symbol) => registry_from_snapshot_expr(&snapshot_expr(cx, symbol)?, cx),
    }
}

fn snapshot_expr(cx: &mut Cx, symbol: &Symbol) -> Result<Expr> {
    let value = cx
        .registry()
        .value_by_symbol(symbol)
        .cloned()
        .ok_or_else(|| Error::UnknownSymbol {
            symbol: symbol.clone(),
        })?;
    value.object().as_expr(cx)
}

fn env_from_snapshot_expr(expr: &Expr, cx: &mut Cx) -> Result<Env> {
    let Expr::Map(entries) = expr else {
        return Err(Error::TypeMismatch {
            expected: "env snapshot table",
            found: "non-table",
        });
    };
    let mut env = Env::default();
    for (key, value) in entries {
        let Expr::Symbol(symbol) = key else {
            return Err(Error::TypeMismatch {
                expected: "symbol table key",
                found: "non-symbol",
            });
        };
        env.define(symbol.clone(), expr_to_value(cx, value)?);
    }
    Ok(env)
}

fn capabilities_from_snapshot_expr(expr: &Expr) -> Result<CapabilitySet> {
    let mut capabilities = CapabilitySet::new();
    match expr {
        Expr::Nil => {}
        Expr::Symbol(symbol) => {
            capabilities.insert(CapabilityName::new(symbol.to_string()));
        }
        Expr::String(text) => {
            capabilities.insert(CapabilityName::new(text.clone()));
        }
        Expr::List(items) | Expr::Vector(items) => {
            for item in items {
                match item {
                    Expr::Symbol(symbol) => {
                        capabilities.insert(CapabilityName::new(symbol.to_string()));
                    }
                    Expr::String(text) => {
                        capabilities.insert(CapabilityName::new(text.clone()));
                    }
                    _ => {
                        return Err(Error::TypeMismatch {
                            expected: "capability symbol or string",
                            found: "non-capability",
                        });
                    }
                }
            }
        }
        _ => {
            return Err(Error::TypeMismatch {
                expected: "capability list",
                found: "non-list",
            });
        }
    }
    Ok(capabilities)
}

fn registry_from_snapshot_expr(expr: &Expr, cx: &mut Cx) -> Result<Registry> {
    let libs = match expr {
        Expr::Nil => Vec::new(),
        Expr::Symbol(symbol) => vec![symbol.clone()],
        Expr::List(items) | Expr::Vector(items) => items
            .iter()
            .map(|item| match item {
                Expr::Symbol(symbol) => Ok(symbol.clone()),
                _ => Err(Error::TypeMismatch {
                    expected: "library symbol",
                    found: "non-symbol",
                }),
            })
            .collect::<Result<Vec<_>>>()?,
        _ => {
            return Err(Error::TypeMismatch {
                expected: "registry snapshot symbol list",
                found: "non-list",
            });
        }
    };
    Ok(cx.registry().subset_for_libs(&libs))
}

fn expr_to_value(cx: &mut Cx, expr: &Expr) -> Result<Value> {
    match expr {
        Expr::Nil => cx.factory().nil(),
        Expr::Bool(value) => cx.factory().bool(*value),
        Expr::Number(number) => cx
            .factory()
            .number_literal(number.domain.clone(), number.canonical.clone()),
        Expr::Symbol(symbol) => cx.factory().symbol(symbol.clone()),
        Expr::String(text) => cx.factory().string(text.clone()),
        Expr::Bytes(bytes) => cx.factory().bytes(bytes.clone()),
        Expr::List(items) | Expr::Vector(items) => {
            let values = items
                .iter()
                .map(|item| expr_to_value(cx, item))
                .collect::<Result<Vec<_>>>()?;
            cx.factory().list(values)
        }
        Expr::Map(entries) => {
            let values = entries
                .iter()
                .map(|(key, value)| {
                    let Expr::Symbol(symbol) = key else {
                        return Err(Error::TypeMismatch {
                            expected: "symbol table key",
                            found: "non-symbol",
                        });
                    };
                    Ok((symbol.clone(), expr_to_value(cx, value)?))
                })
                .collect::<Result<Vec<_>>>()?;
            cx.factory().table(values)
        }
        _ => cx.factory().expr(expr.clone()),
    }
}

#[derive(Clone)]
pub(crate) struct IsolatedEvalSite {
    inner: Arc<dyn EvalSite>,
    policy: IsolationPolicy,
}

impl IsolatedEvalSite {
    pub(crate) fn wrap(inner: Arc<dyn EvalSite>, policy: IsolationPolicy) -> Arc<dyn EvalSite> {
        Arc::new(Self { inner, policy })
    }

    pub(crate) fn inner(&self) -> &Arc<dyn EvalSite> {
        &self.inner
    }
}

impl EvalSite for IsolatedEvalSite {
    fn site_kind(&self) -> &'static str {
        self.inner.site_kind()
    }

    fn address(&self) -> &ServerAddress {
        self.inner.address()
    }

    fn codecs(&self) -> &[Symbol] {
        self.inner.codecs()
    }

    fn answer(&self, cx: &mut Cx, frame: ServerFrame) -> Result<ServerFrame> {
        self.policy.apply(cx, |cx| self.inner.answer(cx, frame))
    }

    fn stream(
        &self,
        cx: &mut Cx,
        frame: ServerFrame,
        sink: &mut dyn crate::StreamSink,
    ) -> Result<()> {
        self.policy
            .apply(cx, |cx| self.inner.stream(cx, frame, sink))
    }

    fn as_eval_fabric(&self) -> Option<&dyn sim_kernel::EvalFabric> {
        self.inner.as_eval_fabric()
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}
