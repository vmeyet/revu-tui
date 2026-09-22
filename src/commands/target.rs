use crate::forge::{Forge, MrKey};
use crate::mrref::{MrRef, ProjectRef, Target};
use anyhow::{Context, Result};

/// The MR whatever the user typed names: a reference, a URL, or the open MR of the current branch.
pub async fn resolve(forge: &Forge, arg: Option<&str>) -> Result<MrKey> {
    let cwd = std::env::current_dir()?;
    match Target::from_arg(arg, &cwd)? {
        Target::Mr(MrRef { project: ProjectRef::Id(id), iid }) => Ok(MrKey::new(forge.project_path(id).await?, iid)),
        Target::Mr(MrRef { project: ProjectRef::Path(path), iid }) => Ok(MrKey::new(path, iid)),
        Target::Branch { project, branch } => {
            let number = forge.mr_for_branch(&project, &branch).await?.with_context(|| format!("no open MR for branch {branch}"))?;
            Ok(MrKey::new(project, number))
        }
    }
}
