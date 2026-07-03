use std::{
    fs,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use sim_kernel::{Args, Cx, Error, Expr, Result, Symbol, Value};

use crate::{
    Server, ServerAddress, ServerFrame, decode_frame_payload, helpers::clone_server_cx,
    symbol_list_value,
};

#[cfg(not(test))]
use super::sources::spawn_stdin_reader;
#[cfg(test)]
use super::sources::{remove_test_stdin_sender, test_stdin_sender, test_stdin_senders};
use super::{
    NEXT_TRIGGER_ID, StdinSource, TriggerConfig, TriggerDecoder, TriggerHandle, TriggerState,
    delivery_timeout, io_error_to_host, source_capability, trigger_read_policy,
};

impl TriggerHandle {
    pub(super) fn new(server: Arc<Server>, config: TriggerConfig) -> Self {
        let TriggerConfig {
            source,
            source_expr,
            role,
            codec,
            decode_expr,
            decoder,
            cron,
            network_source,
        } = config;
        let id = NEXT_TRIGGER_ID.fetch_add(1, Ordering::Relaxed);
        let stdin = match &source {
            ServerAddress::Stdin => {
                let (tx, rx) = mpsc::channel();
                #[cfg(not(test))]
                spawn_stdin_reader(tx);
                #[cfg(test)]
                {
                    test_stdin_senders()
                        .lock()
                        .expect("test stdin sender registry poisoned")
                        .insert(id, tx);
                }
                StdinSource::Channel(Mutex::new(rx))
            }
            _ => StdinSource::Unavailable,
        };
        Self {
            id,
            server,
            source,
            source_expr,
            role,
            codec,
            decode_expr,
            decoder,
            cron,
            network_source: network_source.map(Mutex::new),
            stopping: AtomicBool::new(false),
            handle: Mutex::new(None),
            stdin,
            state: Mutex::new(TriggerState::default()),
        }
    }

    pub(crate) fn start(self: &Arc<Self>, seed: &Cx) -> Result<()> {
        let mut handle = self
            .handle
            .lock()
            .map_err(|_| Error::PoisonedLock("trigger handle"))?;
        if handle.is_some() {
            return Ok(());
        }
        let mut cx = clone_server_cx(seed);
        let trigger = Arc::clone(self);
        *handle = Some(thread::spawn(move || {
            trigger.run(&mut cx);
        }));
        Ok(())
    }

    pub(crate) fn stop(&self) -> Result<()> {
        self.stopping.store(true, Ordering::Relaxed);
        #[cfg(test)]
        remove_test_stdin_sender(self.id);
        let join = self
            .handle
            .lock()
            .map_err(|_| Error::PoisonedLock("trigger handle"))?
            .take();
        if let Some(join) = join {
            join.join()
                .map_err(|_| Error::HostError("trigger thread panicked".to_owned()))?;
        }
        Ok(())
    }

    pub(crate) fn poll(&self, cx: &mut Cx) -> Result<u64> {
        match &self.source {
            ServerAddress::Stdin => self.poll_stdin(cx),
            ServerAddress::FileTail { path } => self.poll_file_tail(cx, path),
            ServerAddress::Cron { .. } => self.poll_cron(cx),
            ServerAddress::Webhook { .. }
            | ServerAddress::Imap { .. }
            | ServerAddress::Smtp { .. }
            | ServerAddress::Telegram { .. }
            | ServerAddress::Matrix { .. } => self.poll_network_source(cx),
            other => Err(Error::Eval(format!(
                "server/trigger does not support source {}",
                other.kind_symbol()
            ))),
        }
    }

    #[cfg(test)]
    pub(crate) fn inject_text(&self, cx: &mut Cx, text: &str) -> Result<u64> {
        let mut delivered = 0;
        for line in text.lines() {
            if line.trim().is_empty() {
                continue;
            }
            self.inject_event(cx, line.as_bytes())?;
            delivered += 1;
        }
        Ok(delivered)
    }

    #[cfg(test)]
    pub(crate) fn feed_stdin(&self, text: &str) -> Result<()> {
        let sender = test_stdin_sender(self.id)?;
        for line in text.lines() {
            sender
                .send(Some(format!("{line}\n").into_bytes()))
                .map_err(|_| Error::HostError("stdin trigger source closed".to_owned()))?;
        }
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn finish_stdin(&self) -> Result<()> {
        test_stdin_sender(self.id)?
            .send(None)
            .map_err(|_| Error::HostError("stdin trigger source closed".to_owned()))
    }

    #[cfg(test)]
    pub(crate) fn is_source_closed(&self) -> bool {
        self.state
            .lock()
            .map(|state| state.source_closed)
            .unwrap_or(true)
    }

    #[cfg(all(test, feature = "trigger-webhook"))]
    pub(crate) fn webhook_port(&self) -> Result<Option<u16>> {
        let Some(source) = &self.network_source else {
            return Ok(None);
        };
        let source = source
            .lock()
            .map_err(|_| Error::PoisonedLock("trigger source"))?;
        let Some(source) = source
            .as_any()
            .downcast_ref::<super::sources::network::WebhookSource>()
        else {
            return Ok(None);
        };
        source.local_port().map(Some)
    }

    /// Reflects the trigger's state as a descriptor table value.
    ///
    /// Includes its kind, role, delivered-event count, and required
    /// capabilities.
    pub fn reflect_value(&self, cx: &mut Cx) -> Result<Value> {
        let role = match &self.role {
            Some(role) => cx.factory().symbol(role.clone())?,
            None => cx.factory().nil()?,
        };
        let delivered = self
            .state
            .lock()
            .map_err(|_| Error::PoisonedLock("trigger state"))?
            .delivered;
        let capabilities = match source_capability(&self.source) {
            Some(capability) => symbol_list_value(cx, &[capability.as_symbol()])?,
            None => cx.factory().list(Vec::new())?,
        };
        cx.factory().table(vec![
            (
                Symbol::new("kind"),
                cx.factory().symbol(Symbol::new("trigger"))?,
            ),
            (Symbol::new("id"), cx.factory().string(self.id.to_string())?),
            (
                Symbol::new("source"),
                cx.factory().expr(self.source_expr.clone())?,
            ),
            (Symbol::new("role"), role),
            (
                Symbol::new("codec"),
                cx.factory().symbol(self.codec.clone())?,
            ),
            (
                Symbol::new("decode"),
                cx.factory().expr(self.decode_expr.clone())?,
            ),
            (
                Symbol::new("delivered"),
                cx.factory().string(delivered.to_string())?,
            ),
            (Symbol::new("requires"), capabilities),
        ])
    }

    fn poll_file_tail(&self, cx: &mut Cx, path: &PathBuf) -> Result<u64> {
        let mut bytes = fs::read(path).map_err(io_error_to_host)?;
        let lines = {
            let mut state = self
                .state
                .lock()
                .map_err(|_| Error::PoisonedLock("trigger state"))?;
            if state.file_offset > bytes.len() {
                state.file_offset = 0;
                state.file_remainder.clear();
            }
            let mut pending = std::mem::take(&mut state.file_remainder);
            pending.extend_from_slice(&bytes[state.file_offset..]);
            state.file_offset = bytes.len();

            let mut lines = Vec::new();
            let mut start = 0usize;
            for index in 0..pending.len() {
                if pending[index] == b'\n' {
                    let mut line = pending[start..index].to_vec();
                    if line.last() == Some(&b'\r') {
                        line.pop();
                    }
                    lines.push(line);
                    start = index + 1;
                }
            }
            state.file_remainder = pending[start..].to_vec();
            lines
        };
        bytes.clear();

        let mut delivered = 0;
        for line in lines {
            if line.iter().all(u8::is_ascii_whitespace) {
                continue;
            }
            self.inject_event(cx, &line)?;
            delivered += 1;
        }
        Ok(delivered)
    }

    fn poll_stdin(&self, cx: &mut Cx) -> Result<u64> {
        let receiver = match &self.stdin {
            StdinSource::Channel(receiver) => receiver,
            StdinSource::Unavailable => return Ok(0),
        };
        let mut delivered = 0;
        loop {
            match receiver
                .lock()
                .map_err(|_| Error::PoisonedLock("stdin trigger source"))?
                .try_recv()
            {
                Ok(Some(mut line)) => {
                    if line.last() == Some(&b'\n') {
                        line.pop();
                    }
                    if line.last() == Some(&b'\r') {
                        line.pop();
                    }
                    if line.iter().all(u8::is_ascii_whitespace) {
                        continue;
                    }
                    self.inject_event(cx, &line)?;
                    delivered += 1;
                }
                Ok(None) | Err(mpsc::TryRecvError::Disconnected) => {
                    self.state
                        .lock()
                        .map_err(|_| Error::PoisonedLock("trigger state"))?
                        .source_closed = true;
                    break;
                }
                Err(mpsc::TryRecvError::Empty) => break,
            }
        }
        Ok(delivered)
    }

    fn poll_network_source(&self, cx: &mut Cx) -> Result<u64> {
        let Some(source) = &self.network_source else {
            return Ok(0);
        };
        let mut delivered = 0;
        loop {
            let timeout = if delivered == 0 {
                delivery_timeout()
            } else {
                Duration::from_millis(0)
            };
            let event = source
                .lock()
                .map_err(|_| Error::PoisonedLock("trigger source"))?
                .next_event(cx, timeout)?;
            let Some(event) = event else {
                break;
            };
            self.inject_event(cx, &event)?;
            source
                .lock()
                .map_err(|_| Error::PoisonedLock("trigger source"))?
                .ack(cx)?;
            delivered += 1;
        }
        Ok(delivered)
    }

    fn poll_cron(&self, cx: &mut Cx) -> Result<u64> {
        let Some(matcher) = &self.cron else {
            return Ok(0);
        };
        let Some(minute_key) = matcher.current_match(SystemTime::now()) else {
            return Ok(0);
        };
        {
            let mut state = self
                .state
                .lock()
                .map_err(|_| Error::PoisonedLock("trigger state"))?;
            if state.last_cron_minute == Some(minute_key) {
                return Ok(0);
            }
            state.last_cron_minute = Some(minute_key);
        }
        self.inject_event(cx, b"tick")?;
        Ok(1)
    }

    fn inject_event(&self, cx: &mut Cx, raw: &[u8]) -> Result<()> {
        if let Some(capability) = source_capability(&self.source) {
            cx.require(&capability)?;
        }
        let expr = self.decode_event(cx, raw)?;
        let source = self.source.kind_symbol();
        let when_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_millis() as u64)
            .unwrap_or(0);
        let mut frame = ServerFrame::from_expr(
            cx,
            self.codec.clone(),
            crate::FrameKind::Trigger {
                source: source.clone(),
                when_ms,
            },
            &expr,
            sim_kernel::Consistency::LocalFirst,
            Vec::new(),
            false,
        )?;
        frame.envelope.role = self.role.clone();
        frame.envelope.trigger_source = Some(source);
        self.server.deliver_trigger_frame(cx, frame)?;
        self.state
            .lock()
            .map_err(|_| Error::PoisonedLock("trigger state"))?
            .delivered += 1;
        Ok(())
    }

    fn decode_event(&self, cx: &mut Cx, raw: &[u8]) -> Result<Expr> {
        match &self.decoder {
            TriggerDecoder::Codec(codec) => {
                decode_frame_payload(cx, codec, raw, trigger_read_policy(cx), Default::default())
            }
            TriggerDecoder::Callable(callable) => {
                let text = String::from_utf8(raw.to_vec()).map_err(|_| {
                    Error::Eval(
                        "trigger event bytes must be valid utf-8 for callable decoders".to_owned(),
                    )
                })?;
                let arg = cx.factory().string(text)?;
                let value = cx.call_value(callable.clone(), Args::new(vec![arg]))?;
                value.object().as_expr(cx)
            }
        }
    }

    fn run(self: Arc<Self>, cx: &mut Cx) {
        while !self.stopping.load(Ordering::Relaxed) {
            match self.poll(cx) {
                Ok(_) => {}
                Err(_) => break,
            }
            let source_closed = self
                .state
                .lock()
                .map(|state| state.source_closed)
                .unwrap_or(true);
            if source_closed {
                break;
            }
            thread::sleep(self.poll_interval());
        }
    }

    fn poll_interval(&self) -> Duration {
        match self.source {
            ServerAddress::Cron { .. } => Duration::from_millis(250),
            _ => Duration::from_millis(25),
        }
    }
}
