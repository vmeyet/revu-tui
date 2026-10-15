//! Everything GitHub answers best in GraphQL: the queue, one PR, its threads, and the pending
//! review that holds my drafts until `publish` submits it.
mod drafts;
mod pr;
mod queue;
#[cfg(test)]
mod test_support;
mod threads;
