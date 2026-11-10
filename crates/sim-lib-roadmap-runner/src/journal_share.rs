// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Persistent fact and evidence roots.
//!
//! A snapshot stores the roots of two balanced trees. An unchanged subtree
//! keeps its content id, so a later snapshot writes only the path of each
//! changed key. The empty root is one canonical datum and is not rewritten.

use std::collections::{BTreeMap, BTreeSet};

use sim_kernel::{ContentId, Datum, NumberLiteral, Symbol};
use sim_lib_journal::JournalObject;

use crate::ExecutionJournalError;
use crate::causal::{content_id_datum, content_id_from_datum};

const EMPTY_TAG: &str = "share-empty-v1";
const NODE_TAG: &str = "share-node-v1";
const SNAPSHOT_TAG: &str = "snapshot-v1";

struct Node {
    key: String,
    value: ContentId,
    height: u64,
    left: Option<ContentId>,
    right: Option<ContentId>,
}

pub(crate) trait ShareIo {
    fn load_share(&self, id: &ContentId) -> Result<Datum, ExecutionJournalError>;
    fn persist_share(&self, datum: Datum) -> Result<ContentId, ExecutionJournalError>;
}

struct Store<'a> {
    fresh: BTreeMap<ContentId, Datum>,
    io: &'a dyn ShareIo,
}

pub(crate) fn empty_id() -> ContentId {
    empty_datum()
        .content_id()
        .expect("empty share datum is canonical")
}

pub(crate) fn is_empty(id: &ContentId) -> bool {
    id == &empty_id()
}

pub(crate) fn snapshot_datum(facts: &ContentId, evidence: &ContentId) -> Datum {
    Datum::Node {
        tag: Symbol::qualified("roadmap-execution", SNAPSHOT_TAG),
        fields: vec![
            (Symbol::new("facts"), content_id_datum(facts)),
            (Symbol::new("evidence-set"), content_id_datum(evidence)),
        ],
    }
}

pub(crate) fn open_snapshot(
    value: &Datum,
) -> Result<(ContentId, ContentId), ExecutionJournalError> {
    let Datum::Node { tag, fields } = value else {
        return Err(ExecutionJournalError::Semantic("snapshot state"));
    };
    if *tag != Symbol::qualified("roadmap-execution", SNAPSHOT_TAG) || fields.len() != 2 {
        return Err(ExecutionJournalError::Semantic("snapshot state"));
    }
    let facts = field(fields, "facts")?;
    let evidence = field(fields, "evidence-set")?;
    Ok((
        content_id_from_datum(facts)?,
        content_id_from_datum(evidence)?,
    ))
}

#[cfg(test)]
pub(crate) fn load_facts(
    root: &ContentId,
    datums: &BTreeMap<ContentId, Datum>,
    objects: &BTreeMap<ContentId, Vec<u8>>,
) -> Result<BTreeMap<String, ContentId>, ExecutionJournalError> {
    load_facts_with(root, &MapIo { datums, objects })
}

pub(crate) fn load_evidence(
    root: &ContentId,
    datums: &BTreeMap<ContentId, Datum>,
    objects: &BTreeMap<ContentId, Vec<u8>>,
) -> Result<BTreeSet<ContentId>, ExecutionJournalError> {
    let mut evidence = BTreeSet::new();
    walk(
        root,
        &MapIo { datums, objects },
        &mut BTreeSet::new(),
        0,
        &mut |key, value| {
            if key != id_key(&value) {
                return Err(ExecutionJournalError::Semantic("shared state"));
            }
            evidence.insert(value);
            Ok(())
        },
    )?;
    Ok(evidence)
}

pub(crate) fn evidence_additions(
    previous: &ContentId,
    next: &ContentId,
    datums: &BTreeMap<ContentId, Datum>,
    objects: &BTreeMap<ContentId, Vec<u8>>,
) -> Result<Vec<ContentId>, ExecutionJournalError> {
    tree::evidence_additions(previous, next, &MapIo { datums, objects })
}

pub(crate) fn fact_changes(
    previous: &ContentId,
    next: &ContentId,
    datums: &BTreeMap<ContentId, Datum>,
    objects: &BTreeMap<ContentId, Vec<u8>>,
) -> Result<BTreeMap<String, Option<ContentId>>, ExecutionJournalError> {
    tree::fact_changes(previous, next, &MapIo { datums, objects })
}

fn load_facts_with(
    root: &ContentId,
    io: &dyn ShareIo,
) -> Result<BTreeMap<String, ContentId>, ExecutionJournalError> {
    let mut facts = BTreeMap::new();
    walk(root, io, &mut BTreeSet::new(), 0, &mut |key, value| {
        facts.insert(key, value);
        Ok(())
    })?;
    Ok(facts)
}

pub(crate) fn sync_facts(
    previous: &ContentId,
    facts: &BTreeMap<String, ContentId>,
    io: &dyn ShareIo,
) -> Result<ContentId, ExecutionJournalError> {
    let mut store = Store {
        fresh: BTreeMap::new(),
        io,
    };
    if is_empty(previous) {
        let entries: Vec<_> = facts
            .iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect();
        return store.build(&entries);
    }
    let current = load_facts_with(previous, io)?;
    let mut root = Some(previous.clone());
    let removed: Vec<_> = current
        .keys()
        .filter(|key| !facts.contains_key(key.as_str()))
        .cloned()
        .collect();
    for key in removed {
        root = store.delete(root, &key)?;
    }
    for (key, value) in facts {
        if current.get(key) != Some(value) {
            root = Some(store.insert(root, key, value)?);
        }
    }
    Ok(root.unwrap_or_else(empty_id))
}

pub(crate) fn insert_evidence(
    previous: &ContentId,
    added: &[ContentId],
    io: &dyn ShareIo,
) -> Result<ContentId, ExecutionJournalError> {
    let mut store = Store {
        fresh: BTreeMap::new(),
        io,
    };
    let mut root = if is_empty(previous) {
        None
    } else {
        Some(previous.clone())
    };
    for id in added {
        root = Some(store.insert(root, &id_key(id), id)?);
    }
    Ok(root.unwrap_or_else(empty_id))
}

fn walk(
    root: &ContentId,
    io: &dyn ShareIo,
    seen: &mut BTreeSet<ContentId>,
    depth: u32,
    visit: &mut dyn FnMut(String, ContentId) -> Result<(), ExecutionJournalError>,
) -> Result<(), ExecutionJournalError> {
    if is_empty(root) {
        return Ok(());
    }
    if depth > 64 || !seen.insert(root.clone()) {
        return Err(ExecutionJournalError::Semantic("shared state"));
    }
    let node = node_from(&io.load_share(root)?)?;
    if let Some(left) = node.left.clone() {
        walk(&left, io, seen, depth + 1, visit)?;
    }
    visit(node.key, node.value)?;
    if let Some(right) = node.right {
        walk(&right, io, seen, depth + 1, visit)?;
    }
    Ok(())
}

struct MapIo<'a> {
    datums: &'a BTreeMap<ContentId, Datum>,
    objects: &'a BTreeMap<ContentId, Vec<u8>>,
}

impl ShareIo for MapIo<'_> {
    fn load_share(&self, id: &ContentId) -> Result<Datum, ExecutionJournalError> {
        let datum = self
            .datums
            .get(id)
            .cloned()
            .ok_or(ExecutionJournalError::MissingObject)?;
        let object = JournalObject::from_datum(datum.clone())
            .map_err(|_| ExecutionJournalError::Semantic("shared state"))?;
        if &object.id != id {
            return Err(ExecutionJournalError::Semantic("shared state"));
        }
        match self.objects.get(id) {
            Some(bytes) if bytes == &object.bytes => Ok(datum),
            Some(_) => Err(ExecutionJournalError::Semantic("shared state")),
            None => Err(ExecutionJournalError::MissingObject),
        }
    }

    fn persist_share(&self, _datum: Datum) -> Result<ContentId, ExecutionJournalError> {
        Err(ExecutionJournalError::Semantic("shared state"))
    }
}

#[path = "journal_share_tree.rs"]
mod tree;

pub(crate) fn empty_datum() -> Datum {
    Datum::Node {
        tag: Symbol::qualified("roadmap-execution", EMPTY_TAG),
        fields: Vec::new(),
    }
}

pub(crate) fn node_datum(
    key: &str,
    value: &ContentId,
    height: u64,
    left: Option<&ContentId>,
    right: Option<&ContentId>,
) -> Datum {
    Datum::Node {
        tag: Symbol::qualified("roadmap-execution", NODE_TAG),
        fields: vec![
            (Symbol::new("height"), height_datum(height)),
            (Symbol::new("key"), Datum::String(key.to_owned())),
            (Symbol::new("left"), child_datum(left)),
            (Symbol::new("right"), child_datum(right)),
            (Symbol::new("value"), content_id_datum(value)),
        ],
    }
}

fn node_from(value: &Datum) -> Result<Node, ExecutionJournalError> {
    let Datum::Node { tag, fields } = value else {
        return Err(ExecutionJournalError::Semantic("shared state"));
    };
    if *tag != Symbol::qualified("roadmap-execution", NODE_TAG) || fields.len() != 5 {
        return Err(ExecutionJournalError::Semantic("shared state"));
    }
    let height = match field(fields, "height")? {
        Datum::Number(number) => number
            .canonical
            .parse::<u64>()
            .map_err(|_| ExecutionJournalError::Semantic("shared state"))?,
        _ => return Err(ExecutionJournalError::Semantic("shared state")),
    };
    let Datum::String(key) = field(fields, "key")? else {
        return Err(ExecutionJournalError::Semantic("shared state"));
    };
    Ok(Node {
        key: key.clone(),
        value: content_id_from_datum(field(fields, "value")?)?,
        height,
        left: child_id(field(fields, "left")?)?,
        right: child_id(field(fields, "right")?)?,
    })
}

fn field<'a>(
    fields: &'a [(Symbol, Datum)],
    name: &str,
) -> Result<&'a Datum, ExecutionJournalError> {
    fields
        .iter()
        .find_map(|(field, value)| (*field == Symbol::new(name)).then_some(value))
        .ok_or(ExecutionJournalError::Semantic("shared state"))
}

fn child_datum(id: Option<&ContentId>) -> Datum {
    match id {
        Some(id) => content_id_datum(id),
        None => Datum::Symbol(Symbol::new("empty")),
    }
}

fn child_id(value: &Datum) -> Result<Option<ContentId>, ExecutionJournalError> {
    match value {
        Datum::Symbol(symbol) if *symbol == Symbol::new("empty") => Ok(None),
        other => Ok(Some(content_id_from_datum(other)?)),
    }
}

fn height_datum(height: u64) -> Datum {
    Datum::Number(NumberLiteral {
        domain: Symbol::qualified("numbers", "u64"),
        canonical: height.to_string(),
    })
}

pub(crate) fn id_key(id: &ContentId) -> String {
    let mut key = String::new();
    if let Some(namespace) = id.algorithm.namespace.as_deref() {
        key.push_str(namespace);
        key.push('/');
    }
    key.push_str(id.algorithm.name.as_ref());
    key.push(':');
    const HEX: &[u8; 16] = b"0123456789abcdef";
    for byte in id.bytes {
        key.push(HEX[(byte >> 4) as usize] as char);
        key.push(HEX[(byte & 0x0f) as usize] as char);
    }
    key
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Mem {
        datums: std::cell::RefCell<BTreeMap<ContentId, Datum>>,
        loads: std::cell::Cell<usize>,
    }

    fn mem() -> Mem {
        Mem {
            datums: std::cell::RefCell::new(BTreeMap::new()),
            loads: std::cell::Cell::new(0),
        }
    }

    fn stored_objects(datums: &BTreeMap<ContentId, Datum>) -> BTreeMap<ContentId, Vec<u8>> {
        datums
            .iter()
            .map(|(id, datum)| {
                let object = JournalObject::from_datum(datum.clone()).expect("share datum");
                (id.clone(), object.bytes)
            })
            .collect()
    }

    impl ShareIo for Mem {
        fn load_share(&self, id: &ContentId) -> Result<Datum, ExecutionJournalError> {
            self.loads.set(self.loads.get().saturating_add(1));
            self.datums
                .borrow()
                .get(id)
                .cloned()
                .ok_or(ExecutionJournalError::MissingObject)
        }

        fn persist_share(&self, datum: Datum) -> Result<ContentId, ExecutionJournalError> {
            let id = datum
                .content_id()
                .map_err(|_| ExecutionJournalError::Semantic("shared state"))?;
            self.datums.borrow_mut().entry(id.clone()).or_insert(datum);
            Ok(id)
        }
    }

    fn cid(byte: u8) -> ContentId {
        ContentId::from_bytes(Symbol::qualified("deck", "sha256-v1"), [byte; 32])
    }

    #[test]
    fn trees_share_unchanged_nodes_and_round_trip() {
        let mem = mem();
        let mut facts = BTreeMap::new();
        for index in 0..32u8 {
            facts.insert(format!("fact/{index:02}"), cid(index));
        }
        let first = sync_facts(&empty_id(), &facts, &mem).unwrap();
        let stored = mem.datums.borrow().len();
        assert_eq!(read_facts(&first, &mem), facts);
        facts.insert("fact/32".into(), cid(40));
        let before = mem.datums.borrow().len();
        let second = sync_facts(&first, &facts, &mem).unwrap();
        assert_eq!(read_facts(&second, &mem), facts);
        assert!(mem.datums.borrow().len() - before < 24);
        assert!(stored > mem.datums.borrow().len() - before);
        let removed = "fact/03".to_owned();
        facts.remove(&removed);
        let third = sync_facts(&second, &facts, &mem).unwrap();
        assert_eq!(read_facts(&third, &mem), facts);
    }

    fn read_facts(root: &ContentId, mem: &Mem) -> BTreeMap<String, ContentId> {
        let datums = mem.datums.borrow().clone();
        load_facts(root, &datums, &stored_objects(&datums)).unwrap()
    }

    #[test]
    fn evidence_inserts_share_the_previous_root() {
        let mem = mem();
        let mut members = Vec::new();
        for byte in 1..16u8 {
            members.push(cid(byte));
        }
        let root = insert_evidence(&empty_id(), &members, &mem).unwrap();
        let before = mem.datums.borrow().len();
        let grown = insert_evidence(&root, &[cid(20)], &mem).unwrap();
        members.push(cid(20));
        let datums = mem.datums.borrow().clone();
        let loaded = load_evidence(&grown, &datums, &stored_objects(&datums)).unwrap();
        assert_eq!(loaded, members.into_iter().collect());
        assert!(mem.datums.borrow().len() - before < 12);
    }

    #[test]
    fn evidence_growth_loads_only_the_changed_path() {
        let mem = mem();
        let mut root = empty_id();
        let mut members = BTreeSet::new();
        let mut total = 0usize;
        for byte in 1..33u8 {
            let next = insert_evidence(&root, &[cid(byte)], &mem).unwrap();
            mem.loads.set(0);
            let datums = mem.datums.borrow().clone();
            let added = tree::evidence_additions(&root, &next, &mem).expect("evidence diff");
            let step = mem.loads.get();
            assert!(step < 48, "byte {byte} loaded {step} nodes");
            assert_eq!(added, vec![cid(byte)]);
            members.insert(cid(byte));
            let loaded = load_evidence(&next, &datums, &stored_objects(&datums)).unwrap();
            assert_eq!(loaded, members);
            root = next;
            total += step;
        }
        assert!(total < 700, "evidence diffs loaded {total} nodes");
        let smaller = insert_evidence(&empty_id(), &[cid(1)], &mem).unwrap();
        let dropped = tree::evidence_additions(&root, &smaller, &mem);
        assert!(matches!(
            dropped,
            Err(ExecutionJournalError::Semantic(
                "evidence set dropped a retained member"
            ))
        ));
    }

    #[test]
    fn altered_share_bytes_fail_the_load() {
        let mem = mem();
        let facts = BTreeMap::from([("fact/01".to_owned(), cid(1))]);
        let root = sync_facts(&empty_id(), &facts, &mem).unwrap();
        let datums = mem.datums.borrow().clone();
        let mut objects = stored_objects(&datums);
        let id = datums
            .keys()
            .find(|id| !is_empty(id))
            .expect("node")
            .clone();
        objects.get_mut(&id).expect("bytes").push(0);
        assert!(matches!(
            load_facts(&root, &datums, &objects),
            Err(ExecutionJournalError::Semantic("shared state"))
        ));
        objects.remove(&id);
        assert!(matches!(
            load_facts(&root, &datums, &objects),
            Err(ExecutionJournalError::MissingObject)
        ));
        mem.loads.set(0);
        assert!(tree::fact_changes(&root, &root, &mem).unwrap().is_empty());
        assert_eq!(mem.loads.get(), 0);
    }
}
