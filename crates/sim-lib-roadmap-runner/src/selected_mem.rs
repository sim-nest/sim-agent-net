// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! In-memory mutation workspace for one selected plan.
//!
//! The bytes stay in the process. Nothing here opens a checkout.

use std::collections::BTreeMap;

use sim_kernel::ContentId;

use super::{
    MutationError, MutationFence, MutationJournal, MutationWorkspace, PortableImage, SealedEntry,
    SealedMutationPlan,
};

pub(super) struct MemWorkspace {
    pub(super) files: BTreeMap<String, PortableImage>,
}

impl MemWorkspace {
    pub(super) fn with(path: &str, bytes: &[u8]) -> Self {
        Self {
            files: BTreeMap::from([(path.to_owned(), PortableImage::file(bytes.to_vec(), 0o644))]),
        }
    }
}

impl MutationWorkspace for MemWorkspace {
    fn observe(&self, path: &str) -> Result<PortableImage, MutationError> {
        Ok(self
            .files
            .get(path)
            .cloned()
            .unwrap_or_else(PortableImage::absent))
    }

    fn apply(
        &mut self,
        entry: &SealedEntry,
        fail: &mut dyn FnMut(crate::Failpoint) -> Result<(), MutationError>,
    ) -> Result<(), MutationError> {
        for point in [
            crate::Failpoint::BeforeTempWrite,
            crate::Failpoint::AfterTempWrite,
            crate::Failpoint::BeforeFlush,
            crate::Failpoint::AfterFlush,
            crate::Failpoint::BeforeReplace,
        ] {
            fail(point)?;
        }
        match &entry.postimage.bytes {
            Some(_) => {
                self.files
                    .insert(entry.path.clone(), entry.postimage.clone());
            }
            None => {
                self.files.remove(&entry.path);
            }
        }
        for point in [
            crate::Failpoint::AfterReplace,
            crate::Failpoint::BeforeDirectorySync,
            crate::Failpoint::AfterDirectorySync,
        ] {
            fail(point)?;
        }
        Ok(())
    }
}

#[derive(Default)]
pub(super) struct MemJournal {
    plans: Vec<ContentId>,
    fences: Vec<MutationFence>,
}

impl MutationJournal for MemJournal {
    fn put_plan(&mut self, plan: &SealedMutationPlan) -> Result<(), MutationError> {
        self.plans.push(plan.id.clone());
        Ok(())
    }

    fn append_fence(&mut self, _: ContentId, fence: MutationFence) -> Result<(), MutationError> {
        self.fences.push(fence);
        Ok(())
    }
}
