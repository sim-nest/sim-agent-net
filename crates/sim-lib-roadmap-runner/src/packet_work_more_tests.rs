// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// conformance: one work packet is admitted from canonical input.

use std::cell::{Cell, RefCell};

use super::support::{PanicProposals, PanicWorker, Returns, Views, preload, work};
use super::{ModelBody, ModelReturn, WorkReceipt, WorkReceiveError, receive_work};
use crate::WorkerView;

#[test]
fn malformed_retries_stop_at_the_attempt_ceiling() {
    let input = work(b"exact source");
    let mut port = preload(&input);
    let worker = Views {
        views: vec![WorkerView::Passed, WorkerView::Passed, WorkerView::Passed],
        calls: Cell::new(0),
    };
    let proposals = Returns {
        items: RefCell::new(vec![
            ModelReturn {
                seen: Vec::new(),
                body: ModelBody::Malformed(b"first".to_vec()),
                unresolved: 2,
            },
            ModelReturn {
                seen: Vec::new(),
                body: ModelBody::Malformed(b"second".to_vec()),
                unresolved: 1,
            },
            ModelReturn {
                seen: Vec::new(),
                body: ModelBody::Proposal(b"proposal/v1\nvalue later".to_vec()),
                unresolved: 0,
            },
        ]),
        calls: Cell::new(0),
    };
    let error = receive_work(&input, &mut port, &worker, &proposals).unwrap_err();
    assert_eq!(
        error,
        WorkReceiveError::MalformedBound {
            attempts: 2,
            evidence: b"second".to_vec(),
        }
    );
    assert_eq!(proposals.calls.get(), 2);
    assert_eq!(worker.calls.get(), 2);
}

#[test]
fn retry_without_descent_refuses() {
    let input = work(b"exact source");
    let mut port = preload(&input);
    let worker = Views {
        views: vec![WorkerView::Passed, WorkerView::Passed],
        calls: Cell::new(0),
    };
    let proposals = Returns {
        items: RefCell::new(vec![ModelReturn {
            seen: Vec::new(),
            body: ModelBody::Malformed(b"same".to_vec()),
            unresolved: 3,
        }]),
        calls: Cell::new(0),
    };
    let error = receive_work(&input, &mut port, &worker, &proposals).unwrap_err();
    assert_eq!(
        error,
        WorkReceiveError::NoDescent {
            evidence: b"same".to_vec(),
        }
    );
    assert_eq!(proposals.calls.get(), 1);
    assert_eq!(worker.calls.get(), 1);
}

#[test]
fn a_narrowing_retry_can_still_admit() {
    let input = work(b"exact source");
    let mut port = preload(&input);
    let worker = Views {
        views: vec![WorkerView::Passed, WorkerView::Passed],
        calls: Cell::new(0),
    };
    let proposals = Returns {
        items: RefCell::new(vec![
            ModelReturn {
                seen: Vec::new(),
                body: ModelBody::Malformed(b"noise".to_vec()),
                unresolved: 2,
            },
            ModelReturn {
                seen: Vec::new(),
                body: ModelBody::Proposal(b"proposal/v1\nvalue mapped".to_vec()),
                unresolved: 1,
            },
        ]),
        calls: Cell::new(0),
    };
    let receipt = receive_work(&input, &mut port, &worker, &proposals).unwrap();
    let WorkReceipt::Admitted {
        proposal, attempts, ..
    } = receipt
    else {
        panic!("expected admission");
    };
    assert_eq!(proposal, "mapped");
    assert_eq!(attempts, 2);
    assert_eq!(proposals.calls.get(), 2);
}

#[test]
fn effect_and_approval_proposals_refuse_without_retry() {
    for body in [
        "proposal/v1\neffect host",
        "proposal/v1\napproval granted",
        "proposal/v1\nvalue effect",
    ] {
        let input = work(b"exact source");
        let mut port = preload(&input);
        let worker = Views {
            views: vec![WorkerView::Passed, WorkerView::Passed],
            calls: Cell::new(0),
        };
        let proposals = Returns {
            items: RefCell::new(vec![ModelReturn {
                seen: Vec::new(),
                body: ModelBody::Proposal(body.as_bytes().to_vec()),
                unresolved: 1,
            }]),
            calls: Cell::new(0),
        };
        assert_eq!(
            receive_work(&input, &mut port, &worker, &proposals).unwrap_err(),
            WorkReceiveError::EffectsOrApproval
        );
        assert_eq!(proposals.calls.get(), 1);
    }
}

#[test]
fn altered_receiver_serialization_fails_when_mirrors_agree() {
    let input = work(b"exact source");
    let mut port = preload(&input);
    let worker = Views {
        views: vec![WorkerView::Passed],
        calls: Cell::new(0),
    };
    let proposals = Returns {
        items: RefCell::new(vec![ModelReturn {
            seen: b"not-the-packet".to_vec(),
            body: ModelBody::Proposal(b"proposal/v1\nvalue mapped".to_vec()),
            unresolved: 2,
        }]),
        calls: Cell::new(0),
    };
    assert_eq!(
        receive_work(&input, &mut port, &worker, &proposals).unwrap_err(),
        WorkReceiveError::Schema {
            mirrors_agree: true
        }
    );
    assert_eq!(port.reads().len(), 1);
    assert_eq!(proposals.calls.get(), 1);
}

#[test]
fn resource_named_as_approval_refuses_before_capture() {
    for kind in ["approval", "effect"] {
        let mut input = work(b"exact source");
        input.resources[0].kind = kind.into();
        let mut port = preload(&input);
        assert_eq!(
            receive_work(&input, &mut port, &PanicWorker, &PanicProposals).unwrap_err(),
            WorkReceiveError::EffectsOrApproval
        );
        assert!(port.reads().is_empty());
    }
}

#[test]
fn invalid_resource_name_is_schema_before_capture() {
    for kind in ["", "../slot"] {
        let mut input = work(b"exact source");
        assert!(input.mirrors_agree);
        input.resources[0].kind = kind.into();
        let mut port = preload(&input);
        assert_eq!(
            receive_work(&input, &mut port, &PanicWorker, &PanicProposals).unwrap_err(),
            WorkReceiveError::Schema {
                mirrors_agree: true
            }
        );
        assert!(port.reads().is_empty());
    }
}
