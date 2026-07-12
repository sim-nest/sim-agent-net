//! A [`LibCatalog`] over the crate-shipped seed domains.
//!
//! `sim-lib-cookbook` already depends on the seeded domain libs (to embed their
//! recipe books), so exposing a catalog over them adds NO new dependency and no
//! cycle: it is a convenience for hosts and for the COOKBOOK_7 spike, gated on
//! the same `seed-recipes` feature. A production host assembles its own catalog
//! from the full standard distribution at the facade level; this one covers the
//! numbers domains, which is enough to demonstrate Category A (pure compute that
//! only needs its lib loaded).
//!
//! A recipe resolves against this catalog by its `requires` name (`numbers/cas`,
//! or the tail `cas`): the returned lib's own manifest id IS the cookbook name,
//! so there is no separate name table to drift.

use sim_kernel::{CodecId, Lib};

use crate::catalog::LibCatalog;

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
            // COOK8.07: tensor broadcast arithmetic, so `tensor-scale`
            // (`(math/mul (vec 3 4) 2)`) loads via `requires` and runs
            // host-independently rather than only through the boot prelude.
            Box::new(sim_lib_numbers_tensor_bcast::TensorBroadcastLib::new()),
            Box::new(sim_lib_discrete::DiscreteLib),
            // COOK8.03 organs: the eval-policy special forms load on demand via a
            // recipe's `requires` (`binding`/`control`/`sequence`/`pattern`), so
            // `let`/`if`/`seq/*`/`match` recipes run without preloading them.
            Box::new(sim_lib_binding::BindingLib),
            Box::new(sim_lib_control::ControlLib),
            Box::new(sim_lib_sequence::SequenceLib),
            Box::new(sim_lib_pattern::PatternLib),
            // COOK8.05 language codecs: loaded on demand so a recipe with
            // `codec = "algol"`/`"scheme"` parses+evals its own surface. Each is
            // bound to a distinct fixed CodecId that cannot collide with the boot
            // `codec/lisp` (id 1); both lower an eval-position surface to a Term the
            // general evaluator runs. Algol depends on `codec/lisp` (boot-loaded).
            Box::new(sim_codec_algol::AlgolCodecLib::new(CodecId(201))),
            Box::new(sim_lib_lang_scheme::SchemeCodecLib::new(CodecId(202))),
            // COOK8.06 Category C: an offline MIDI render reduced to a
            // deterministic frame digest (no device/clock/entropy).
            Box::new(sim_lib_midi_core::MidiDigestLib),
        ];
        Self { libs }
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
