use sim_kernel::{CapabilityName, Cx, Error, Result};

pub(crate) fn fs_read_capability() -> CapabilityName {
    CapabilityName::new("fs/read")
}

pub(crate) fn fs_write_capability() -> CapabilityName {
    CapabilityName::new("fs/write")
}

pub(crate) fn find_capability() -> CapabilityName {
    CapabilityName::new("find")
}

pub(crate) fn edit_capability() -> CapabilityName {
    CapabilityName::new("edit")
}

pub(crate) fn exec_capability() -> CapabilityName {
    CapabilityName::new("exec")
}

pub(crate) fn net_http_capability() -> CapabilityName {
    CapabilityName::new("net/http")
}

pub(crate) fn require_fs_read_capability(cx: &Cx) -> Result<()> {
    require_with_aliases(cx, fs_read_capability(), fs_read_aliases())
}

pub(crate) fn require_fs_write_capability(cx: &Cx) -> Result<()> {
    require_with_aliases(cx, fs_write_capability(), fs_write_aliases())
}

pub(crate) fn require_net_http_capability(cx: &Cx) -> Result<()> {
    require_with_aliases(cx, net_http_capability(), net_http_aliases())
}

pub(crate) fn require_component_capability(cx: &Cx, capability: &CapabilityName) -> Result<()> {
    if capability == &fs_read_capability() {
        require_fs_read_capability(cx)
    } else if capability == &fs_write_capability() {
        require_fs_write_capability(cx)
    } else if capability == &net_http_capability() {
        require_net_http_capability(cx)
    } else {
        cx.require(capability)
    }
}

fn fs_read_aliases() -> &'static [&'static str] {
    &["table.fs.read", "stream.file.read", "file-read"]
}

fn fs_write_aliases() -> &'static [&'static str] {
    &[
        "table.fs.write",
        "table.fs.mkdir",
        "table.fs.rmdir",
        "stream.file.write",
        "file-write",
    ]
}

fn net_http_aliases() -> &'static [&'static str] {
    &["net.http", "net-connect", "network"]
}

fn require_with_aliases(
    cx: &Cx,
    canonical: CapabilityName,
    aliases: &'static [&'static str],
) -> Result<()> {
    if cx.capabilities().contains(&canonical)
        || aliases
            .iter()
            .any(|alias| cx.capabilities().contains(&CapabilityName::new(*alias)))
    {
        Ok(())
    } else {
        Err(Error::CapabilityDenied {
            capability: canonical,
        })
    }
}
