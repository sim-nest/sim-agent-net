//! A [`LibCatalog`] over the crate-shipped seed domains.
//!
//! `sim-lib-cookbook` already depends on the seeded domain libs (to embed their
//! recipe books), so exposing a catalog over them adds NO new dependency and no
//! cycle: it is a convenience for hosts, gated on the same `seed-recipes`
//! feature. A production host assembles its own catalog
//! from the full standard distribution at the facade level; this one covers the
//! numbers domains, which is enough to demonstrate Category A (pure compute that
//! only needs its lib loaded).
//!
//! A recipe resolves against this catalog by its `requires` name (`numbers/cas`,
//! or the tail `cas`): the returned lib's own manifest id IS the cookbook name,
//! so there is no separate name table to drift.

use std::sync::Arc;

use sim_cookbook::EmbeddedDir;
use sim_kernel::{CodecId, Lib};

use crate::catalog::LibCatalog;
use crate::config::{ConfigProvider, LoadableLibResolver, ResolvedLoadable, built_in_config};
use crate::loadable::LoadableLibList;

/// A catalog over the seeded numbers domains.
pub struct SeededLibCatalog {
    libs: Vec<Box<dyn Lib>>,
}

impl SeededLibCatalog {
    /// Build a catalog over the seeded numbers domain libs.
    ///
    /// Each lib is a pure, deterministic number domain; loading one adds its ops
    /// (construction, arithmetic, CAS diff/eval, tensor arithmetic) to the eval
    /// `Cx` without any effect capability.
    pub fn standard() -> Self {
        let libs: Vec<Box<dyn Lib>> = vec![
            Box::new(sim_lib_numbers_i64::I64NumbersLib::new()),
            Box::new(sim_lib_numbers_arith::NumbersArithmeticLib::new()),
            Box::new(sim_lib_numbers_bigint::BigIntNumbersLib::new()),
            Box::new(sim_lib_numbers_bool::BoolNumbersLib::new()),
            Box::new(sim_lib_numbers_f64::F64NumbersLib::new()),
            Box::new(sim_lib_numbers_rational::RationalNumbersLib::new()),
            Box::new(sim_lib_numbers_complex::ComplexNumbersLib::new()),
            Box::new(sim_lib_numbers_func::FuncNumbersLib::new()),
            Box::new(sim_lib_numbers_cas::CasNumbersLib::new()),
            Box::new(sim_lib_numbers_tensor::TensorNumbersLib::new()),
            // Tensor broadcast arithmetic loads via `requires`, so
            // `tensor-scale` (`(math/mul (vec 3 4) 2)`) runs host-independently
            // rather than only through the boot prelude.
            Box::new(sim_lib_numbers_tensor_bcast::TensorBroadcastLib::new()),
            Box::new(sim_lib_discrete::DiscreteLib),
            // Eval-policy special forms load on demand via a recipe's `requires`
            // (`binding`/`control`/`sequence`/`pattern`), so
            // `let`/`if`/`seq/*`/`match` recipes run without preloading them.
            Box::new(sim_lib_binding::BindingLib),
            Box::new(sim_lib_control::ControlLib),
            Box::new(sim_lib_sequence::SequenceLib),
            Box::new(sim_lib_pattern::PatternLib),
            // Language codecs load on demand so a recipe with
            // `codec = "algol"`/`"scheme"` parses+evals its own surface. Each is
            // bound to a distinct fixed CodecId that cannot collide with the
            // boot `codec/lisp` (id 1); both lower an eval-position surface to a
            // Term the general evaluator runs. Algol depends on `codec/lisp`
            // (boot-loaded).
            Box::new(sim_codec_algol::AlgolCodecLib::new(CodecId(201))),
            Box::new(sim_lib_lang_scheme::SchemeCodecLib::new(CodecId(202))),
            // Offline MIDI render reduced to a deterministic frame digest (no
            // device/clock/entropy).
            Box::new(sim_lib_midi_core::MidiDigestLib),
        ];
        Self { libs }
    }

    /// Build the default host loadable-lib directory for the seeded catalog.
    pub fn loadable_libs() -> LoadableLibList {
        let resolver = BuiltInLoadableResolver;
        let provider = ConfigProvider::new(built_in_config(), &resolver);
        let (directory, diagnostics) = provider.loadable_libs();
        debug_assert!(diagnostics.is_empty(), "{diagnostics:?}");
        directory
    }
}

impl LibCatalog for SeededLibCatalog {
    fn resolve(&self, name: &str) -> Option<&dyn Lib> {
        self.libs
            .iter()
            .find(|lib| {
                let id = lib.manifest().id;
                id.as_qualified_str() == name || id.name.as_ref() == name
            })
            .map(|lib| lib.as_ref())
    }
}

/// Resolver for the seed-recipes built-in loadable-lib directory.
pub struct BuiltInLoadableResolver;

impl LoadableLibResolver for BuiltInLoadableResolver {
    fn resolve(&self, source: &str, _id: &str) -> Option<ResolvedLoadable> {
        match source {
            "symbol:numbers/i64" => Some(resolved(
                "I64 numbers",
                Some(sim_lib_numbers_i64::RECIPES),
                || Box::new(sim_lib_numbers_i64::I64NumbersLib::new()),
            )),
            "symbol:numbers/arith" => Some(resolved(
                "Arithmetic numbers",
                Some(sim_lib_numbers_arith::RECIPES),
                || Box::new(sim_lib_numbers_arith::NumbersArithmeticLib::new()),
            )),
            "symbol:numbers/bigint" => Some(resolved(
                "Big integer numbers",
                Some(sim_lib_numbers_bigint::RECIPES),
                || Box::new(sim_lib_numbers_bigint::BigIntNumbersLib::new()),
            )),
            "symbol:numbers/bool" => Some(resolved(
                "Boolean numbers",
                Some(sim_lib_numbers_bool::RECIPES),
                || Box::new(sim_lib_numbers_bool::BoolNumbersLib::new()),
            )),
            "symbol:numbers/f64" => Some(resolved(
                "F64 numbers",
                Some(sim_lib_numbers_f64::RECIPES),
                || Box::new(sim_lib_numbers_f64::F64NumbersLib::new()),
            )),
            "symbol:numbers/rational" => Some(resolved(
                "Rational numbers",
                Some(sim_lib_numbers_rational::RECIPES),
                || Box::new(sim_lib_numbers_rational::RationalNumbersLib::new()),
            )),
            "symbol:numbers/complex" => Some(resolved(
                "Complex numbers",
                Some(sim_lib_numbers_complex::RECIPES),
                || Box::new(sim_lib_numbers_complex::ComplexNumbersLib::new()),
            )),
            "symbol:numbers/func" => Some(resolved(
                "Function numbers",
                Some(sim_lib_numbers_func::RECIPES),
                || Box::new(sim_lib_numbers_func::FuncNumbersLib::new()),
            )),
            "symbol:numbers/cas" => Some(resolved(
                "CAS numbers",
                Some(sim_lib_numbers_cas::RECIPES),
                || Box::new(sim_lib_numbers_cas::CasNumbersLib::new()),
            )),
            "symbol:numbers/tensor" => Some(resolved(
                "Tensor numbers",
                Some(sim_lib_numbers_tensor::RECIPES),
                || Box::new(sim_lib_numbers_tensor::TensorNumbersLib::new()),
            )),
            "symbol:numbers/tensor-bcast" => {
                Some(resolved("Tensor broadcast numbers", None, || {
                    Box::new(sim_lib_numbers_tensor_bcast::TensorBroadcastLib::new())
                }))
            }
            "symbol:discrete" => Some(resolved(
                "Discrete math",
                Some(sim_lib_discrete::RECIPES),
                || Box::new(sim_lib_discrete::DiscreteLib),
            )),
            "symbol:organ/binding" => Some(resolved(
                "Binding organ",
                Some(sim_lib_binding::RECIPES),
                || Box::new(sim_lib_binding::BindingLib),
            )),
            "symbol:organ/control" => Some(resolved(
                "Control organ",
                Some(sim_lib_control::RECIPES),
                || Box::new(sim_lib_control::ControlLib),
            )),
            "symbol:organ/sequence" => Some(resolved(
                "Sequence organ",
                Some(sim_lib_sequence::RECIPES),
                || Box::new(sim_lib_sequence::SequenceLib),
            )),
            "symbol:organ/pattern" => Some(resolved(
                "Pattern organ",
                Some(sim_lib_pattern::RECIPES),
                || Box::new(sim_lib_pattern::PatternLib),
            )),
            "symbol:codec/algol" => Some(resolved(
                "Algol codec",
                Some(sim_codec_algol::RECIPES),
                || Box::new(sim_codec_algol::AlgolCodecLib::new(CodecId(201))),
            )),
            "symbol:codec/scheme-r7rs-small" => Some(resolved(
                "Scheme codec",
                Some(sim_lib_lang_scheme::RECIPES),
                || Box::new(sim_lib_lang_scheme::SchemeCodecLib::new(CodecId(202))),
            )),
            "symbol:midi/digest" => Some(resolved(
                "MIDI digest",
                Some(sim_lib_midi_core::RECIPES),
                || Box::new(sim_lib_midi_core::MidiDigestLib),
            )),
            _ => None,
        }
    }
}

fn resolved<F>(title: &str, recipes: Option<EmbeddedDir>, make: F) -> ResolvedLoadable
where
    F: Fn() -> Box<dyn Lib + Send + Sync> + Send + Sync + 'static,
{
    ResolvedLoadable {
        title: title.to_owned(),
        recipes,
        factory: Arc::new(make),
    }
}
