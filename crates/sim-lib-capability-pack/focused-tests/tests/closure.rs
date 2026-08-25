use std::collections::{BTreeMap, BTreeSet};
use sim_lib_capability_pack::*;
use sim_kernel::{Expr, Symbol};

fn id(c: char) -> ContentId { ContentId::parse(format!("sha256:{}", c.to_string().repeat(64))).unwrap() }
fn set(v: &[&str]) -> BTreeSet<Symbol> { v.iter().map(|s| Symbol::new(*s)).collect() }
fn expr_import(alias: &str, id: &ContentId, ceiling: &[&str]) -> Expr { Expr::List(vec![Expr::Symbol(Symbol::new(alias)), Expr::String(id.to_string()), Expr::List(ceiling.iter().map(|v| Expr::Symbol(Symbol::new(*v))).collect())]) }
fn pack(id: &ContentId, imports: Vec<Expr>, library: &str, capability: &str) -> CapabilityPack {
    let s=|v:&str| Expr::Symbol(Symbol::new(v));
    CapabilityPack { content:id.to_string(), imports,
        libraries:vec![Expr::List(vec![s(library),Expr::Symbol(Symbol::qualified("route",library)),Expr::Symbol(Symbol::qualified("shape",library)),Expr::List(vec![s(capability)])])],
        claims:vec![Expr::List(vec![s("claim"),s(capability),s(library)])],
        outputs:vec![Expr::List(vec![s(library),s(library)])], surfaces:vec![Expr::List(vec![s("tty"),s("public")])],
        specimens:vec![Expr::List(vec![s("ok"),s("success")]),Expr::List(vec![s("no"),s("refusal")])],
        fallbacks:vec![Expr::List(vec![s("gap"),Expr::String("use manual route".into())])], ..CapabilityPack::default() }
}
#[derive(Default)] struct Dir(BTreeMap<ContentId,CapabilityPack>);
impl PackDir for Dir { fn get(&self,id:&ContentId)->Option<(ContentId,CapabilityPack)>{self.0.get(id).cloned().map(|p|(id.clone(),p))} }
struct Cat;
impl Catalog for Cat { fn has_route(&self,_:&Symbol)->bool{true} fn has_shape(&self,_:&Symbol)->bool{true} fn effects(&self,r:&Symbol)->Option<BTreeSet<Symbol>>{Some(set(&[if r.name.as_ref()=="b"{"write"}else{"read"}]))} fn has_disclosure(&self,_:&Symbol)->bool{true} }

#[test] fn independent_roots_share_one_deterministic_import(){let shared=id('a');let one=id('b');let two=id('c');let mut d=Dir::default();d.0.insert(shared.clone(),pack(&shared,vec![],"shared","read"));for root in [&one,&two]{d.0.insert(root.clone(),pack(root,vec![expr_import("shared",&shared,&["read"])],root.as_str(),"read"));}for root in [one,two]{let r=resolve(&d,root.clone(),set(&["read","write"])).unwrap();assert_eq!(r.packs[0].0,shared);assert_eq!(r.packs[1].0,root);assert!(validate(r,&Cat).is_ok());}}

#[test] fn cycle_and_authority_widening_refuse_during_preflight(){let a=id('a');let b=id('b');let mut d=Dir::default();d.0.insert(a.clone(),pack(&a,vec![expr_import("b",&b,&["read"])],"a","read"));d.0.insert(b.clone(),pack(&b,vec![expr_import("a",&a,&["read"])],"b","read"));assert!(matches!(resolve(&d,a.clone(),set(&["read"])),Err(ResolveError::Cycle(_))));d.0.insert(b.clone(),pack(&b,vec![],"b","write"));let r=resolve(&d,a,set(&["read","write"])).unwrap();assert!(matches!(validate(r,&Cat),Err(ValidationError::AuthorityWidening{..})));}
