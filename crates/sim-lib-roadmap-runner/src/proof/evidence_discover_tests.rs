// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use std::collections::BTreeSet;

use super::discover_definitions;

#[test]
fn unverified_and_mismatched_members_are_not_definitions() {
    let text = "\
sim-world-inputs-v1
root-package lantern
domain resolver/lantern . compiling Cargo.lock
member resolver/lantern registry demo-crate 1.0.0 registry+https://github.com/rust-lang/crates.io-index - none closure
member resolver/lantern git demo-git 1.0.0 git+https://example.com/demo.git - none outside
member resolver/lantern path stale 0.1.0 crates/stale eeee5555 version-mismatch outside 0.2.0
member resolver/lantern registry sim-kernel 0.4.0 registry+https://github.com/rust-lang/crates.io-index cccc3333 registry closure
";
    let found = discover_definitions(text).unwrap();
    let names = found
        .iter()
        .map(|row| row.name.as_str())
        .collect::<BTreeSet<_>>();
    assert_eq!(names, BTreeSet::from(["lantern", "sim-kernel"]));
    assert!(found.iter().all(|row| row.class == "package"));
}

#[test]
fn definition_identity_keeps_domain_origin_and_source() {
    let base = "\
sim-world-inputs-v1
root-package lantern
domain resolver/lantern . compiling Cargo.lock
member resolver/lantern path lantern 1.0.0 . aaaa1111 path-content closure
member resolver/lantern registry sim-kernel 0.4.0 registry+https://github.com/rust-lang/crates.io-index cccc3333 registry closure
member resolver/lantern git sim-kernel 0.4.0 git+https://example.com/sim-kernel.git cccc3333 git outside
";
    let found = discover_definitions(base).unwrap();
    let registry = found
        .iter()
        .find(|row| {
            row.class == "package" && row.name == "sim-kernel" && row.semantic.contains("registry")
        })
        .unwrap();
    let git = found
        .iter()
        .find(|row| row.name == "sim-kernel" && row.semantic.contains("git+"))
        .unwrap();
    assert_ne!(registry.semantic, git.semantic);
    assert_ne!(registry.semantic, registry.storage);
    let root = found
        .iter()
        .find(|row| row.class == "package" && row.name == "lantern")
        .unwrap();
    assert!(root.semantic.contains("resolver/lantern"));
    assert!(root.semantic.contains("1.0.0"));
    let other = base.replace("resolver/lantern", "resolver/other");
    let other_root = discover_definitions(&other)
        .unwrap()
        .into_iter()
        .find(|row| row.name == "lantern")
        .unwrap();
    assert_ne!(root.semantic, other_root.semantic);
    let bumped = base.replace("lantern 1.0.0 .", "lantern 2.0.0 .");
    let bumped_root = discover_definitions(&bumped)
        .unwrap()
        .into_iter()
        .find(|row| row.name == "lantern")
        .unwrap();
    assert_ne!(root.semantic, bumped_root.semantic);
    let spelled = "\
sim-world-inputs-v1
root-package parent
spell /work/sim-runtime/ compiling Cargo.lock
spell /work/sim-runtime/. compiling Cargo.lock
member /work/sim-runtime path parent 0.1.0 crates/parent abcdef path-content closure
";
    let spelled = discover_definitions(spelled).unwrap();
    assert_eq!(spelled.len(), 2);
    assert!(spelled.iter().all(|row| row.domain == "/work/sim-runtime"));
    assert!(spelled.iter().any(|row| row.class == "workspace"));
}

#[test]
fn ambiguous_root_package_keeps_the_other_definitions() {
    let spelled = "\
sim-world-inputs-v1
root-package parent
spell /work/sim-runtime compiling Cargo.lock
spell /work/sim-runtime/nested compiling nested/Cargo.lock
member /work/sim-runtime path parent 0.1.0 crates/parent abcdef path-content closure
member /work/sim-runtime/nested path nested-crate 0.1.0 crates/nested 123456 path-content closure
";
    let found = discover_definitions(spelled).unwrap();
    assert_eq!(found.len(), 2);
    assert!(found.iter().all(|row| row.class == "workspace"));
    assert_ne!(found[0].semantic, found[1].semantic);
    let enumerated = "\
sim-world-inputs-v1
root-package demo
domain resolver/demo . enumerated Cargo.lock
member resolver/demo registry demo-crate 1.0.0 registry+https://github.com/rust-lang/crates.io-index abcdef registry closure
generated resolver/demo src/generated.rs 4 aaaa
";
    let found = discover_definitions(enumerated).unwrap();
    let classes = found.iter().map(|row| row.class).collect::<BTreeSet<_>>();
    assert_eq!(classes, BTreeSet::from(["generate", "package"]));
    assert!(found.iter().all(|row| row.name != "demo"));
}
