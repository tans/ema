use anyhow::{bail, Context, Result};
use lettre::{message::{header::{HeaderName, HeaderValue}, Mailbox}, transport::smtp::authentication::Credentials, Message, SmtpTransport, Transport};
use mailparse::{MailAddr, MailHeaderMap, ParsedMail};
use crate::{config::MailSettings, db::Outbound};

#[derive(Debug, Clone)]
pub struct Incoming {
    pub uid: u32,
    pub message_id: String,
    pub sender: String,
    pub subject: String,
    pub body: String,
    pub references: Vec<String>,
    pub automated: bool,
    /// X-EMA-Kind header value, if present. Only EMA-written mail carries it.
    pub ema_kind: Option<String>,
    /// X-EMA-Message-Id header value (the original generated id), if present.
    pub ema_message_id: Option<String>,
}
#[derive(Debug)]
pub struct FetchedMail {
    pub uid: u32,
    pub message_id: String,
    pub sender: String,
    pub recipient: String,
    pub cc: String,
    pub references: String,
    pub original_date: String,
    pub subject: String,
    pub body: String,
    pub archive_note: String,
    pub message: Option<Incoming>,
    pub problem: &'static str,
}
fn plaintext(part: &ParsedMail<'_>) -> Result<String> {
    if part.ctype.mimetype.eq_ignore_ascii_case("text/plain") { return Ok(part.get_body()?); }
    for sub in &part.subparts {
        if let Ok(text) = plaintext(sub) { if !text.is_empty() { return Ok(text); } }
    }
    Ok(String::new())
}
fn archive_text(part: &ParsedMail<'_>, mimetype: &str) -> Result<String> {
    let disposition = part.get_content_disposition();
    if disposition.disposition == mailparse::DispositionType::Attachment
        || disposition.params.contains_key("filename") || part.ctype.params.contains_key("name") {
        return Ok(String::new());
    }
    if part.ctype.mimetype.eq_ignore_ascii_case(mimetype) { return Ok(part.get_body()?); }
    for sub in &part.subparts {
        if let Ok(text) = archive_text(sub, mimetype) { if !text.is_empty() { return Ok(text); } }
    }
    Ok(String::new())
}
pub(crate) fn redact_access_codes(text: &str) -> String {
    text.split('\n').map(|line| {
        if let Some(pos) = line.to_ascii_lowercase().find("ema-code:") {
            format!("{}EMA-Code: [redacted]", &line[..pos])
        } else { line.to_string() }
    }).collect::<Vec<_>>().join("\n")
}
fn archive(uid: u32, raw: &[u8]) -> FetchedMail {
    let headers = mailparse::parse_headers(raw).ok().map(|(headers, _)| headers).unwrap_or_default();
    let mut record = FetchedMail {
        uid,
        message_id: headers.get_first_value("Message-ID").unwrap_or_default(),
        sender: headers.get_first_value("From").unwrap_or_default(),
        recipient: headers.get_first_value("To").unwrap_or_default(),
        cc: headers.get_first_value("Cc").unwrap_or_default(),
        references: [headers.get_first_value("In-Reply-To"),headers.get_first_value("References")]
            .into_iter().flatten().collect::<Vec<_>>().join(" "),
        original_date: headers.get_first_value("Date").unwrap_or_default(),
        subject: headers.get_first_value("Subject").unwrap_or_default(),
        body: String::new(), archive_note: String::new(), message: None, problem: "malformed",
    };
    // The task parser retains its existing 256 KiB limit. Archiving can still
    // extract a body from larger messages with attachments, up to this bound.
    if raw.len() > 16 * 1024 * 1024 {
        record.archive_note = "too_large".into();
    } else if let Ok(parsed) = mailparse::parse_mail(raw) {
        let plain = archive_text(&parsed, "text/plain").unwrap_or_default();
        let is_html = plain.is_empty();
        let text = if is_html { archive_text(&parsed, "text/html").unwrap_or_default() } else { plain };
        let redacted = redact_access_codes(&text);
        let mut chars = redacted.chars();
        record.body = chars.by_ref().take(200_000).collect();
        record.archive_note = match (is_html && !record.body.is_empty(), chars.next().is_some()) {
            (true,true) => "html_truncated", (true,false) => "html",
            (false,true) => "truncated", (false,false) => "",
        }.into();
    } else {
        record.archive_note = "unreadable".into();
    }
    match parse(uid, raw) {
        Ok(message) => { record.sender=message.sender.clone(); record.message=Some(message); record.problem=""; }
        Err(_) if raw.len() > 256 * 1024 => record.problem="too_large_for_task",
        Err(_) => {}
    }
    record
}
fn parse(uid: u32, raw: &[u8]) -> Result<Incoming> {
    if raw.len() > 256 * 1024 { bail!("incoming mail exceeds 256 KiB"); }
    let parsed = mailparse::parse_mail(raw)?;
    let headers = &parsed.headers;
    let from = headers.get_first_value("From").context("missing From")?;
    let sender = mailparse::addrparse(&from)?.iter().find_map(|a| match a {
        MailAddr::Single(s) => Some(s.addr.trim().to_ascii_lowercase()), _ => None,
    }).context("missing sender address")?;
    let mid = headers.get_first_value("Message-ID").context("missing Message-ID")?.trim().to_string();
    if !mid.starts_with('<') || !mid.ends_with('>') { bail!("invalid Message-ID"); }
    let mut refs=Vec::new();
    for value in [headers.get_first_value("In-Reply-To"),headers.get_first_value("References")]
        .into_iter().flatten() {
        refs.extend(value.split_whitespace().filter(|v|v.starts_with('<') && v.ends_with('>'))
            .map(str::to_string));
    }
    let auto = headers.get_first_value("Auto-Submitted")
        .map(|v| !v.eq_ignore_ascii_case("no")).unwrap_or(false)
        || headers.get_first_value("Precedence")
            .map(|v|["bulk","list","junk"].contains(&v.to_ascii_lowercase().as_str()))
            .unwrap_or(false);
    let ema_kind=headers.get_first_value("X-EMA-Kind").map(|v|v.trim().to_string()).filter(|v|!v.is_empty());
        let ema_message_id=headers.get_first_value("X-EMA-Message-Id").map(|v|v.trim().to_string()).filter(|v|!v.is_empty());
        Ok(Incoming{uid,message_id:mid,sender,subject:headers.get_first_value("Subject")
            .unwrap_or_else(||"Task".into()),body:plaintext(&parsed)?.chars().take(20_000).collect(),
            references:refs,automated:auto,ema_kind,ema_message_id})
}
fn login(c:&MailSettings,pass:&str)->Result<imap::Session<native_tls::TlsStream<std::net::TcpStream>>> {
    let tls=native_tls::TlsConnector::builder().build()?;
    let client=imap::connect((&*c.imap_host,c.imap_port),&c.imap_host,&tls)?;
    client.login(&c.username,pass).map_err(|e|anyhow::anyhow!("{}",e.0))
}
/// Validate the IMAP login without fetching or marking any message read.
pub fn check_connection(c: &MailSettings, pass: &str) -> Result<()> {
    let mut session = login(c, pass)?;
    session.noop()?;
    session.logout()?;
    Ok(())
}
pub fn receive(c:&MailSettings,pass:&str,last_uid:u32,known_validity:u32)->Result<(Vec<FetchedMail>,u32,u32)> {
    let mut session=login(c,pass)?;
    let mailbox = session.select("INBOX")?;
    let validity = mailbox.uid_validity.unwrap_or(0);
    // First installation or UIDVALIDITY reset: start from future mail only.
    if known_validity == 0 || known_validity != validity {
        return Ok((Vec::new(),mailbox.uid_next.unwrap_or(1).saturating_sub(1),validity));
    }
    let cursor = last_uid;
    let query = format!("UID {}:*", cursor.saturating_add(1));
    let mut uids:Vec<u32>=session.uid_search(&query)?.into_iter().filter(|uid| *uid > cursor).collect();
    uids.sort_unstable();
    let mut result=Vec::new();
    let mut watermark=cursor;
    for uid in uids.into_iter().take(30) {
        watermark=uid;
        let fetched = session.uid_fetch(uid.to_string(),"BODY.PEEK[]")?;
        if let Some(raw)=fetched.iter().find_map(|msg| msg.body()) {
            result.push(archive(uid,raw));
        } else {
            result.push(FetchedMail {uid,message_id:String::new(),sender:String::new(),
                recipient:String::new(),cc:String::new(),references:String::new(),original_date:String::new(),
                subject:String::new(),body:String::new(),archive_note:"unavailable".into(),
                message:None,problem:"unavailable"});
        }
    }
    let _=session.logout();
    Ok((result,watermark,validity))
}
pub fn acknowledge(c:&MailSettings,pass:&str,uids:&[u32])->Result<()> {
    if uids.is_empty(){return Ok(());}
    let mut session=login(c,pass)?;
    session.select("INBOX")?;
    for uid in uids {session.uid_store(uid.to_string(),"+FLAGS (\\Seen)")?;}
    let _=session.logout();
    Ok(())
}
pub fn send(c:&MailSettings,pass:&str,o:&Outbound)->Result<()> {
    if !crate::email_format::KINDS.contains(&o.kind.as_str()) { bail!("invalid outbound mail kind: {}", o.kind); }
    let from:Mailbox=c.address.parse()?;
    let to:Mailbox=o.recipient.parse()?;
    let mut builder=Message::builder().from(from).to(to).subject(&o.subject)
        .message_id(Some(o.message_id.clone()));
    if !o.in_reply_to.is_empty() { builder=builder.in_reply_to(o.in_reply_to.clone()); }
    if !o.references.is_empty() { builder=builder.references(o.references.clone()); }
    // Self-identification headers: survive QQ Mail's Message-ID rewrite so our own
    // outbound mail coming back into the INBOX can be recognized and dropped as echo.
    let msg = builder.body(crate::email_format::envelope(&o.kind,&o.message_id,&o.body))?;
    let mut msg = msg;
    msg.headers_mut().insert_raw(HeaderValue::new(HeaderName::new_from_ascii_str("X-EMA-Version"), "1".into()));
    if !o.kind.is_empty() {
        msg.headers_mut().insert_raw(HeaderValue::new(HeaderName::new_from_ascii_str("X-EMA-Kind"), o.kind.clone()));
    }
    if !o.message_id.is_empty() {
        msg.headers_mut().insert_raw(HeaderValue::new(HeaderName::new_from_ascii_str("X-EMA-Message-Id"), o.message_id.clone()));
    }
    let builder = if c.smtp_port == 587 {
        SmtpTransport::starttls_relay(&c.smtp_host)?
    } else {
        SmtpTransport::relay(&c.smtp_host)?
    };
    let transport=builder.port(c.smtp_port)
        .credentials(Credentials::new(c.username.clone(),pass.into())).build();
    transport.send(&msg)?;
    Ok(())
}
pub fn redact_smtp_error(error: &str, password: &str) -> String {
    let text = if password.is_empty() { error.to_string() } else { error.replace(password, "[redacted]") };
    text.chars().take(2_000).collect()
}
