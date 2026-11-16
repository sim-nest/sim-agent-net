// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

#![forbid(unsafe_code)]
#![deny(missing_docs)]
//! Agent-conduct certification and topology execution adapter.

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use sim_kernel::{
    CapabilityName, Cx, DefaultFactory, Error, Expr, NoopEvalPolicy, Result, Symbol, Value,
};
use sim_lib_agent_conduct_core::{AgentConductContract, AgentStepCard};
use sim_lib_topology::{
    CompiledGraph, NodeId, TopologyBindingDescriptor, TopologyBindings, TopologyContinuation,
    TopologyEntry, TopologyPackage, TopologyPackageSource, TopologyProgress, TopologyRegistry,
    TopologyRunReport, compile_graph, parse_package, topology_reflect, topology_reflect_graph,
};

mod validation;
use validation::*;

include!("catalog.rs");
include!("conduct.rs");
include!("runtime.rs");

#[cfg(test)]
mod catalog_tests;
#[cfg(test)]
mod tests;
