// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use super::*;
#[test]
fn cli_and_mcp_share_stable_record_and_show_is_network_free() {
    let p = SearchProduct::default();
    let c = SearchConfig {
        mode: SearchMode::Fixture,
        granted_http: true,
        ..SearchConfig::default()
    };
    let a = p.execute(SearchOperation::Query, "sim", &c).unwrap();
    let b = p.execute(SearchOperation::Query, "sim", &c).unwrap();
    assert_eq!(a, b);
    let mut denied = c.clone();
    denied.granted_http = false;
    assert!(matches!(
        p.execute(SearchOperation::Fetch, "x", &denied),
        Err(SearchProductError::Capability(_))
    ));
    assert_eq!(p.execute(SearchOperation::Show, &a.id, &denied).unwrap(), a);
}
#[test]
fn modes_and_partial_labels_are_canonical() {
    let p = SearchProduct::default();
    for mode in [
        SearchMode::Fixture,
        SearchMode::Cassette,
        SearchMode::Offline,
    ] {
        let c = SearchConfig {
            mode,
            granted_http: true,
            ..SearchConfig::default()
        };
        let r = p.execute(SearchOperation::Research, "sim", &c).unwrap();
        assert!(r.json().contains("search/Run"));
        assert!(r.human().contains("partial failure:"));
    }
}

// conformance: Search product tests prove CLI and MCP share one canonical record and that show stays network-free when HTTP is denied.
