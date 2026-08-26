use super::*;
struct R(u8);
impl SecureRandom for R {
    fn random(&mut self, n: usize) -> Result<Vec<u8>> {
        self.0 += 1;
        Ok(vec![self.0; n])
    }
}
#[test]
fn pkce_state_issuer_and_resource_are_bound() {
    let i = SecureUrl::parse("https://as.example/", false).unwrap();
    let r = SecureUrl::parse("https://mcp.example/", false).unwrap();
    let red = SecureUrl::parse("https://client.example/cb", false).unwrap();
    let (flow, req) = AuthorizationCodeFlow::begin(
        &mut R(0),
        i.clone(),
        r.clone(),
        ScopeSet::parse("read").unwrap(),
        red,
        "client",
    )
    .unwrap();
    assert!(
        req.parameters
            .contains(&("resource".into(), r.as_str().into()))
    );
    let state = req
        .parameters
        .iter()
        .find(|p| p.0 == "state")
        .unwrap()
        .1
        .clone();
    assert!(
        flow.finish(
            AuthorizationResponse {
                code: "c".into(),
                state,
                issuer: Some(i)
            },
            "client"
        )
        .is_ok()
    );
}
#[test]
fn secrets_and_errors_never_echo_material() {
    let s = Secret::new("token-material-47").unwrap();
    assert_eq!(format!("{s:?}"), "<secret>");
    assert!(
        !BearerChallenge::parse("Bearer bad\nmaterial")
            .unwrap_err()
            .to_string()
            .contains("material")
    );
}
#[test]
fn ssrf_and_redirect_policy_fail_closed() {
    assert!(SecureUrl::parse("http://169.254.169.254/latest", false).is_err());
    assert!(SecureUrl::parse("https://u:p@example/x", false).is_err());
    let u = SecureUrl::parse("https://client.example/cb", false).unwrap();
    assert!(validate_redirect(&u, "https://evil.example/cb").is_err());
}
