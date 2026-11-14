use anyhow::{bail, Context, Result};
use lettre::{message::Mailbox, transport::smtp::authentication::Credentials, Message, SmtpTransport, Transport};
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
}
fn plaintext(part: &ParsedMail<'_>) -> Result<String> {
    if part.ctype.mimetype.eq_ignore_ascii_case("text/plain") { return Ok(part.get_body()?); }
    for sub in &part.subparts {
        if let Ok(text) = plaintext(sub) { if !text.is_empty() { return Ok(text); } }
    }
    Ok(String::new())
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
    Ok(Incoming{uid,message_id:mid,sender,subject:headers.get_first_value("Subject")
        .unwrap_or_else(||"Task".into()),body:plaintext(&parsed)?.chars().take(20_000).collect(),
        references:refs,automated:auto})
}
fn login(c:&MailSettings,pass:&str)->Result<imap::Session<native_tls::TlsStream<std::net::TcpStream>>> {
    let tls=native_tls::TlsConnector::builder().build()?;
    let client=imap::connect((&*c.imap_host,c.imap_port),&c.imap_host,&tls)?;
    client.login(&c.username,pass).map_err(|e|anyhow::anyhow!("{}",e.0))
}
pub fn receive(c:&MailSettings,pass:&str)->Result<Vec<Incoming>> {
    let mut session=login(c,pass)?;
    session.select("INBOX")?;
    let mut uids:Vec<u32>=session.uid_search("UNSEEN")?.into_iter().collect();
    uids.sort_unstable();
    let mut result=Vec::new();
    for uid in uids.into_iter().take(30) {
        for msg in session.uid_fetch(uid.to_string(),"BODY.PEEK[]")?.iter() {
            if let Some(raw)=msg.body() {
                match parse(uid,raw) {
                    Ok(v)=>result.push(v),
                    Err(err)=>{
                        tracing::warn!(uid,"ignored malformed email: {err}");
                        session.uid_store(uid.to_string(),"+FLAGS (\\Seen)")?;
                    }
                }
            }
        }
    }
    let _=session.logout();
    Ok(result)
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
    let from:Mailbox=c.address.parse()?;
    let to:Mailbox=o.recipient.parse()?;
    let msg=Message::builder().from(from).to(to).subject(&o.subject)
        .message_id(Some(o.message_id.clone()))
        .in_reply_to(o.in_reply_to.clone())
        .references(o.references.clone())
        .body(o.body.clone())?;
    let transport=SmtpTransport::relay(&c.smtp_host)?.port(c.smtp_port)
        .credentials(Credentials::new(c.username.clone(),pass.into())).build();
    transport.send(&msg)?;
    Ok(())
}
