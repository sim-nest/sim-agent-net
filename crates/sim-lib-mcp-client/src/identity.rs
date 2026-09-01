use std::fmt;

use crate::ClientError;

/// A normalized HTTP origin and resource path.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct HttpEndpoint {
    scheme: String,
    authority: String,
    resource: String,
}

impl HttpEndpoint {
    /// Parses an absolute HTTP(S) URL, removing fragments, query parameters,
    /// default ports, repeated path separators, and a trailing slash.
    pub fn parse(value: &str) -> Result<Self, ClientError> {
        let (scheme, rest) = value
            .split_once("://")
            .ok_or_else(|| ClientError::Policy("endpoint URL must be absolute".into()))?;
        let scheme = scheme.to_ascii_lowercase();
        if scheme != "http" && scheme != "https" {
            return Err(ClientError::Policy("endpoint URL must use HTTP(S)".into()));
        }
        let without_fragment = rest.split('#').next().unwrap_or(rest);
        let without_query = without_fragment
            .split('?')
            .next()
            .unwrap_or(without_fragment);
        let (authority, raw_path) = without_query
            .split_once('/')
            .map_or((without_query, ""), |(host, path)| (host, path));
        if authority.is_empty() || authority.contains('@') {
            return Err(ClientError::Policy(
                "endpoint identity excludes credentials and requires a host".into(),
            ));
        }
        let mut authority = authority.to_ascii_lowercase();
        if (scheme == "http" && authority.ends_with(":80"))
            || (scheme == "https" && authority.ends_with(":443"))
        {
            authority.truncate(authority.rfind(':').unwrap_or(authority.len()));
        }
        let components: Vec<_> = raw_path
            .split('/')
            .filter(|component| !component.is_empty() && *component != ".")
            .collect();
        if components.contains(&"..") {
            return Err(ClientError::Policy(
                "endpoint resource cannot contain parent traversal".into(),
            ));
        }
        let resource = if components.is_empty() {
            "/".to_owned()
        } else {
            format!("/{}", components.join("/"))
        };
        Ok(Self {
            scheme,
            authority,
            resource,
        })
    }

    /// Normalized origin.
    pub fn origin(&self) -> String {
        format!("{}://{}", self.scheme, self.authority)
    }

    /// Normalized MCP resource path.
    pub fn resource(&self) -> &str {
        &self.resource
    }
}

/// Stable cache identity for a remote HTTP resource or one child instance.
/// Credentials and display metadata are deliberately absent.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum EndpointIdentity {
    /// Normalized HTTP origin and resource.
    Http(HttpEndpoint),
    /// Unique identity assigned by the process host to one child lifetime.
    ProcessInstance(String),
}

impl EndpointIdentity {
    /// Constructs an HTTP identity.
    pub fn http(url: &str) -> Result<Self, ClientError> {
        Ok(Self::Http(HttpEndpoint::parse(url)?))
    }

    /// Constructs a child-process identity. The instance id must change after exit.
    pub fn process(instance_id: impl Into<String>) -> Result<Self, ClientError> {
        let value = instance_id.into();
        if value.trim().is_empty() {
            return Err(ClientError::Policy(
                "process instance identity cannot be empty".into(),
            ));
        }
        Ok(Self::ProcessInstance(value))
    }
}

impl fmt::Display for EndpointIdentity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Http(endpoint) => write!(f, "{}{}", endpoint.origin(), endpoint.resource()),
            Self::ProcessInstance(instance) => write!(f, "process:{instance}"),
        }
    }
}
