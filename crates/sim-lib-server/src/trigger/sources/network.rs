#[cfg(feature = "trigger-webhook")]
use crate::transport::port_io::PortTcpListener as TcpListener;
#[cfg(any(feature = "trigger-imap", feature = "trigger-smtp"))]
use std::io::{BufReader, Write};
use std::time::Duration;

use sim_kernel::{Cx, Error, Result};

use super::{TriggerSource, loopback_smtp_messages, queued_trigger_events};

mod common;
mod polling;

#[cfg(any(
    feature = "trigger-imap",
    feature = "trigger-smtp",
    feature = "trigger-telegram",
    feature = "trigger-matrix"
))]
use common::connect_with_timeout;
#[cfg(any(
    feature = "trigger-webhook",
    feature = "trigger-imap",
    feature = "trigger-smtp",
    feature = "trigger-telegram",
    feature = "trigger-matrix"
))]
use common::io_to_host;
#[cfg(any(feature = "trigger-imap", feature = "trigger-smtp"))]
use common::parse_host_port;
#[cfg(feature = "trigger-smtp")]
use common::{expect_smtp_code, smtp_command};
#[cfg(feature = "trigger-imap")]
use common::{
    first_search_result, imap_command_collect, imap_command_ok, read_imap_fetch_body,
    read_imap_line,
};

pub(crate) struct QueueSource {
    key: String,
}

impl QueueSource {
    pub(crate) fn new(key: String) -> Self {
        Self { key }
    }
}

impl TriggerSource for QueueSource {
    fn next_event(&mut self, _cx: &mut Cx, _timeout: Duration) -> Result<Option<Vec<u8>>> {
        let mut queues = queued_trigger_events()
            .lock()
            .map_err(|_| Error::PoisonedLock("trigger queue"))?;
        let Some(events) = queues.get_mut(&self.key) else {
            return Ok(None);
        };
        if events.is_empty() {
            queues.remove(&self.key);
            return Ok(None);
        }
        let event = events.remove(0);
        if events.is_empty() {
            queues.remove(&self.key);
        }
        Ok(Some(event))
    }

    #[cfg(test)]
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

pub(crate) struct LoopbackSmtpSource {
    key: String,
}

impl LoopbackSmtpSource {
    pub(crate) fn new(key: String) -> Self {
        Self { key }
    }
}

impl TriggerSource for LoopbackSmtpSource {
    fn next_event(&mut self, cx: &mut Cx, timeout: Duration) -> Result<Option<Vec<u8>>> {
        let mut source = QueueSource::new(self.key.clone());
        let Some(event) = source.next_event(cx, timeout)? else {
            return Ok(None);
        };
        loopback_smtp_messages()
            .lock()
            .map_err(|_| Error::PoisonedLock("loopback smtp messages"))?
            .entry(self.key.clone())
            .or_default()
            .push(event.clone());
        Ok(Some(event))
    }

    #[cfg(test)]
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

#[cfg(feature = "trigger-smtp")]
pub(crate) struct SmtpSource {
    address: String,
    key: String,
    helo_name: String,
    from: Option<String>,
    to: Option<String>,
}

#[cfg(feature = "trigger-smtp")]
impl SmtpSource {
    pub(crate) fn new(
        address: String,
        key: String,
        helo_name: Option<String>,
        from: Option<String>,
        to: Option<String>,
    ) -> Self {
        Self {
            address,
            key,
            helo_name: helo_name.unwrap_or_else(|| "sim-server".to_owned()),
            from,
            to,
        }
    }
}

#[cfg(feature = "trigger-smtp")]
impl TriggerSource for SmtpSource {
    fn next_event(&mut self, _cx: &mut Cx, timeout: Duration) -> Result<Option<Vec<u8>>> {
        let event = {
            let mut queues = queued_trigger_events()
                .lock()
                .map_err(|_| Error::PoisonedLock("trigger queue"))?;
            let Some(events) = queues.get_mut(&self.key) else {
                return Ok(None);
            };
            if events.is_empty() {
                queues.remove(&self.key);
                return Ok(None);
            }
            let event = events.remove(0);
            if events.is_empty() {
                queues.remove(&self.key);
            }
            event
        };
        let from = self
            .from
            .clone()
            .ok_or_else(|| Error::Eval("smtp trigger requires :from".to_owned()))?;
        let to = self
            .to
            .clone()
            .ok_or_else(|| Error::Eval("smtp trigger requires :to".to_owned()))?;
        let (host, port) = parse_host_port(&self.address, 25)?;
        let stream = connect_with_timeout(&host, port, timeout)?;
        stream.set_read_timeout(Some(timeout)).map_err(io_to_host)?;
        stream
            .set_write_timeout(Some(timeout))
            .map_err(io_to_host)?;
        let mut reader = BufReader::new(stream);
        expect_smtp_code(&mut reader, 220)?;
        smtp_command(&mut reader, &format!("EHLO {}\r\n", self.helo_name), 250)?;
        smtp_command(&mut reader, &format!("MAIL FROM:<{from}>\r\n"), 250)?;
        smtp_command(&mut reader, &format!("RCPT TO:<{to}>\r\n"), 250)?;
        smtp_command(&mut reader, "DATA\r\n", 354)?;
        reader.get_mut().write_all(&event).map_err(io_to_host)?;
        if !event.ends_with(b"\r\n") {
            reader.get_mut().write_all(b"\r\n").map_err(io_to_host)?;
        }
        reader.get_mut().write_all(b".\r\n").map_err(io_to_host)?;
        reader.get_mut().flush().map_err(io_to_host)?;
        expect_smtp_code(&mut reader, 250)?;
        let _ = smtp_command(&mut reader, "QUIT\r\n", 221);
        Ok(Some(event))
    }

    #[cfg(test)]
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

#[cfg(feature = "trigger-imap")]
pub(crate) struct ImapSource {
    address: String,
    mailbox: String,
    user: Option<String>,
    password: Option<String>,
    pending_seen: Option<u32>,
}

#[cfg(feature = "trigger-imap")]
impl ImapSource {
    pub(crate) fn new(
        address: String,
        mailbox: String,
        user: Option<String>,
        password: Option<String>,
    ) -> Self {
        Self {
            address,
            mailbox,
            user,
            password,
            pending_seen: None,
        }
    }
}

#[cfg(feature = "trigger-imap")]
impl TriggerSource for ImapSource {
    fn next_event(&mut self, _cx: &mut Cx, timeout: Duration) -> Result<Option<Vec<u8>>> {
        let (host, port) = parse_host_port(&self.address, 143)?;
        let mut reader = BufReader::new(connect_with_timeout(&host, port, timeout)?);
        reader
            .get_mut()
            .set_read_timeout(Some(timeout))
            .map_err(io_to_host)?;
        reader
            .get_mut()
            .set_write_timeout(Some(timeout))
            .map_err(io_to_host)?;
        read_imap_line(&mut reader)?;
        let mut tag = 1u32;
        if let (Some(user), Some(password)) = (&self.user, &self.password) {
            imap_command_ok(&mut reader, &mut tag, &format!("LOGIN {user} {password}"))?;
        }
        imap_command_ok(&mut reader, &mut tag, &format!("SELECT {}", self.mailbox))?;
        let search = imap_command_collect(&mut reader, &mut tag, "SEARCH UNSEEN")?;
        let Some(message_id) = first_search_result(&search) else {
            let _ = imap_command_ok(&mut reader, &mut tag, "LOGOUT");
            return Ok(None);
        };
        let fetch_tag = format!("A{tag:04}");
        tag += 1;
        reader
            .get_mut()
            .write_all(format!("{fetch_tag} FETCH {message_id} BODY[]\r\n").as_bytes())
            .map_err(io_to_host)?;
        reader.get_mut().flush().map_err(io_to_host)?;
        let body = read_imap_fetch_body(&mut reader, &fetch_tag)?;
        self.pending_seen = Some(message_id);
        let _ = imap_command_ok(&mut reader, &mut tag, "LOGOUT");
        Ok(Some(body))
    }

    fn ack(&mut self, _cx: &mut Cx) -> Result<()> {
        let Some(message_id) = self.pending_seen.take() else {
            return Ok(());
        };
        let (host, port) = parse_host_port(&self.address, 143)?;
        let mut reader = BufReader::new(connect_with_timeout(&host, port, Duration::from_secs(1))?);
        read_imap_line(&mut reader)?;
        let mut tag = 1u32;
        if let (Some(user), Some(password)) = (&self.user, &self.password) {
            imap_command_ok(&mut reader, &mut tag, &format!("LOGIN {user} {password}"))?;
        }
        imap_command_ok(&mut reader, &mut tag, &format!("SELECT {}", self.mailbox))?;
        imap_command_ok(
            &mut reader,
            &mut tag,
            &format!("STORE {message_id} +FLAGS (\\Seen)"),
        )?;
        let _ = imap_command_ok(&mut reader, &mut tag, "LOGOUT");
        Ok(())
    }

    #[cfg(test)]
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

#[cfg(feature = "trigger-webhook")]
pub(crate) struct WebhookSource {
    listener: TcpListener,
    route: String,
}

#[cfg(feature = "trigger-webhook")]
impl WebhookSource {
    pub(crate) fn bind(route: String, port: u16) -> Result<Self> {
        #[cfg(feature = "server-net-http")]
        {
            let listener = TcpListener::bind(("127.0.0.1", port)).map_err(io_to_host)?;
            listener.set_nonblocking(true).map_err(io_to_host)?;
            Ok(Self { listener, route })
        }
        #[cfg(not(feature = "server-net-http"))]
        {
            let _ = (route, port);
            Err(Error::Eval(
                "webhook trigger requires feature server-net-http".to_owned(),
            ))
        }
    }

    #[cfg(test)]
    pub(crate) fn local_port(&self) -> Result<u16> {
        self.listener
            .local_addr()
            .map(|addr| addr.port())
            .map_err(io_to_host)
    }
}

#[cfg(feature = "trigger-webhook")]
impl TriggerSource for WebhookSource {
    fn next_event(&mut self, _cx: &mut Cx, timeout: Duration) -> Result<Option<Vec<u8>>> {
        #[cfg(feature = "server-net-http")]
        {
            self.listener.set_nonblocking(true).map_err(io_to_host)?;
            let deadline = std::time::Instant::now() + timeout;
            loop {
                match self.listener.accept() {
                    Ok((mut stream, _)) => {
                        stream.set_read_timeout(Some(timeout)).map_err(io_to_host)?;
                        let response = match crate::http::read_request(&mut stream)? {
                            Some(request)
                                if request.method == "POST" && request.path == self.route =>
                            {
                                let body = request.body;
                                crate::http::write_response(
                                    &mut stream,
                                    &crate::http::HttpResponse {
                                        status: 200,
                                        headers: Vec::new(),
                                        body: Vec::new(),
                                    },
                                )?;
                                return Ok(Some(body));
                            }
                            Some(request) if request.path != self.route => {
                                crate::http::HttpResponse {
                                    status: 404,
                                    headers: Vec::new(),
                                    body: b"not found".to_vec(),
                                }
                            }
                            Some(_) => crate::http::HttpResponse {
                                status: 405,
                                headers: Vec::new(),
                                body: b"method not allowed".to_vec(),
                            },
                            None => crate::http::HttpResponse {
                                status: 400,
                                headers: Vec::new(),
                                body: b"empty request".to_vec(),
                            },
                        };
                        crate::http::write_response(&mut stream, &response)?;
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        if std::time::Instant::now() >= deadline {
                            return Ok(None);
                        }
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    Err(error) => return Err(io_to_host(error)),
                }
            }
        }
        #[cfg(not(feature = "server-net-http"))]
        {
            let _ = timeout;
            Err(Error::Eval(
                "webhook trigger requires feature server-net-http".to_owned(),
            ))
        }
    }

    #[cfg(test)]
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

#[cfg(feature = "trigger-matrix")]
pub(crate) use polling::MatrixSource;
#[cfg(feature = "trigger-telegram")]
pub(crate) use polling::TelegramSource;
