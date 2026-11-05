//! The local backend: the branch of a worktree read with `git`, with no host and no token.
mod split;
#[cfg(test)]
pub(crate) mod tests;
