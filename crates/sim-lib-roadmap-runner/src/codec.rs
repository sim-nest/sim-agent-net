use sim_kernel::{ContentId, Datum, NumberLiteral, Symbol};

use crate::{
    ExecutionJournalError, ExecutionPins, ExecutionRecord, ObjectKind, ObjectRef, RECORD_VERSION,
    SemanticChange,
};

pub(crate) fn encode(execution: &str, record: &ExecutionRecord) -> Vec<u8> {
    let mut w = Writer(Vec::new());
    w.raw(b"sim-roadmap-execution-record\0");
    w.u16(RECORD_VERSION);
    w.text(execution);
    w.text(record.tag());
    match record {
        ExecutionRecord::ExecutionOpened { pins, parent } => {
            w.pins(pins);
            w.optional_text(parent.as_deref());
        }
        ExecutionRecord::StateTransition { from, to } => {
            w.text(from);
            w.text(to);
        }
        ExecutionRecord::EffectRequested {
            effect_id,
            kind,
            input,
        } => {
            w.text(effect_id);
            w.text(kind);
            w.object_ref(input);
        }
        ExecutionRecord::EffectReceipt {
            effect_id,
            outcome,
            output,
        } => {
            w.text(effect_id);
            w.text(outcome);
            w.object_ref(output);
        }
        ExecutionRecord::MutationFence {
            mutation_id,
            expected,
        } => {
            w.text(mutation_id);
            w.text(expected);
        }
        ExecutionRecord::ProofResult {
            proof,
            passed,
            evidence,
        } => {
            w.text(proof);
            w.u8(u8::from(*passed));
            w.object_ref(evidence);
        }
        ExecutionRecord::Discharge { obligation } => w.text(obligation),
        ExecutionRecord::Ambiguity { reason } => w.text(reason),
        ExecutionRecord::TerminalReceipt { outcome } => w.text(outcome),
        ExecutionRecord::SemanticDelta {
            cause,
            changes,
            evidence_set,
        } => {
            w.text(cause);
            w.u32(changes.len() as u32);
            for change in changes {
                w.text(&change.key);
                w.optional_id(change.before.as_ref());
                w.optional_id(change.after.as_ref());
            }
            w.id(evidence_set);
        }
        ExecutionRecord::Snapshot {
            covers,
            state,
            evidence_set,
        } => {
            w.id(covers);
            w.id(state);
            w.id(evidence_set);
        }
    }
    w.0
}

/// Builds the canonical typed payload stored by new journal writers.
pub(crate) fn encode_datum(execution: &str, record: &ExecutionRecord) -> Datum {
    Datum::Node {
        tag: Symbol::qualified("roadmap-execution", "record-v2"),
        fields: vec![
            (
                Symbol::new("version"),
                Datum::Number(NumberLiteral {
                    domain: Symbol::qualified("numbers", "u64"),
                    canonical: RECORD_VERSION.to_string(),
                }),
            ),
            (
                Symbol::new("execution"),
                Datum::String(execution.to_owned()),
            ),
            (
                Symbol::new("kind"),
                Datum::Symbol(Symbol::qualified("roadmap-execution", record.tag())),
            ),
            (Symbol::new("wire"), Datum::Bytes(encode(execution, record))),
        ],
    }
}

/// Checks the typed record Shape and canonical representation. Retained
/// exact-byte objects are accepted only through the v1 compatibility branch.
pub(crate) fn decode_datum(
    value: &Datum,
) -> Result<(String, ExecutionRecord), ExecutionJournalError> {
    if let Some(bytes) = exact_bytes(value) {
        return decode(bytes);
    }
    let Datum::Node { tag, fields } = value else {
        return Err(ExecutionJournalError::Codec("record shape"));
    };
    if *tag != Symbol::qualified("roadmap-execution", "record-v2") || fields.len() != 4 {
        return Err(ExecutionJournalError::Codec("record shape"));
    }
    let field = |name: &str| {
        fields
            .iter()
            .find(|(field, _)| *field == Symbol::new(name))
            .map(|(_, value)| value)
            .ok_or(ExecutionJournalError::Codec("record shape"))
    };
    match field("version")? {
        Datum::Number(number)
            if number.domain == Symbol::qualified("numbers", "u64")
                && number.canonical == RECORD_VERSION.to_string() => {}
        _ => return Err(ExecutionJournalError::Codec("record version shape")),
    }
    let execution = match field("execution")? {
        Datum::String(value) => value,
        _ => return Err(ExecutionJournalError::Codec("record execution shape")),
    };
    let kind = match field("kind")? {
        Datum::Symbol(value) => value,
        _ => return Err(ExecutionJournalError::Codec("record kind shape")),
    };
    let wire = match field("wire")? {
        Datum::Bytes(value) => value,
        _ => return Err(ExecutionJournalError::Codec("record wire shape")),
    };
    let decoded = decode(wire)?;
    if decoded.0 != *execution
        || *kind != Symbol::qualified("roadmap-execution", decoded.1.tag())
        || encode_datum(&decoded.0, &decoded.1) != *value
    {
        return Err(ExecutionJournalError::Codec("non-canonical typed record"));
    }
    Ok(decoded)
}

fn exact_bytes(value: &Datum) -> Option<&[u8]> {
    let Datum::Node { tag, fields } = value else {
        return None;
    };
    if *tag != Symbol::qualified("journal", "exact-bytes-v1") || fields.len() != 1 {
        return None;
    }
    match &fields[0] {
        (name, Datum::Bytes(bytes)) if *name == Symbol::new("bytes") => Some(bytes),
        _ => None,
    }
}

pub(crate) fn decode(bytes: &[u8]) -> Result<(String, ExecutionRecord), ExecutionJournalError> {
    let mut r = Reader(bytes);
    if r.take(29)? != b"sim-roadmap-execution-record\0" {
        return Err(ExecutionJournalError::Codec("magic"));
    }
    let version = r.u16()?;
    if version != 1 && version != RECORD_VERSION {
        return Err(ExecutionJournalError::Codec("version"));
    }
    let execution = r.text()?;
    let tag = r.text()?;
    let record = match tag.as_str() {
        "execution-opened" => ExecutionRecord::ExecutionOpened {
            pins: r.pins()?,
            parent: r.optional_text()?,
        },
        "state-transition" => ExecutionRecord::StateTransition {
            from: r.text()?,
            to: r.text()?,
        },
        "effect-requested" => ExecutionRecord::EffectRequested {
            effect_id: r.text()?,
            kind: r.text()?,
            input: r.object_ref()?,
        },
        "effect-receipt" => ExecutionRecord::EffectReceipt {
            effect_id: r.text()?,
            outcome: r.text()?,
            output: r.object_ref()?,
        },
        "mutation-fence" => ExecutionRecord::MutationFence {
            mutation_id: r.text()?,
            expected: r.text()?,
        },
        "proof-result" => ExecutionRecord::ProofResult {
            proof: r.text()?,
            passed: r.u8()? != 0,
            evidence: r.object_ref()?,
        },
        "discharge" => ExecutionRecord::Discharge {
            obligation: r.text()?,
        },
        "ambiguity" => ExecutionRecord::Ambiguity { reason: r.text()? },
        "terminal-receipt" => ExecutionRecord::TerminalReceipt { outcome: r.text()? },
        "semantic-delta" if version == RECORD_VERSION => {
            let cause = r.text()?;
            let count = r.u32()? as usize;
            if count > 65_536 {
                return Err(ExecutionJournalError::Codec("change bound"));
            }
            let mut changes = Vec::with_capacity(count);
            for _ in 0..count {
                changes.push(SemanticChange {
                    key: r.text()?,
                    before: r.optional_id()?,
                    after: r.optional_id()?,
                });
            }
            ExecutionRecord::SemanticDelta {
                cause,
                changes,
                evidence_set: r.id()?,
            }
        }
        "snapshot" if version == RECORD_VERSION => ExecutionRecord::Snapshot {
            covers: r.id()?,
            state: r.id()?,
            evidence_set: r.id()?,
        },
        _ => return Err(ExecutionJournalError::Codec("record tag")),
    };
    if !r.0.is_empty() {
        return Err(ExecutionJournalError::Codec("trailing bytes"));
    }
    Ok((execution, record))
}

struct Writer(Vec<u8>);
impl Writer {
    fn raw(&mut self, v: &[u8]) {
        self.0.extend_from_slice(v);
    }
    fn u8(&mut self, v: u8) {
        self.0.push(v);
    }
    fn u16(&mut self, v: u16) {
        self.raw(&v.to_be_bytes());
    }
    fn u32(&mut self, v: u32) {
        self.raw(&v.to_be_bytes());
    }
    fn u64(&mut self, v: u64) {
        self.raw(&v.to_be_bytes());
    }
    fn text(&mut self, v: &str) {
        self.u32(v.len() as u32);
        self.raw(v.as_bytes());
    }
    fn optional_text(&mut self, v: Option<&str>) {
        self.u8(u8::from(v.is_some()));
        if let Some(v) = v {
            self.text(v);
        }
    }
    fn id(&mut self, id: &ContentId) {
        self.text(&id.algorithm.as_qualified_str());
        self.raw(&id.bytes);
    }
    fn optional_id(&mut self, id: Option<&ContentId>) {
        self.u8(u8::from(id.is_some()));
        if let Some(id) = id {
            self.id(id);
        }
    }
    fn pins(&mut self, p: &ExecutionPins) {
        self.text(&p.conduct);
        self.text(&p.policy);
        self.id(&p.source_deck);
        self.text(&p.model_pick);
        self.text(&p.runner_generation);
    }
    fn object_ref(&mut self, v: &Option<ObjectRef>) {
        self.u8(u8::from(v.is_some()));
        if let Some(v) = v {
            self.u8(match v.kind {
                ObjectKind::Packet => 0,
                ObjectKind::Deck => 1,
                ObjectKind::ProcessOutput => 2,
                ObjectKind::FileBytes => 3,
                ObjectKind::EvidenceSet => 4,
                ObjectKind::Snapshot => 5,
            });
            self.id(&v.content);
            self.u64(v.bytes);
            self.text(&v.summary);
        }
    }
}
struct Reader<'a>(&'a [u8]);
impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], ExecutionJournalError> {
        if self.0.len() < n {
            return Err(ExecutionJournalError::Codec("truncated"));
        }
        let (a, b) = self.0.split_at(n);
        self.0 = b;
        Ok(a)
    }
    fn u8(&mut self) -> Result<u8, ExecutionJournalError> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16, ExecutionJournalError> {
        Ok(u16::from_be_bytes(
            self.take(2)?.try_into().expect("length"),
        ))
    }
    fn u32(&mut self) -> Result<u32, ExecutionJournalError> {
        Ok(u32::from_be_bytes(
            self.take(4)?.try_into().expect("length"),
        ))
    }
    fn u64(&mut self) -> Result<u64, ExecutionJournalError> {
        Ok(u64::from_be_bytes(
            self.take(8)?.try_into().expect("length"),
        ))
    }
    fn text(&mut self) -> Result<String, ExecutionJournalError> {
        let n = self.u32()? as usize;
        if n > 64 * 1024 {
            return Err(ExecutionJournalError::Codec("text bound"));
        }
        String::from_utf8(self.take(n)?.to_vec()).map_err(|_| ExecutionJournalError::Codec("utf8"))
    }
    fn optional_text(&mut self) -> Result<Option<String>, ExecutionJournalError> {
        match self.u8()? {
            0 => Ok(None),
            1 => Ok(Some(self.text()?)),
            _ => Err(ExecutionJournalError::Codec("option")),
        }
    }
    fn id(&mut self) -> Result<ContentId, ExecutionJournalError> {
        let text = self.text()?;
        let symbol = match text.split_once('/') {
            Some((a, b)) => Symbol::qualified(a, b),
            None => Symbol::new(text),
        };
        let bytes = self.take(32)?.try_into().expect("length");
        Ok(ContentId::from_bytes(symbol, bytes))
    }
    fn optional_id(&mut self) -> Result<Option<ContentId>, ExecutionJournalError> {
        match self.u8()? {
            0 => Ok(None),
            1 => Ok(Some(self.id()?)),
            _ => Err(ExecutionJournalError::Codec("optional id")),
        }
    }
    fn pins(&mut self) -> Result<ExecutionPins, ExecutionJournalError> {
        Ok(ExecutionPins {
            conduct: self.text()?,
            policy: self.text()?,
            source_deck: self.id()?,
            model_pick: self.text()?,
            runner_generation: self.text()?,
        })
    }
    fn object_ref(&mut self) -> Result<Option<ObjectRef>, ExecutionJournalError> {
        if self.u8()? == 0 {
            return Ok(None);
        }
        let kind = match self.u8()? {
            0 => ObjectKind::Packet,
            1 => ObjectKind::Deck,
            2 => ObjectKind::ProcessOutput,
            3 => ObjectKind::FileBytes,
            4 => ObjectKind::EvidenceSet,
            5 => ObjectKind::Snapshot,
            _ => return Err(ExecutionJournalError::Codec("object kind")),
        };
        Ok(Some(ObjectRef {
            kind,
            content: self.id()?,
            bytes: self.u64()?,
            summary: self.text()?,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record() -> ExecutionRecord {
        ExecutionRecord::Discharge {
            obligation: "verify canonical typed payload".into(),
        }
    }

    #[test]
    fn typed_record_shape_and_semantic_identity_are_exact() {
        let value = encode_datum("execution-1", &record());
        let identity = value.content_id().unwrap();
        assert_eq!(
            decode_datum(&value).unwrap(),
            ("execution-1".into(), record())
        );
        assert_eq!(value.content_id().unwrap(), identity);

        let mut wrong_shape = value;
        let Datum::Node { fields, .. } = &mut wrong_shape else {
            unreachable!()
        };
        fields[2].1 = Datum::Symbol(Symbol::qualified("roadmap-execution", "snapshot"));
        assert!(decode_datum(&wrong_shape).is_err());
    }

    #[test]
    fn retained_version_one_wire_record_decodes_only_through_exact_bytes() {
        let mut bytes = encode("execution-1", &record());
        bytes[29..31].copy_from_slice(&1_u16.to_be_bytes());
        assert_eq!(decode(&bytes).unwrap(), ("execution-1".into(), record()));
        let legacy = Datum::Node {
            tag: Symbol::qualified("journal", "exact-bytes-v1"),
            fields: vec![(Symbol::new("bytes"), Datum::Bytes(bytes))],
        };
        assert_eq!(
            decode_datum(&legacy).unwrap(),
            ("execution-1".into(), record())
        );
    }
}
