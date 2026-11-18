//! Pure policy decision core. Caller MUST supply a cryptographically verified
//! principal; caller-controlled tenant / role strings are not authentication.
use serde::{Deserialize, Serialize};
use anyhow::{bail, Result};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum Role { Owner, Admin, Editor, Reviewer, Viewer, Agent }
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerifiedPrincipal {
    pub account_id: String,
    pub tenant_id: String,
    pub workspace_id: String,
    pub role: Role,
    pub allowed_scopes: Vec<String>,
}
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum Permission { List, Read, Comment, Write, Archive, Restore, ManageMembers, Publish }

pub fn authorize(p:&VerifiedPrincipal,tenant:&str,workspace:&str,
                 permission:Permission,resource_scope:&str,sensitive:bool)->Result<()> {
    if p.tenant_id!=tenant || p.workspace_id!=workspace {bail!("tenant/workspace boundary denied");}
    if p.role==Role::Agent && !p.allowed_scopes.iter().any(|s| resource_scope==s || (s=="read:*" && matches!(permission,Permission::List|Permission::Read))) {
        bail!("agent scope denied");
    }
    if sensitive && matches!(p.role,Role::Viewer|Role::Agent) {bail!("sensitive resource denied");}
    let allowed=match p.role {
        Role::Owner|Role::Admin=>true,
        Role::Editor=>!matches!(permission,Permission::ManageMembers),
        Role::Reviewer=>matches!(permission,Permission::List|Permission::Read|Permission::Comment),
        Role::Viewer=>matches!(permission,Permission::List|Permission::Read),
        Role::Agent=>matches!(permission,Permission::List|Permission::Read|Permission::Comment|Permission::Write),
    };
    if !allowed {bail!("role denied");}
    Ok(())
}

#[cfg(test)] mod tests {
    use super::*;
    fn principal(role:Role)->VerifiedPrincipal{VerifiedPrincipal{account_id:"user".into(),tenant_id:"a".into(),workspace_id:"w".into(),role,allowed_scopes:vec!["read:*".into()]}}
    #[test] fn ut_auth_cross_tenant(){assert!(authorize(&principal(Role::Owner),"b","w",Permission::Read,"file",false).is_err());}
    #[test] fn ut_auth_reviewer_no_write(){assert!(authorize(&principal(Role::Reviewer),"a","w",Permission::Write,"file",false).is_err());}
    #[test] fn ut_auth_viewer_sensitive(){assert!(authorize(&principal(Role::Viewer),"a","w",Permission::Read,"file",true).is_err());}
    #[test] fn ut_auth_agent_scoped(){assert!(authorize(&principal(Role::Agent),"a","w",Permission::Write,"file:42",false).is_err());}
}
