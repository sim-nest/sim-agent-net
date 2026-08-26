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
