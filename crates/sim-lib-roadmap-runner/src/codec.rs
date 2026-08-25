use sim_kernel::{ContentId, Symbol};

use crate::{ExecutionJournalError, ExecutionPins, ExecutionRecord, ObjectKind, ObjectRef, RECORD_VERSION};

pub(crate) fn encode(execution: &str, record: &ExecutionRecord) -> Vec<u8> {
    let mut w = Writer(Vec::new());
    w.raw(b"sim-roadmap-execution-record\0"); w.u16(RECORD_VERSION); w.text(execution); w.text(record.tag());
    match record {
        ExecutionRecord::ExecutionOpened { pins, parent } => { w.pins(pins); w.optional_text(parent.as_deref()); }
        ExecutionRecord::StateTransition { from, to } => { w.text(from); w.text(to); }
        ExecutionRecord::EffectRequested { effect_id, kind, input } => { w.text(effect_id); w.text(kind); w.object_ref(input); }
        ExecutionRecord::EffectReceipt { effect_id, outcome, output } => { w.text(effect_id); w.text(outcome); w.object_ref(output); }
        ExecutionRecord::MutationFence { mutation_id, expected } => { w.text(mutation_id); w.text(expected); }
        ExecutionRecord::ProofResult { proof, passed, evidence } => { w.text(proof); w.u8(u8::from(*passed)); w.object_ref(evidence); }
        ExecutionRecord::Discharge { obligation } => w.text(obligation),
        ExecutionRecord::Ambiguity { reason } => w.text(reason),
        ExecutionRecord::TerminalReceipt { outcome } => w.text(outcome),
    }
    w.0
}

pub(crate) fn decode(bytes: &[u8]) -> Result<(String, ExecutionRecord), ExecutionJournalError> {
    let mut r = Reader(bytes);
    if r.take(29)? != b"sim-roadmap-execution-record\0" { return Err(ExecutionJournalError::Codec("magic")); }
    if r.u16()? != RECORD_VERSION { return Err(ExecutionJournalError::Codec("version")); }
    let execution = r.text()?; let tag = r.text()?;
    let record = match tag.as_str() {
        "execution-opened" => ExecutionRecord::ExecutionOpened { pins: r.pins()?, parent: r.optional_text()? },
        "state-transition" => ExecutionRecord::StateTransition { from: r.text()?, to: r.text()? },
        "effect-requested" => ExecutionRecord::EffectRequested { effect_id: r.text()?, kind: r.text()?, input: r.object_ref()? },
        "effect-receipt" => ExecutionRecord::EffectReceipt { effect_id: r.text()?, outcome: r.text()?, output: r.object_ref()? },
        "mutation-fence" => ExecutionRecord::MutationFence { mutation_id: r.text()?, expected: r.text()? },
        "proof-result" => ExecutionRecord::ProofResult { proof: r.text()?, passed: r.u8()? != 0, evidence: r.object_ref()? },
        "discharge" => ExecutionRecord::Discharge { obligation: r.text()? },
        "ambiguity" => ExecutionRecord::Ambiguity { reason: r.text()? },
        "terminal-receipt" => ExecutionRecord::TerminalReceipt { outcome: r.text()? },
        _ => return Err(ExecutionJournalError::Codec("record tag")),
    };
    if !r.0.is_empty() { return Err(ExecutionJournalError::Codec("trailing bytes")); }
    Ok((execution, record))
}

struct Writer(Vec<u8>);
impl Writer {
    fn raw(&mut self, v: &[u8]) { self.0.extend_from_slice(v); } fn u8(&mut self, v: u8) { self.0.push(v); }
    fn u16(&mut self, v: u16) { self.raw(&v.to_be_bytes()); } fn u32(&mut self, v: u32) { self.raw(&v.to_be_bytes()); }
    fn u64(&mut self, v: u64) { self.raw(&v.to_be_bytes()); }
    fn text(&mut self, v: &str) { self.u32(v.len() as u32); self.raw(v.as_bytes()); }
    fn optional_text(&mut self, v: Option<&str>) { self.u8(u8::from(v.is_some())); if let Some(v) = v { self.text(v); } }
    fn id(&mut self, id: &ContentId) { self.text(&id.algorithm.as_qualified_str()); self.raw(&id.bytes); }
    fn pins(&mut self, p: &ExecutionPins) { self.text(&p.conduct); self.text(&p.policy); self.id(&p.source_deck); self.text(&p.model_pick); self.text(&p.runner_generation); }
    fn object_ref(&mut self, v: &Option<ObjectRef>) { self.u8(u8::from(v.is_some())); if let Some(v) = v { self.u8(match v.kind { ObjectKind::Packet => 0, ObjectKind::Deck => 1, ObjectKind::ProcessOutput => 2, ObjectKind::FileBytes => 3 }); self.id(&v.content); self.u64(v.bytes); self.text(&v.summary); } }
}
struct Reader<'a>(&'a [u8]);
impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], ExecutionJournalError> { if self.0.len() < n { return Err(ExecutionJournalError::Codec("truncated")); } let (a,b)=self.0.split_at(n); self.0=b; Ok(a) }
    fn u8(&mut self) -> Result<u8, ExecutionJournalError> { Ok(self.take(1)?[0]) }
    fn u16(&mut self) -> Result<u16, ExecutionJournalError> { Ok(u16::from_be_bytes(self.take(2)?.try_into().expect("length"))) }
    fn u32(&mut self) -> Result<u32, ExecutionJournalError> { Ok(u32::from_be_bytes(self.take(4)?.try_into().expect("length"))) }
    fn u64(&mut self) -> Result<u64, ExecutionJournalError> { Ok(u64::from_be_bytes(self.take(8)?.try_into().expect("length"))) }
    fn text(&mut self) -> Result<String, ExecutionJournalError> { let n=self.u32()? as usize; if n > 64 * 1024 { return Err(ExecutionJournalError::Codec("text bound")); } String::from_utf8(self.take(n)?.to_vec()).map_err(|_| ExecutionJournalError::Codec("utf8")) }
    fn optional_text(&mut self) -> Result<Option<String>, ExecutionJournalError> { match self.u8()? { 0=>Ok(None),1=>Ok(Some(self.text()?)),_=>Err(ExecutionJournalError::Codec("option")) } }
    fn id(&mut self) -> Result<ContentId, ExecutionJournalError> { let text=self.text()?; let symbol=match text.split_once('/') { Some((a,b))=>Symbol::qualified(a,b), None=>Symbol::new(text) }; let bytes=self.take(32)?.try_into().expect("length"); Ok(ContentId::from_bytes(symbol,bytes)) }
    fn pins(&mut self) -> Result<ExecutionPins, ExecutionJournalError> { Ok(ExecutionPins { conduct:self.text()?, policy:self.text()?, source_deck:self.id()?, model_pick:self.text()?, runner_generation:self.text()? }) }
    fn object_ref(&mut self) -> Result<Option<ObjectRef>, ExecutionJournalError> { if self.u8()? == 0 { return Ok(None); } let kind=match self.u8()? {0=>ObjectKind::Packet,1=>ObjectKind::Deck,2=>ObjectKind::ProcessOutput,3=>ObjectKind::FileBytes,_=>return Err(ExecutionJournalError::Codec("object kind"))}; Ok(Some(ObjectRef { kind, content:self.id()?, bytes:self.u64()?, summary:self.text()? })) }
}
