//! Local replies target stored native conversations; arbitrary mail is never sent.
use crate::{Gateway, cli, db::MailRecord, mail::Incoming};
use anyhow::{ensure, Context, Result};
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct ReplyContext {
    pub agent: String,
    pub cwd: String,
    pub native_session: String,
    pub session_id: Option<String>,
    pub blocked: Option<&'static str>,
}

impl Gateway {
    fn reply_target(&self, record: &MailRecord) -> Result<Option<(ReplyContext, String)>> {
        if matches!(record.kind.as_str(), "unrelated" | "invalid" | "test" | "error") { return Ok(None); }
        let refs=vec![record.message_id.clone()];
        let session=self.database.find_by_references(&refs)?;
        let hook=self.database.find_hook_report(&refs)?;
        let (agent,cwd,native,owner,id)=if let Some(s)=session {
            (s.agent,s.cwd.unwrap_or_default(),s.agent_session.unwrap_or_default(),s.owner,Some(s.id))
        } else if let Some(h)=hook {
            (h.agent,h.cwd.unwrap_or_default(),h.native_session,self.settings().mail.address,None)
        } else { return Ok(None); };
        if native.trim().is_empty() { return Ok(None); }
        let agent=if agent=="claude" {"cc".to_string()} else {agent};
        let cfg=self.settings();
        let blocked=if !cfg.enabled {Some("paused")}
            else if !cfg.is_allowed(&owner) {Some("owner_not_allowed")}
            else if agent!="alma" && cli::resolve(&agent,&cfg).is_none() {Some("not_installed")}
            else if agent!="alma" && !std::path::Path::new(&cwd).is_dir() {Some("directory_unavailable")}
            else if matches!(agent.as_str(),"pi"|"omp") && !std::path::Path::new(&native).is_file() {Some("session_unavailable")}
            else {None};
        Ok(Some((ReplyContext {agent,cwd,native_session:native,session_id:id,blocked},owner)))
    }
    pub fn mail_reply_context(&self, record_id:&str) -> Result<Option<ReplyContext>> {
        let record=self.database.mail_record(record_id)?.context("Mail record not found")?;
        Ok(self.reply_target(&record)?.map(|(context,_)|context))
    }
    pub async fn reply_to_mail(&self, record_id:&str, request_id:&str, body:&str) -> Result<String> {
        let _activity = self.begin_activity()?;
        let id=uuid::Uuid::parse_str(request_id).context("Invalid reply request ID")?;
        let body=body.trim();
        ensure!(!body.is_empty(),"Reply cannot be empty");
        ensure!(body.chars().count()<=12_000,"Reply exceeds 12000 characters");
        // A pasted machine marker must not silently turn a local instruction into an echo.
        ensure!(crate::email_format::marker_id(body).is_none(),"Enter your instruction, not an EMA machine marker");
        let record=self.database.mail_record(record_id)?.context("Mail record not found")?;
        let (context,owner)=self.reply_target(&record)?.context("No native Agent conversation is linked to this mail")?;
        ensure!(context.blocked.is_none(),"Reply unavailable: {}",context.blocked.unwrap_or(""));
        let cfg=self.settings();
        let mid=format!("<ema-ui-{id}@localhost>");
        self.database.record_local_reply(&mid,&owner,&cfg.mail.address,&record.message_id,&record.subject,body)?;
        if self.database.find_by_references(&[mid.clone()])?.is_some() {
            self.database.local_reply_result(&mid,"duplicate","")?;
            return Ok("duplicate".into());
        }
        let incoming=Incoming {uid:0,message_id:mid.clone(),sender:owner,subject:record.subject,
            body:body.to_string(),references:vec![record.message_id],automated:false,ema_kind:None,ema_message_id:None};
        match self.on_message(&cfg,incoming).await {
            Ok(status)=>{
                self.database.local_reply_result(&mid,status,"")?;
                ensure!(matches!(status,"queued"|"duplicate"|"awaiting_directory"),"Reply was not accepted: {status}");
                Ok(status.into())
            }
            Err(error)=>{
                self.database.local_reply_result(&mid,"retry","Local submission failed; retry with the same request ID")?;
                Err(error)
            }
        }
    }
}
