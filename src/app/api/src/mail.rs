//! Outgoing email. With `SMTP_URL` set, mail goes through that server
//! (`smtp://host:1025` for a local catcher, `smtps://user:pass@host:465` or
//! `smtp://user:pass@host:587?tls=required` for a real one). Without it,
//! messages are printed to the log, which is only good for development.

use lettre::message::Mailbox;
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug, PartialEq)]
pub struct Sent {
    pub to: String,
    pub subject: String,
    pub body: String,
}

#[derive(Clone)]
pub enum Mailer {
    Smtp { transport: AsyncSmtpTransport<Tokio1Executor>, from: Mailbox },
    Log,
    /// Keeps messages in memory, for tests.
    #[cfg_attr(not(test), allow(dead_code))]
    Memory(Arc<Mutex<Vec<Sent>>>),
}

impl Mailer {
    pub fn from_env() -> Result<Self, String> {
        // Compose passes unset settings as empty strings; treat those as unset.
        let var = |name: &str| std::env::var(name).ok().filter(|v| !v.trim().is_empty());
        let Some(url) = var("SMTP_URL") else {
            println!("SMTP_URL is not set: emails (sign-in links) are printed to this log instead of being sent");
            return Ok(Mailer::Log);
        };
        let from = var("MAIL_FROM").unwrap_or_else(|| "Lilanaya <no-reply@lilanaya.local>".into());
        let from: Mailbox = from.parse().map_err(|e| format!("MAIL_FROM is not a valid sender (\"{from}\"): {e}"))?;
        let transport = AsyncSmtpTransport::<Tokio1Executor>::from_url(&url).map_err(|e| format!("SMTP_URL is not valid: {e}"))?.build();
        Ok(Mailer::Smtp { transport, from })
    }

    pub async fn send(&self, to: &str, subject: &str, body: &str) -> Result<(), String> {
        match self {
            Mailer::Smtp { transport, from } => {
                let to: Mailbox = to.parse().map_err(|e| format!("recipient: {e}"))?;
                let message = Message::builder()
                    .from(from.clone())
                    .to(to)
                    .subject(subject)
                    .body(body.to_string())
                    .map_err(|e| format!("build message: {e}"))?;
                transport.send(message).await.map(|_| ()).map_err(|e| format!("smtp: {e}"))
            }
            Mailer::Log => {
                println!("--- email to {to} ---\nSubject: {subject}\n\n{body}\n--- end of email ---");
                Ok(())
            }
            Mailer::Memory(sent) => {
                sent.lock().unwrap().push(Sent { to: to.into(), subject: subject.into(), body: body.into() });
                Ok(())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn mail_kept_in_memory_is_kept_in_order_and_word_for_word() {
        let kept = Arc::new(Mutex::new(Vec::new()));
        let mailer = Mailer::Memory(kept.clone());
        mailer.send("ada@example.com", "First", "Open this link:\n\nhttp://localhost:3000/log-in/confirm?token=abc\n").await.unwrap();
        mailer.clone().send("bob@example.com", "Second", "ü").await.unwrap();
        let kept = kept.lock().unwrap();
        assert_eq!(kept.len(), 2);
        assert_eq!(kept[0], Sent { to: "ada@example.com".into(), subject: "First".into(), body: "Open this link:\n\nhttp://localhost:3000/log-in/confirm?token=abc\n".into() });
        assert_eq!((kept[1].to.as_str(), kept[1].body.as_str()), ("bob@example.com", "ü"));
    }

    #[tokio::test]
    async fn without_a_mail_server_a_message_goes_to_the_log_and_that_is_not_an_error() {
        assert_eq!(Mailer::Log.send("ada@example.com", "Subject", "Body").await, Ok(()));
    }

    #[tokio::test]
    async fn a_recipient_that_is_not_an_address_is_refused_before_anything_is_sent() {
        let transport = AsyncSmtpTransport::<Tokio1Executor>::from_url("smtp://127.0.0.1:1").unwrap().build();
        let mailer = Mailer::Smtp { transport, from: "Lilanaya <no-reply@lilanaya.local>".parse().unwrap() };
        for bad in ["", "not an address", "a@b\r\nBcc: everyone@example.com", "two@@example.com"] {
            let error = mailer.send(bad, "Subject", "Body").await.unwrap_err();
            assert!(error.starts_with("recipient:"), "{bad:?}: {error}");
        }
        // A real address gets as far as the server, which is not there.
        assert!(mailer.send("ada@example.com", "Subject", "Body").await.unwrap_err().starts_with("smtp:"));
    }
}
