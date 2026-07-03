#[cfg(any(feature = "trigger-imap", feature = "trigger-smtp"))]
use std::io::{BufRead, BufReader, Read, Write};
#[cfg(any(
    feature = "trigger-imap",
    feature = "trigger-smtp",
    feature = "trigger-telegram",
    feature = "trigger-matrix"
))]
use std::net::TcpStream;
#[cfg(any(
    feature = "trigger-imap",
    feature = "trigger-smtp",
    feature = "trigger-telegram",
    feature = "trigger-matrix"
))]
use std::time::Duration;

#[cfg(any(
    feature = "trigger-webhook",
    feature = "trigger-imap",
    feature = "trigger-smtp",
    feature = "trigger-telegram",
    feature = "trigger-matrix"
))]
use sim_kernel::{Error, Result};

#[cfg(any(feature = "trigger-imap", feature = "trigger-smtp"))]
pub(super) fn parse_host_port(address: &str, default_port: u16) -> Result<(String, u16)> {
    match address.rsplit_once(':') {
        Some((host, port)) => Ok((
            host.to_owned(),
            port.parse::<u16>()
                .map_err(|_| Error::Eval(format!("invalid port in address {address}")))?,
        )),
        None => Ok((address.to_owned(), default_port)),
    }
}

#[cfg(any(
    feature = "trigger-imap",
    feature = "trigger-smtp",
    feature = "trigger-telegram",
    feature = "trigger-matrix"
))]
pub(super) fn connect_with_timeout(host: &str, port: u16, timeout: Duration) -> Result<TcpStream> {
    let addr = format!("{host}:{port}");
    let stream = TcpStream::connect(addr).map_err(io_to_host)?;
    stream.set_read_timeout(Some(timeout)).map_err(io_to_host)?;
    stream
        .set_write_timeout(Some(timeout))
        .map_err(io_to_host)?;
    Ok(stream)
}

#[cfg(any(
    feature = "trigger-webhook",
    feature = "trigger-imap",
    feature = "trigger-smtp",
    feature = "trigger-telegram",
    feature = "trigger-matrix"
))]
pub(super) fn io_to_host(error: std::io::Error) -> Error {
    Error::HostError(error.to_string())
}

#[cfg(feature = "trigger-smtp")]
pub(super) fn smtp_command(
    reader: &mut BufReader<TcpStream>,
    command: &str,
    expected: u16,
) -> Result<()> {
    reader
        .get_mut()
        .write_all(command.as_bytes())
        .map_err(io_to_host)?;
    reader.get_mut().flush().map_err(io_to_host)?;
    expect_smtp_code(reader, expected)
}

#[cfg(feature = "trigger-smtp")]
pub(super) fn expect_smtp_code(reader: &mut BufReader<TcpStream>, expected: u16) -> Result<()> {
    loop {
        let line = read_line(reader)?;
        let Some(prefix) = line.get(0..3) else {
            return Err(Error::HostError("short smtp response".to_owned()));
        };
        let code = prefix
            .parse::<u16>()
            .map_err(|_| Error::HostError("invalid smtp status code".to_owned()))?;
        if code != expected {
            return Err(Error::Eval(format!(
                "smtp expected {expected}, received {code}"
            )));
        }
        if line.as_bytes().get(3) != Some(&b'-') {
            return Ok(());
        }
    }
}

#[cfg(feature = "trigger-imap")]
pub(super) fn imap_command_ok(
    reader: &mut BufReader<TcpStream>,
    tag_counter: &mut u32,
    command: &str,
) -> Result<()> {
    let _ = imap_command_collect(reader, tag_counter, command)?;
    Ok(())
}

#[cfg(feature = "trigger-imap")]
pub(super) fn imap_command_collect(
    reader: &mut BufReader<TcpStream>,
    tag_counter: &mut u32,
    command: &str,
) -> Result<Vec<String>> {
    let tag = format!("A{tag_counter:04}");
    *tag_counter += 1;
    reader
        .get_mut()
        .write_all(format!("{tag} {command}\r\n").as_bytes())
        .map_err(io_to_host)?;
    reader.get_mut().flush().map_err(io_to_host)?;
    let mut lines = Vec::new();
    loop {
        let line = read_imap_line(reader)?;
        if line.starts_with(&tag) {
            if !line.contains("OK") {
                return Err(Error::Eval(format!("imap command failed: {line}")));
            }
            return Ok(lines);
        }
        lines.push(line);
    }
}

#[cfg(feature = "trigger-imap")]
pub(super) fn first_search_result(lines: &[String]) -> Option<u32> {
    lines.iter().find_map(|line| {
        let suffix = line.strip_prefix("* SEARCH ")?;
        suffix.split_whitespace().next()?.parse::<u32>().ok()
    })
}

#[cfg(feature = "trigger-imap")]
pub(super) fn read_imap_fetch_body(
    reader: &mut BufReader<TcpStream>,
    tag: &str,
) -> Result<Vec<u8>> {
    let mut body = Vec::new();
    loop {
        let line = read_imap_line(reader)?;
        if line.starts_with(tag) {
            if !line.contains("OK") {
                return Err(Error::Eval(format!("imap fetch failed: {line}")));
            }
            return Ok(body);
        }
        if let Some(len) = imap_literal_len(&line)? {
            body.resize(len, 0);
            reader.read_exact(&mut body).map_err(io_to_host)?;
        }
    }
}

#[cfg(feature = "trigger-imap")]
fn imap_literal_len(line: &str) -> Result<Option<usize>> {
    let Some(start) = line.rfind('{') else {
        return Ok(None);
    };
    let Some(end) = line[start..].find('}') else {
        return Ok(None);
    };
    let digits = &line[start + 1..start + end];
    let len = digits
        .parse::<usize>()
        .map_err(|_| Error::HostError("invalid imap literal length".to_owned()))?;
    Ok(Some(len))
}

#[cfg(feature = "trigger-imap")]
pub(super) fn read_imap_line(reader: &mut BufReader<TcpStream>) -> Result<String> {
    read_line(reader)
}

#[cfg(any(feature = "trigger-imap", feature = "trigger-smtp"))]
pub(super) fn read_line(reader: &mut BufReader<TcpStream>) -> Result<String> {
    let mut line = String::new();
    let read = reader.read_line(&mut line).map_err(io_to_host)?;
    if read == 0 {
        return Err(Error::HostError("unexpected end of stream".to_owned()));
    }
    Ok(line.trim_end_matches(['\r', '\n']).to_owned())
}
