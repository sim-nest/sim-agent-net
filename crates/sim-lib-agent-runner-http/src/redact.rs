pub(crate) fn redact_text(text: &str, secrets: &[&str]) -> String {
    let mut redacted = text.to_owned();
    for secret in secrets {
        if !secret.is_empty() {
            redacted = redacted.replace(secret, "[REDACTED]");
        }
    }
    redacted
}
