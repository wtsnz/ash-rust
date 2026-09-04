use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::{MailerError, Result};

/// Represents an outgoing email message.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Email {
    /// Unique identifier for this email message instance.
    pub id: Uuid,
    /// Sender address (e.g. `"No Reply <noreply@example.com>"`).
    pub from: Option<String>,
    /// Primary recipient email addresses.
    pub to: Vec<String>,
    /// Carbon-copy recipient email addresses.
    pub cc: Vec<String>,
    /// Blind carbon-copy recipient email addresses.
    pub bcc: Vec<String>,
    /// Optional Reply-To address.
    pub reply_to: Option<String>,
    /// Email subject line.
    pub subject: Option<String>,
    /// Plaintext message body.
    pub text: Option<String>,
    /// HTML formatted message body.
    pub html: Option<String>,
    /// Custom SMTP or transport headers.
    pub headers: HashMap<String, String>,
}

impl Default for Email {
    fn default() -> Self {
        Self {
            id: Uuid::new_v4(),
            from: None,
            to: Vec::new(),
            cc: Vec::new(),
            bcc: Vec::new(),
            reply_to: None,
            subject: None,
            text: None,
            html: None,
            headers: HashMap::new(),
        }
    }
}

impl Email {
    /// Create a new blank email message with a generated UUID.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the sender `From` address.
    pub fn from(mut self, from: impl Into<String>) -> Self {
        self.from = Some(from.into());
        self
    }

    /// Add a primary `To` recipient address.
    pub fn to(mut self, to: impl Into<String>) -> Self {
        self.to.push(to.into());
        self
    }

    /// Add multiple primary `To` recipient addresses.
    pub fn to_many<I, S>(mut self, recipients: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        for r in recipients {
            self.to.push(r.into());
        }
        self
    }

    /// Add a `Cc` recipient address.
    pub fn cc(mut self, cc: impl Into<String>) -> Self {
        self.cc.push(cc.into());
        self
    }

    /// Add a `Bcc` recipient address.
    pub fn bcc(mut self, bcc: impl Into<String>) -> Self {
        self.bcc.push(bcc.into());
        self
    }

    /// Set the `Reply-To` address.
    pub fn reply_to(mut self, reply_to: impl Into<String>) -> Self {
        self.reply_to = Some(reply_to.into());
        self
    }

    /// Set the email subject line.
    pub fn subject(mut self, subject: impl Into<String>) -> Self {
        self.subject = Some(subject.into());
        self
    }

    /// Set the plain-text message body.
    pub fn text(mut self, text: impl Into<String>) -> Self {
        self.text = Some(text.into());
        self
    }

    /// Set the HTML message body.
    pub fn html(mut self, html: impl Into<String>) -> Self {
        self.html = Some(html.into());
        self
    }

    /// Attach a custom transport header.
    pub fn header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.insert(name.into(), value.into());
        self
    }

    /// Validate that required email fields (`from`, at least one recipient, and `subject`) are populated.
    pub fn validate(&self) -> Result<()> {
        if self.from.as_ref().map(|s| s.trim()).unwrap_or("").is_empty() {
            return Err(MailerError::MissingSender);
        }

        if self.to.is_empty() && self.cc.is_empty() && self.bcc.is_empty() {
            return Err(MailerError::MissingRecipient);
        }

        if self.subject.as_ref().map(|s| s.trim()).unwrap_or("").is_empty() {
            return Err(MailerError::MissingSubject);
        }

        Ok(())
    }
}
