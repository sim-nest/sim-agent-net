// conformance: hostile local roadmap executor qualification.

#[cfg(test)]
#[allow(dead_code)]
#[path = "../../src/refiner.rs"]
mod refiner_product;

#[cfg(test)]

#[cfg(test)]
mod tests {
    include!("tests/common.rs");
    include!("tests/local_runner.rs");
    include!("tests/replay.rs");
    include!("tests/recovery.rs");
}

#[cfg(test)]
mod implementer_contract { include!("tests/implementer.rs"); }
#[cfg(test)]
mod supervisor_service { include!("tests/supervisor.rs"); }
#[cfg(test)]
mod mutation_contract { include!("tests/mutation.rs"); }
#[cfg(test)]
mod proof_leaf_tests { include!("tests/proof_leaf.rs"); }
