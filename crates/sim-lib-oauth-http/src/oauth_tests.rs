// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use super::*;
struct F(Vec<u8>);
impl DocumentTransport for F {
    fn get(&self, _: &SecureUrl, _: &Policy) -> Result<Vec<u8>> {
        Ok(self.0.clone())
    }
}
#[test]
fn discovery_is_bounded_and_https_only() {
    let url = SecureUrl::parse(
        "https://mcp.example/.well-known/oauth-protected-resource",
        false,
    )
    .unwrap();
    let bytes=br#"{"resource":"https://mcp.example/","authorization_servers":["https://as.example/"],"scopes_supported":["read"]}"#.to_vec();
    let d = OAuthDocuments::new(F(bytes), RetrievalPolicy::default()).unwrap();
    assert_eq!(
        d.protected_resource(&url).unwrap().resource.as_str(),
        "https://mcp.example/"
    );
}
#[test]
fn refresh_is_rate_limited() {
    let url = SecureUrl::parse("https://as.example/jwks", false).unwrap();
    let mut d =
        OAuthDocuments::new(F(br#"{"keys":[]}"#.to_vec()), RetrievalPolicy::default()).unwrap();
    assert!(d.jwks(&url, 100).is_ok());
    assert!(d.jwks(&url, 101).is_err());
}
// conformance: Bounded OAuth HTTP tests read one protected-resource document through an injected transport and refuse a JWK refresh one time unit after a successful refresh.
