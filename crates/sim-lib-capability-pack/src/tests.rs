use crate::*;
use sim_citizen::CitizenField;
use sim_kernel::{Expr, Symbol};
use std::collections::{BTreeMap, BTreeSet};

fn id(n: char) -> ContentId {
    ContentId::parse(format!("sha256:{}", n.to_string().repeat(64))).unwrap()
}
fn syms(values: &[&str]) -> BTreeSet<Symbol> {
    values.iter().map(|v| Symbol::new(*v)).collect()
}
fn import(alias: &str, content: &ContentId, caps: &[&str]) -> Expr {
    Expr::List(vec![
        Symbol::new(alias).encode_field(),
        content.as_str().to_owned().encode_field(),
        caps.iter()
            .map(|v| Symbol::new(*v))
            .collect::<Vec<_>>()
            .encode_field(),
    ])
}
fn library(name: &str, effects: &[&str]) -> Expr {
    Expr::List(vec![
        Symbol::new(name).encode_field(),
        Symbol::qualified("route", name).encode_field(),
        Symbol::qualified("shape", name).encode_field(),
        effects
            .iter()
            .map(|v| Symbol::new(*v))
            .collect::<Vec<_>>()
            .encode_field(),
    ])
}
fn claim(name: &str, cap: &str, lib: &str) -> Expr {
    Expr::List(vec![
        Symbol::new(name).encode_field(),
        Symbol::new(cap).encode_field(),
        Symbol::new(lib).encode_field(),
    ])
}
fn specimen(id: &str, outcome: &str) -> Expr {
    Expr::List(vec![
        Symbol::new(id).encode_field(),
        Symbol::new(outcome).encode_field(),
    ])
}
fn fallback() -> Expr {
    Expr::List(vec![
        Symbol::new("route-gap").encode_field(),
        "select the documented manual route"
            .to_owned()
            .encode_field(),
    ])
}
fn pack(content: &ContentId, imports: Vec<Expr>, lib: &str, cap: &str) -> CapabilityPack {
    CapabilityPack {
        content: content.to_string(),
        imports,
        libraries: vec![library(lib, &[cap])],
        claims: vec![claim("required", cap, lib)],
        outputs: vec![Expr::List(vec![
            Symbol::new(lib).encode_field(),
            Symbol::new(lib).encode_field(),
        ])],
        surfaces: vec![Expr::List(vec![
            Symbol::new(lib).encode_field(),
            Symbol::new("public").encode_field(),
        ])],
        specimens: vec![
            specimen(&format!("{lib}-ok"), "success"),
            specimen(&format!("{lib}-no"), "refusal"),
        ],
        fallbacks: vec![fallback()],
        ..CapabilityPack::default()
    }
}

#[derive(Default)]
struct Dir(BTreeMap<ContentId, CapabilityPack>);
impl PackDir for Dir {
    fn get(&self, id: &ContentId) -> Option<(ContentId, CapabilityPack)> {
        self.0.get(id).cloned().map(|p| (id.clone(), p))
    }
}
struct Cat;
impl Catalog for Cat {
    fn has_route(&self, _: &Symbol) -> bool {
        true
    }
    fn has_shape(&self, _: &Symbol) -> bool {
        true
    }
    fn effects(&self, route: &Symbol) -> Option<BTreeSet<Symbol>> {
        Some(syms(&[if route.name.as_ref() == "b" {
            "write"
        } else {
            "read"
        }]))
    }
    fn has_disclosure(&self, d: &Symbol) -> bool {
        d == &Symbol::new("public")
    }
}
#[derive(Default)]
struct Loader(Vec<Symbol>);
impl LibraryLoader for Loader {
    fn load(&mut self, l: &LibrarySpec) -> Result<(), String> {
        self.0.push(l.id.clone());
        Ok(())
    }
}

#[test]
fn two_roots_share_one_import_and_resolve_deterministically() {
    let shared = id('a');
    let left = id('b');
    let right = id('c');
    let mut dir = Dir::default();
    dir.0
        .insert(shared.clone(), pack(&shared, vec![], "shared", "read"));
    dir.0.insert(
        left.clone(),
        pack(
            &left,
            vec![import("common", &shared, &["read"])],
            "left",
            "read",
        ),
    );
    dir.0.insert(
        right.clone(),
        pack(
            &right,
            vec![import("common", &shared, &["read"])],
            "right",
            "read",
        ),
    );
    for root in [left, right] {
        let closure = resolve(&dir, root.clone(), syms(&["read", "write"])).unwrap();
        assert_eq!(
            closure
                .packs
                .iter()
                .map(|(id, _)| id.clone())
                .collect::<Vec<_>>(),
            vec![shared.clone(), root.clone()]
        );
        let checked = validate(closure, &Cat).unwrap();
        let mut loader = Loader::default();
        let loaded = load(checked, &mut loader).unwrap();
        assert_eq!(loaded.libraries.len(), 2);
    }
}

#[test]
fn cycle_and_authority_widening_refuse_before_load() {
    let a = id('a');
    let b = id('b');
    let mut cyclic = Dir::default();
    cyclic.0.insert(
        a.clone(),
        pack(&a, vec![import("b", &b, &["read"])], "a", "read"),
    );
    cyclic.0.insert(
        b.clone(),
        pack(&b, vec![import("a", &a, &["read"])], "b", "read"),
    );
    assert!(matches!(
        resolve(&cyclic, a.clone(), syms(&["read"])),
        Err(ResolveError::Cycle(_))
    ));
    let mut widening = Dir::default();
    widening.0.insert(
        a.clone(),
        pack(&a, vec![import("b", &b, &["read"])], "a", "read"),
    );
    widening.0.insert(b.clone(), pack(&b, vec![], "b", "write"));
    let closure = resolve(&widening, a, syms(&["read", "write"])).unwrap();
    assert!(matches!(
        validate(closure, &Cat),
        Err(ValidationError::AuthorityWidening { .. })
    ));
}

#[test]
fn mutable_missing_and_alias_conflict_refuse() {
    assert!(ContentId::parse("latest").is_err());
    let missing = id('d');
    assert!(matches!(
        resolve(&Dir::default(), missing, syms(&[])),
        Err(ResolveError::Missing(_))
    ));
    let root = id('e');
    let one = id('1');
    let two = id('2');
    let mut dir = Dir::default();
    dir.0.insert(
        root.clone(),
        pack(
            &root,
            vec![import("same", &one, &[]), import("same", &two, &[])],
            "root",
            "read",
        ),
    );
    dir.0.insert(one.clone(), pack(&one, vec![], "one", "read"));
    dir.0.insert(two.clone(), pack(&two, vec![], "two", "read"));
    assert!(matches!(
        resolve(&dir, root, syms(&["read"])),
        Err(ResolveError::AliasConflict(_))
    ));
}

#[test]
fn citizen_codec_round_trips_pack() {
    let content = id('f');
    let value = pack(&content, vec![], "demo", "read");
    let encoded = encode_pack(&value);
    assert_eq!(decode_pack(CURRENT_PACK_VERSION, &encoded).unwrap(), value);
}
// conformance: capability-pack tests prove deterministic closure and diminishing authority.
