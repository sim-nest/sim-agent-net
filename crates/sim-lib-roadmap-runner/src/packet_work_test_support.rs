// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Shared fixtures for packet receiver tests.

use std::cell::{Cell, RefCell};

use sim_conformance_packet::FakeInputPort;

use super::{
    CanonicalWork, ModelBody, ModelReturn, ProposalModel, WorkReceipt, WorkSource, receive_work,
};
use crate::{ModelWorker, ResourceAsk, WorkerView};

pub(super) fn source(body: &[u8]) -> WorkSource {
    WorkSource {
        surface: "api/local-adapter".into(),
        path: "crates/widget/src/lib.rs".into(),
        body: body.to_vec(),
    }
}

pub(super) fn work(body: &[u8]) -> CanonicalWork {
    CanonicalWork {
        namespace: "work".into(),
        phase: "work.1".into(),
        law: "law/packet".into(),
        owner_package: "sim-lib-roadmap-runner".into(),
        sources: vec![source(body)],
        source_role: "released-dependency".into(),
        target_surface: "api/projection".into(),
        target_role: "funded-target".into(),
        target_status: "declared".into(),
        budget_bytes: 64,
        budget_files: 2,
        budget_tokens: 64,
        budget_output: 64,
        resources: vec![ResourceAsk {
            kind: "slot".into(),
            amount: 1,
        }],
        copy: "gen-2".into(),
        current: "gen-2".into(),
        schema: "shape/proposal".into(),
        mirrors_agree: true,
        max_attempts: 2,
        unresolved: 4,
        narrowed_to: 3,
    }
}

pub(super) fn preload(input: &CanonicalWork) -> FakeInputPort {
    let mut port = FakeInputPort::default();
    for source in &input.sources {
        port.insert(source.body.clone());
    }
    port
}

pub(super) fn proposal(serialized: &[u8], value: &str) -> ModelReturn {
    ModelReturn {
        seen: serialized.to_vec(),
        body: ModelBody::Proposal(format!("proposal/v1\nvalue {value}").into_bytes()),
        unresolved: 3,
    }
}

pub(super) struct Views {
    pub(super) views: Vec<WorkerView>,
    pub(super) calls: Cell<usize>,
}

impl ModelWorker for Views {
    fn outcome(&self, id: &str) -> WorkerView {
        assert_eq!(id, "work/api/projection");
        let index = self.calls.get();
        self.calls.set(index + 1);
        self.views[index]
    }
}

pub(super) struct Returns {
    pub(super) items: RefCell<Vec<ModelReturn>>,
    pub(super) calls: Cell<u32>,
}

impl ProposalModel for Returns {
    fn propose(&self, receiver: &str, attempt: u32, serialized: &[u8]) -> ModelReturn {
        assert_eq!(receiver, "work/api/projection");
        let call = self.calls.get() + 1;
        self.calls.set(call);
        assert_eq!(attempt, call);
        let mut items = self.items.borrow_mut();
        let mut item = items.remove(0);
        if item.seen.is_empty() {
            item.seen = serialized.to_vec();
        }
        item
    }
}

pub(super) struct PanicWorker;

impl ModelWorker for PanicWorker {
    fn outcome(&self, _: &str) -> WorkerView {
        panic!("worker called");
    }
}

pub(super) struct PanicProposals;

impl ProposalModel for PanicProposals {
    fn propose(&self, _: &str, _: u32, _: &[u8]) -> ModelReturn {
        panic!("proposal called");
    }
}

pub(super) fn admitted(body: &[u8]) -> (WorkReceipt, usize) {
    let input = work(body);
    let mut port = preload(&input);
    let worker = Views {
        views: vec![WorkerView::Passed],
        calls: Cell::new(0),
    };
    let proposals = Returns {
        items: RefCell::new(vec![proposal(b"", "mapped")]),
        calls: Cell::new(0),
    };
    let receipt = receive_work(&input, &mut port, &worker, &proposals).unwrap();
    assert_eq!(port.reads().len(), 1);
    (receipt, worker.calls.get())
}
