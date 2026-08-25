use std::sync::Arc;

use sim_kernel::{ContentId, Symbol};
use sim_lib_journal::{JournalError, MemoryBackend};

use crate::*;

fn pins(n:u8)->ExecutionPins{ExecutionPins{conduct:format!("conduct-{n}"),policy:format!("policy-{n}"),source_deck:ContentId::from_bytes(Symbol::qualified("deck","sha256-v1"),[n;32]),model_pick:format!("model-{n}"),runner_generation:format!("runner-{n}")}}

#[test]
fn complete_record_family_replays_exactly_without_effects(){
    let backend=Arc::new(MemoryBackend::new()); let log=ExecutionJournal::new(backend,"exec",Limits::default());
    let mut state=log.open(pins(1),None).unwrap();
    let packet=log.prepare_object(ObjectKind::Packet,b"bounded packet".to_vec(),"packet summary").unwrap();
    state.head=log.append(Some(&state.head),ExecutionRecord::StateTransition{from:"planned".into(),to:"running".into()},vec![]).unwrap();
    state.head=log.append(Some(&state.head),ExecutionRecord::EffectRequested{effect_id:"effect-1".into(),kind:"process".into(),input:Some(packet.reference.clone())},vec![packet]).unwrap();
    let output=log.prepare_object(ObjectKind::ProcessOutput,b"ok".to_vec(),"exit zero").unwrap();
    state.head=log.append(Some(&state.head),ExecutionRecord::EffectReceipt{effect_id:"effect-1".into(),outcome:"ok".into(),output:Some(output.reference.clone())},vec![output]).unwrap();
    for record in [ExecutionRecord::MutationFence{mutation_id:"m1".into(),expected:"preimage".into()},ExecutionRecord::ProofResult{proof:"tests".into(),passed:true,evidence:None},ExecutionRecord::Discharge{obligation:"tests".into()},ExecutionRecord::Ambiguity{reason:"none".into()},ExecutionRecord::StateTransition{from:"running".into(),to:"reconciling".into()},ExecutionRecord::StateTransition{from:"reconciling".into(),to:"succeeded".into()},ExecutionRecord::TerminalReceipt{outcome:"succeeded".into()}] { state.head=log.append(Some(&state.head),record,vec![]).unwrap(); }
    let left=log.rebuild().unwrap(); let right=log.rebuild().unwrap(); assert_eq!(left,right); assert_eq!(left.records.len(),11);
}

#[test]
fn fences_duplicates_redaction_budgets_and_identity_changes_fail_closed(){
    let backend=Arc::new(MemoryBackend::new()); let log=ExecutionJournal::new(backend,"exec",Limits{max_object_bytes:8,..Limits::default()});
    let opened=log.open(pins(1),None).unwrap();
    assert!(matches!(log.prepare_object(ObjectKind::Packet,b"secret=oops".to_vec(),"packet"),Err(ExecutionJournalError::Budget("object"))));
    assert!(matches!(log.prepare_object(ObjectKind::Packet,b"password=x".to_vec(),"packet"),Err(ExecutionJournalError::Budget("object"))));
    let head=log.append(Some(&opened.head),ExecutionRecord::EffectRequested{effect_id:"x".into(),kind:"write".into(),input:None},vec![]).unwrap();
    assert!(matches!(log.append(Some(&opened.head),ExecutionRecord::Ambiguity{reason:"stale".into()},vec![]),Err(ExecutionJournalError::Journal(JournalError::WrongHead|JournalError::ConflictingDelivery))));
    let receipt=ExecutionRecord::EffectReceipt{effect_id:"x".into(),outcome:"ok".into(),output:None}; let head=log.append(Some(&head),receipt.clone(),vec![]).unwrap();
    assert!(matches!(log.append(Some(&head),receipt,vec![]),Err(ExecutionJournalError::Illegal{..})));
    assert!(matches!(log.open(pins(2),Some(&head)),Err(ExecutionJournalError::ChildRequired{..})));
}

#[test]
fn secret_shaped_environment_and_packet_data_never_reach_objects(){
    let log=ExecutionJournal::new(Arc::new(MemoryBackend::new()),"exec",Limits::default());
    for bytes in [b"API_KEY=abc".as_slice(),b"Authorization: Bearer abc".as_slice(),b"-----PRIVATE KEY-----".as_slice()] { assert!(matches!(log.prepare_object(ObjectKind::Packet,bytes.to_vec(),"safe"),Err(ExecutionJournalError::Secret))); }
    assert!(matches!(log.prepare_object(ObjectKind::FileBytes,b"safe".to_vec(),"password=hunter2"),Err(ExecutionJournalError::Secret)));
}
