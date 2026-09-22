use crate::api::Client;
use crate::mrref::{MrRef, ProjectRef, Target};
use anyhow::{Context, Result};

/// The numeric project id and iid every REST call needs, from whatever the user typed.
pub async fn resolve(gitlab: &Client, arg: Option<&str>) -> Result<(u64, u64)> {
    let cwd = std::env::current_dir()?;
    match Target::from_arg(arg, &cwd)? {
        Target::Mr(MrRef { project: ProjectRef::Id(id), iid }) => Ok((id, iid)),
        Target::Mr(MrRef { project: ProjectRef::Path(path), iid }) => Ok((gitlab.project(&path).await?.id, iid)),
        Target::Branch { project, branch } => {
            let project_id = gitlab.project(&project).await?.id;
            let iid = gitlab.mr_for_branch(project_id, &branch).await?.with_context(|| format!("no open MR for branch {branch}"))?;
            Ok((project_id, iid))
        }
    }
}
