// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use std::cell::{Cell, RefCell};

use super::support::{PanicProposals, PanicWorker, Returns, Views, admitted, preload, work};
use super::{ModelBody, ModelReturn, WorkReceipt, WorkReceiveError, receive_work};
use crate::{ResourceAsk, WorkerView};

#[test]
fn equal_inputs_share_one_packet_identity() {
    let (first, calls) = admitted(b"exact source");
    let (second, _) = admitted(b"exact source");
    let (other, _) = admitted(b"other source");
    let WorkReceipt::Admitted {
        work,
        proposal,
        attempts,
    } = first
    else {
        panic!("expected admission");
    };
    let WorkReceipt::Admitted { work: again, .. } = second else {
        panic!("expected admission");
    };
    let WorkReceipt::Admitted { work: changed, .. } = other else {
        panic!("expected admission");
    };
    assert_eq!(calls, 1);
    assert_eq!(attempts, 1);
    assert_eq!(proposal, "mapped");
    assert_eq!(work.packet_id, again.packet_id);
    assert_ne!(work.packet_id, changed.packet_id);
    assert_eq!(work.bytes, b"exact source".len() as u64);
    assert_eq!(work.files, 1);
    assert_eq!(work.tokens, work.bytes);
    assert_eq!(work.paths, vec!["crates/widget/src/lib.rs".to_owned()]);
    assert_eq!(work.role, "funded-target");
    assert_eq!(work.status, "declared");
    assert_eq!(work.receiver, "work/api/projection");
    assert_eq!(
        work.resources,
        vec![ResourceAsk {
            kind: "slot".into(),
            amount: 1
        }]
    );
    assert!(!work.serialized.is_empty());
}

#[test]
fn oversize_input_refuses_before_capture() {
    let mut input = work(b"source larger than budget");
    input.budget_bytes = 3;
    let mut port = preload(&input);
    let error = receive_work(&input, &mut port, &PanicWorker, &PanicProposals).unwrap_err();
    assert_eq!(error, WorkReceiveError::Oversize);
    assert!(port.reads().is_empty());
}

#[test]
fn token_and_file_ceilings_refuse_before_capture() {
    let mut tokens = work(b"abcd");
    tokens.budget_tokens = 3;
    let mut port = preload(&tokens);
    assert_eq!(
        receive_work(&tokens, &mut port, &PanicWorker, &PanicProposals).unwrap_err(),
        WorkReceiveError::Oversize
    );
    assert!(port.reads().is_empty());

    let mut files = work(b"abcd");
    files.budget_files = 0;
    let mut port = preload(&files);
    assert_eq!(
        receive_work(&files, &mut port, &PanicWorker, &PanicProposals).unwrap_err(),
        WorkReceiveError::Oversize
    );
    assert!(port.reads().is_empty());
}

#[test]
fn wrong_namespace_role_copy_status_and_schema_fail_when_mirrors_agree() {
    let cases = [
        (
            "namespace",
            "mirror",
            WorkReceiveError::Namespace {
                mirrors_agree: true,
            },
        ),
        (
            "source_role",
            "owner",
            WorkReceiveError::Role {
                mirrors_agree: true,
            },
        ),
        (
            "target_role",
            "released-dependency",
            WorkReceiveError::Role {
                mirrors_agree: true,
            },
        ),
        (
            "copy",
            "gen-1",
            WorkReceiveError::StaleCopy {
                mirrors_agree: true,
            },
        ),
        (
            "target_status",
            "existing",
            WorkReceiveError::Status {
                mirrors_agree: true,
            },
        ),
        (
            "schema",
            "shape/other",
            WorkReceiveError::Schema {
                mirrors_agree: true,
            },
        ),
    ];
    for (field, value, expected) in cases {
        let mut input = work(b"exact source");
        assert!(input.mirrors_agree);
        match field {
            "namespace" => input.namespace = value.into(),
            "source_role" => input.source_role = value.into(),
            "target_role" => input.target_role = value.into(),
            "copy" => input.copy = value.into(),
            "target_status" => input.target_status = value.into(),
            "schema" => input.schema = value.into(),
            _ => unreachable!(),
        }
        let mut port = preload(&input);
        assert_eq!(
            receive_work(&input, &mut port, &PanicWorker, &PanicProposals).unwrap_err(),
            expected
        );
        assert!(port.reads().is_empty());
    }
}

#[test]
fn initial_descent_refuses_before_capture() {
    let mut input = work(b"exact source");
    input.narrowed_to = 4;
    let mut port = preload(&input);
    assert_eq!(
        receive_work(&input, &mut port, &PanicWorker, &PanicProposals).unwrap_err(),
        WorkReceiveError::NoDescent {
            evidence: Vec::new()
        }
    );
    assert!(port.reads().is_empty());
}

#[test]
fn escaping_path_refuses_before_capture() {
    let mut input = work(b"exact source");
    input.sources[0].path = "../secret".into();
    let mut port = preload(&input);
    assert!(matches!(
        receive_work(&input, &mut port, &PanicWorker, &PanicProposals).unwrap_err(),
        WorkReceiveError::Schema {
            mirrors_agree: true
        }
    ));
    assert!(port.reads().is_empty());
}

#[test]
fn incomplete_output_retains_packet_evidence() {
    let input = work(b"exact source");
    let mut port = preload(&input);
    let worker = Views {
        views: vec![WorkerView::Blocked],
        calls: Cell::new(0),
    };
    let receipt = receive_work(&input, &mut port, &worker, &PanicProposals).unwrap();
    let WorkReceipt::Incomplete {
        work,
        partial,
        attempts,
    } = receipt
    else {
        panic!("expected incomplete");
    };
    assert_eq!(attempts, 1);
    assert!(partial.is_empty());
    assert_eq!(work.bytes, b"exact source".len() as u64);
    assert_eq!(work.receiver, "work/api/projection");
    assert!(!work.packet_id.is_empty());
    assert!(!work.serialized.is_empty());
    assert_eq!(port.reads().len(), 1);

    let worker = Views {
        views: vec![WorkerView::Passed],
        calls: Cell::new(0),
    };
    let proposals = Returns {
        items: RefCell::new(vec![ModelReturn {
            seen: Vec::new(),
            body: ModelBody::Incomplete(b"partial-map".to_vec()),
            unresolved: 3,
        }]),
        calls: Cell::new(0),
    };
    let mut port = preload(&input);
    let receipt = receive_work(&input, &mut port, &worker, &proposals).unwrap();
    let WorkReceipt::Incomplete {
        work: again,
        partial,
        attempts,
    } = receipt
    else {
        panic!("expected incomplete");
    };
    assert_eq!(attempts, 1);
    assert_eq!(partial, b"partial-map");
    assert_eq!(again.packet_id, work.packet_id);
    assert_eq!(again.serialized, work.serialized);
}
