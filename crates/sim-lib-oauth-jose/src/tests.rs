use super::*;
use ring::rand::SystemRandom;
use ring::signature::{Ed25519KeyPair, KeyPair};
fn token(key: &Ed25519KeyPair, kid: &str, exp: u64, resource: &str) -> Secret {
    let h = URL_SAFE_NO_PAD.encode(format!(r#"{{"alg":"EdDSA","kid":"{kid}","typ":"at+jwt"}}"#));
    let c=URL_SAFE_NO_PAD.encode(format!(r#"{{"iss":"https://as.example/","sub":"alice","aud":"{resource}","resource":"{resource}","scope":"read write","exp":{exp},"jti":"one"}}"#));
    let m = format!("{h}.{c}");
    let s = URL_SAFE_NO_PAD.encode(key.sign(m.as_bytes()).as_ref());
    Secret::new(format!("{m}.{s}")).unwrap()
}
fn fixture() -> (LocalJwtVerifier, Ed25519KeyPair) {
    let doc = Ed25519KeyPair::generate_pkcs8(&SystemRandom::new()).unwrap();
    let key = Ed25519KeyPair::from_pkcs8(doc.as_ref()).unwrap();
    let x = URL_SAFE_NO_PAD.encode(key.public_key().as_ref());
    let json=format!(r#"{{"keys":[{{"kty":"OKP","crv":"Ed25519","kid":"one","alg":"EdDSA","use":"sig","x":"{x}"}}]}}"#).into_bytes();
    (
        LocalJwtVerifier::new(
            KeyDocument {
                generation: 1,
                json,
            },
            [Algorithm::EdDsa],
            30,
        )
        .unwrap(),
        key,
    )
}
#[test]
fn verifies_bound_token_and_rejects_substitutions() {
    let (v, k) = fixture();
    let i = SecureUrl::parse("https://as.example/", false).unwrap();
    let r = SecureUrl::parse("https://mcp.example/", false).unwrap();
    let s = ScopeSet::parse("read").unwrap();
    assert!(
        v.verify(&token(&k, "one", 200, r.as_str()), &i, &r, &s, 100)
            .is_ok()
    );
    let other = SecureUrl::parse("https://other.example/", false).unwrap();
    assert!(
        v.verify(&token(&k, "one", 200, r.as_str()), &i, &other, &s, 100)
            .is_err()
    );
    assert!(
        v.verify(&token(&k, "missing", 200, r.as_str()), &i, &r, &s, 100)
            .is_err()
    );
    assert!(
        v.verify(&token(&k, "one", 1, r.as_str()), &i, &r, &s, 100)
            .is_err()
    );
}
#[test]
fn rotation_is_monotonic() {
    let (mut v, _) = fixture();
    assert!(
        v.rotate(KeyDocument {
            generation: 1,
            json: b"{}".to_vec()
        })
        .is_err()
    );
}
