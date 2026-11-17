use anyhow::Result;
use std::{collections::HashSet, path::{Path,PathBuf}};
use crate::{config::Settings,db::Database};

pub enum Resolution { Found(String), Clarify }

fn project_hint(subject:&str,body:&str)->bool {
    let text=format!("{} {}",subject,body).to_lowercase();
    ["项目","仓库","代码库","repository"," repo "," project ","git clone"].iter().any(|w|text.contains(w))
}
fn mentions(haystack:&str,name:&str)->bool {
    if name.len()<3 { return false; }
    haystack.match_indices(name).any(|(start,_)| {
        let before=haystack[..start].chars().next_back();
        let after=haystack[start+name.len()..].chars().next();
        let boundary=|c:Option<char>|c.map(|x| !x.is_alphanumeric() && x!='_' && x!='-').unwrap_or(true);
        boundary(before)&&boundary(after)
    })
}
fn candidate_dirs(config:&Settings,db:&Database)->Result<Vec<PathBuf>> {
    let mut candidates=Vec::new();
    for prior in db.known_workspaces()? { candidates.push(PathBuf::from(prior)); }
    for legacy in &config.projects { candidates.push(legacy.path.clone()); }
    if let Some(home)=std::env::var_os("HOME").or_else(||std::env::var_os("USERPROFILE")) {
        let home=PathBuf::from(home);
        for root in ["code","Code","projects","Projects","dev","Developer","work","src","Documents/GitHub"] {
            if let Ok(entries)=std::fs::read_dir(home.join(root)) {
                for entry in entries.flatten().take(300) {
                    if entry.file_type().map(|kind|kind.is_dir()).unwrap_or(false) { candidates.push(entry.path()); }
                }
            }
        }
    }
    let mut seen=HashSet::new();
    Ok(candidates.into_iter().filter_map(|p|p.canonicalize().ok()).filter(|p|p.is_dir()&&seen.insert(p.clone())).collect())
}

/// Resolve a working directory using known local directories.
pub fn resolve(data_dir:&Path,db:&Database,config:&Settings,subject:&str,body:&str,key:&str)->Result<Resolution> {
    let text=format!("{} {}",subject,body).to_lowercase();
    let matched:Vec<PathBuf>=candidate_dirs(config,db)?.into_iter().filter(|p| {
        p.file_name().and_then(|s|s.to_str()).map(|name|mentions(&text,&name.to_lowercase())).unwrap_or(false)
    }).collect();
    if matched.len()==1 { return Ok(Resolution::Found(matched[0].to_string_lossy().into_owned())); }
    if matched.len()>1 || project_hint(subject,body) { return Ok(Resolution::Clarify); }
    // Non-project requests use an independent, stable session working directory.
    let safe=key.chars().filter(|c|c.is_ascii_alphanumeric()).take(64).collect::<String>();
    let path=data_dir.join("sessions").join(if safe.is_empty() {"temporary"} else {&safe});
    std::fs::create_dir_all(&path)?;
    Ok(Resolution::Found(path.canonicalize()?.to_string_lossy().into_owned()))
}
