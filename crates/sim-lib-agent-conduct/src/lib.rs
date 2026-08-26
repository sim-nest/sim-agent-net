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
mod tests;
