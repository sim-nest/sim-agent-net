use sim_kernel::{Error, Result};
use std::fmt;

/// Opaque provider credential material.
///
/// Printable representations are always redacted. Callers must opt in to the
/// narrowly named [`Secret::expose`] method at the transport boundary.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    /// Admits non-empty, single-line credential material.
    pub fn new(material: impl Into<String>) -> Result<Self> {
        let material = material.into();
        if material.is_empty() {
            return Err(Error::Eval(
                "secret provider returned empty material".to_owned(),
            ));
        }
        if material.chars().any(char::is_control) {
            return Err(Error::Eval(
                "secret provider returned malformed material".to_owned(),
            ));
        }
        Ok(Self(material))
    }

    /// Exposes material only for immediate use at an authenticated transport.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<secret>")
    }
}

impl fmt::Display for Secret {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<secret>")
    }
}

#[cfg(test)]
mod tests {
    use super::Secret;

    #[test]
    fn every_printable_face_is_fully_redacted() {
        let material = "seat-specific-material-47";
        let secret = Secret::new(material).unwrap();
        for printed in [format!("{secret}"), format!("{secret:?}")] {
            assert_eq!(printed, "<secret>");
            assert!(!printed.contains(material));
            assert!(!printed.contains("47"));
        }
        for sink in ["card", "receipt", "journal", "cache-key"] {
            let printable = format!("{sink}:{secret:?}");
            assert_eq!(printable, format!("{sink}:<secret>"));
            assert!(!printable.contains(material));
        }
    }

    #[test]
    fn empty_and_control_bearing_material_is_rejected_without_echoing_it() {
        assert!(Secret::new("").is_err());
        let material = "credential\nleak";
        let error = Secret::new(material).unwrap_err().to_string();
        assert!(!error.contains(material));
        assert!(!error.contains("credential"));
    }
}
