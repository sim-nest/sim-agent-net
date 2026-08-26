use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex},
    time::Duration,
};

use serde_json::{Map, Value, json};
use sim_cancel::Cancellation;
use sim_kernel::{Cx, Symbol};
use sim_lib_skill::{SkillCard, SkillPolicy, SkillRole};
use sim_shape::{AnyShape, shape_value};

use crate::{
    BindingError, BindingPeer, CacheDisposition, CacheKey, CallContext, ClientCache, ClientError,
    ClientEvent, ClientLedger, Discovery, EndpointIdentity, Era, InputBroker, InputRequest,
    Invocation, Outcome, PeerReply, SchemaContract, Subscription,
};

mod discovery;
use discovery::*;

include!("client/session.rs");
include!("client/callable.rs");
