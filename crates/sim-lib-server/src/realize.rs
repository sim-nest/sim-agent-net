use sim_kernel::{Cx, Result};
use sim_lib_stream_core::{PlacedFragment, StreamEnvelope};

use crate::Site;

/// Realizes a placed fragment on `site`, returning the resulting stream
/// envelopes for its output edges.
pub fn realize(
    cx: &mut Cx,
    fragment: PlacedFragment,
    site: &dyn Site,
) -> Result<Vec<StreamEnvelope>> {
    site.run_fragment(cx, &fragment)
}

/// Realizes `fragment` on `site` and returns its stream events; an alias for
/// [`realize`] named for the streaming call site.
pub fn realize_stream_events(
    cx: &mut Cx,
    fragment: PlacedFragment,
    site: &dyn Site,
) -> Result<Vec<StreamEnvelope>> {
    realize(cx, fragment, site)
}
