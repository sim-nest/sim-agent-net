#[cfg(not(test))]
use std::io::{self, BufRead};
#[cfg(test)]
use std::sync::mpsc;
use std::{
    collections::BTreeMap,
    sync::{Mutex, OnceLock},
};
#[cfg(not(test))]
use std::{sync::mpsc, thread};

use sim_kernel::{CapabilityName, Cx, Error, Result};

use crate::ServerAddress;

use super::source_loopback_enabled;

pub(super) mod network;

pub(crate) trait TriggerSource: Send + Sync {
    fn next_event(&mut self, cx: &mut Cx, timeout: std::time::Duration) -> Result<Option<Vec<u8>>>;

    fn ack(&mut self, _cx: &mut Cx) -> Result<()> {
        Ok(())
    }

    #[cfg(test)]
    #[allow(dead_code)]
    fn as_any(&self) -> &dyn std::any::Any;
}

const CRON_SCHEDULE_CAPABILITY: &str = "cron-schedule";
const WEBHOOK_SERVE_CAPABILITY: &str = "webhook-serve";
const MAIL_READ_CAPABILITY: &str = "mail-read";
const MAIL_WRITE_CAPABILITY: &str = "mail-write";
const TELEGRAM_BOT_CAPABILITY: &str = "telegram-bot";
const MATRIX_BOT_CAPABILITY: &str = "matrix-bot";

pub(crate) fn queued_trigger_events() -> &'static Mutex<BTreeMap<String, Vec<Vec<u8>>>> {
    static EVENTS: OnceLock<Mutex<BTreeMap<String, Vec<Vec<u8>>>>> = OnceLock::new();
    EVENTS.get_or_init(|| Mutex::new(BTreeMap::new()))
}

pub(crate) fn loopback_smtp_messages() -> &'static Mutex<BTreeMap<String, Vec<Vec<u8>>>> {
    static EVENTS: OnceLock<Mutex<BTreeMap<String, Vec<Vec<u8>>>>> = OnceLock::new();
    EVENTS.get_or_init(|| Mutex::new(BTreeMap::new()))
}

pub(crate) fn source_capability(source: &ServerAddress) -> Option<CapabilityName> {
    match source {
        ServerAddress::Stdin => None,
        ServerAddress::FileTail { .. } => Some(fs_read_capability()),
        ServerAddress::Cron { .. } => Some(CapabilityName::new(CRON_SCHEDULE_CAPABILITY)),
        ServerAddress::Webhook { .. } => Some(CapabilityName::new(WEBHOOK_SERVE_CAPABILITY)),
        ServerAddress::Imap { .. } => Some(CapabilityName::new(MAIL_READ_CAPABILITY)),
        ServerAddress::Smtp { .. } => Some(CapabilityName::new(MAIL_WRITE_CAPABILITY)),
        ServerAddress::Telegram { .. } => Some(CapabilityName::new(TELEGRAM_BOT_CAPABILITY)),
        ServerAddress::Matrix { .. } => Some(CapabilityName::new(MATRIX_BOT_CAPABILITY)),
        _ => None,
    }
}

pub(crate) fn require_source_capability(cx: &Cx, source: &ServerAddress) -> Result<()> {
    match source {
        ServerAddress::FileTail { .. } => {
            require_with_aliases(cx, fs_read_capability(), fs_read_aliases())
        }
        _ => match source_capability(source) {
            Some(capability) => cx.require(&capability),
            None => Ok(()),
        },
    }
}

fn fs_read_capability() -> CapabilityName {
    CapabilityName::new("fs/read")
}

fn fs_read_aliases() -> &'static [&'static str] {
    &["table.fs.read", "stream.file.read", "file-read"]
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

pub(crate) fn build_network_source(
    source: &ServerAddress,
    source_expr: &sim_kernel::Expr,
) -> Result<Option<Box<dyn TriggerSource>>> {
    let loopback = source_loopback_enabled(source_expr);
    match source {
        ServerAddress::Webhook { route } => {
            if loopback {
                Ok(Some(Box::new(network::QueueSource::new(queue_key(
                    source,
                )?))))
            } else {
                #[cfg(feature = "trigger-webhook")]
                {
                    Ok(Some(Box::new(network::WebhookSource::bind(
                        route.clone(),
                        super::source_options::source_u16(source_expr, "port")?.unwrap_or(0),
                    )?)))
                }
                #[cfg(not(feature = "trigger-webhook"))]
                {
                    let _ = route;
                    Err(Error::Eval(
                        "webhook trigger requires feature trigger-webhook".to_owned(),
                    ))
                }
            }
        }
        ServerAddress::Imap { address, mailbox } => {
            if loopback {
                Ok(Some(Box::new(network::QueueSource::new(queue_key(
                    source,
                )?))))
            } else {
                #[cfg(feature = "trigger-imap")]
                {
                    Ok(Some(Box::new(network::ImapSource::new(
                        address.clone(),
                        mailbox.clone(),
                        super::source_options::source_string(source_expr, "user")?.or_else(|| {
                            super::source_options::source_string(source_expr, "username")
                                .ok()
                                .flatten()
                        }),
                        super::source_options::source_string(source_expr, "password")?,
                    ))))
                }
                #[cfg(not(feature = "trigger-imap"))]
                {
                    let _ = (address, mailbox);
                    Err(Error::Eval(
                        "imap trigger requires feature trigger-imap".to_owned(),
                    ))
                }
            }
        }
        ServerAddress::Smtp { address } => {
            if loopback {
                Ok(Some(Box::new(network::LoopbackSmtpSource::new(queue_key(
                    source,
                )?))))
            } else {
                #[cfg(feature = "trigger-smtp")]
                {
                    Ok(Some(Box::new(network::SmtpSource::new(
                        address.clone(),
                        queue_key(source)?,
                        super::source_options::source_string(source_expr, "user")?.or_else(|| {
                            super::source_options::source_string(source_expr, "username")
                                .ok()
                                .flatten()
                        }),
                        super::source_options::source_string(source_expr, "from")?,
                        super::source_options::source_string(source_expr, "to")?,
                    ))))
                }
                #[cfg(not(feature = "trigger-smtp"))]
                {
                    let _ = address;
                    Err(Error::Eval(
                        "smtp trigger requires feature trigger-smtp".to_owned(),
                    ))
                }
            }
        }
        ServerAddress::Telegram { chat_id, bot } => {
            if loopback {
                Ok(Some(Box::new(network::QueueSource::new(queue_key(
                    source,
                )?))))
            } else {
                #[cfg(feature = "trigger-telegram")]
                {
                    Ok(Some(Box::new(network::TelegramSource::new(
                        bot.clone(),
                        chat_id.clone(),
                        super::source_options::source_string(source_expr, "base-url")?.ok_or_else(
                            || {
                                Error::Eval(
                                    "telegram trigger requires :base-url when not using :loopback"
                                        .to_owned(),
                                )
                            },
                        )?,
                    )?)))
                }
                #[cfg(not(feature = "trigger-telegram"))]
                {
                    let _ = (chat_id, bot);
                    Err(Error::Eval(
                        "telegram trigger requires feature trigger-telegram".to_owned(),
                    ))
                }
            }
        }
        ServerAddress::Matrix { room_id } => {
            if loopback {
                Ok(Some(Box::new(network::QueueSource::new(queue_key(
                    source,
                )?))))
            } else {
                #[cfg(feature = "trigger-matrix")]
                {
                    Ok(Some(Box::new(network::MatrixSource::new(
                        room_id.clone(),
                        super::source_options::source_string(source_expr, "base-url")?.ok_or_else(
                            || {
                                Error::Eval(
                                    "matrix trigger requires :base-url when not using :loopback"
                                        .to_owned(),
                                )
                            },
                        )?,
                    )?)))
                }
                #[cfg(not(feature = "trigger-matrix"))]
                {
                    let _ = room_id;
                    Err(Error::Eval(
                        "matrix trigger requires feature trigger-matrix".to_owned(),
                    ))
                }
            }
        }
        _ => Ok(None),
    }
}

pub(crate) fn queue_key(source: &ServerAddress) -> Result<String> {
    match source {
        ServerAddress::Webhook { route } => Ok(format!("webhook:{route}")),
        ServerAddress::Imap { address, mailbox } => Ok(format!("imap:{address}:{mailbox}")),
        ServerAddress::Smtp { address } => Ok(format!("smtp:{address}")),
        ServerAddress::Telegram { chat_id, bot } => Ok(format!("telegram:{chat_id}:{bot}")),
        ServerAddress::Matrix { room_id } => Ok(format!("matrix:{room_id}")),
        _ => Err(Error::Eval(
            "source does not use the queued trigger runtime".to_owned(),
        )),
    }
}

#[cfg(not(test))]
pub(crate) fn spawn_stdin_reader(tx: mpsc::Sender<Option<Vec<u8>>>) {
    thread::spawn(move || {
        let stdin = io::stdin();
        let mut locked = stdin.lock();
        loop {
            let mut line = String::new();
            match locked.read_line(&mut line) {
                Ok(0) => {
                    let _ = tx.send(None);
                    break;
                }
                Ok(_) => {
                    if tx.send(Some(line.into_bytes())).is_err() {
                        break;
                    }
                }
                Err(error) => {
                    let _ = tx.send(None);
                    let _ = error;
                    break;
                }
            }
        }
    });
}

#[cfg(test)]
// Test-only registry handle; the concrete sender-map type is intentionally explicit.
#[allow(clippy::type_complexity)]
pub(crate) fn test_stdin_senders() -> &'static Mutex<BTreeMap<u64, mpsc::Sender<Option<Vec<u8>>>>> {
    static SENDERS: OnceLock<Mutex<BTreeMap<u64, mpsc::Sender<Option<Vec<u8>>>>>> = OnceLock::new();
    SENDERS.get_or_init(|| Mutex::new(BTreeMap::new()))
}

#[cfg(test)]
pub(crate) fn test_stdin_sender(id: u64) -> Result<mpsc::Sender<Option<Vec<u8>>>> {
    test_stdin_senders()
        .lock()
        .map_err(|_| Error::PoisonedLock("test stdin sender registry"))?
        .get(&id)
        .cloned()
        .ok_or_else(|| Error::HostError("stdin trigger sender not available".to_owned()))
}

#[cfg(test)]
pub(crate) fn remove_test_stdin_sender(id: u64) {
    let _ = test_stdin_senders()
        .lock()
        .map(|mut senders| senders.remove(&id));
}
