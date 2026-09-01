#[path = "../../src/compatibility.rs"]
#[allow(dead_code)]
mod compatibility;

#[test]
fn public_shadow_contract_is_linkable_without_runner_authority() {
    assert_eq!(compatibility::SHADOW_DIMENSIONS.len(), 10);
    assert_eq!(
        compatibility::CompatibilityClass::Unsupported.as_str(),
        "unsupported"
    );
    assert_eq!(compatibility::CompatibilityClass::Failed.as_str(), "failed");
    assert_eq!(
        compatibility::CompatibilityClass::IntentionallyChanged.as_str(),
        "intentionally-changed"
    );
}
