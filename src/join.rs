use std::sync::{Arc, Mutex};

/// A thread joiner that can be joined using [join_multiple()].
pub trait MultiJoinable: std::fmt::Debug {
    fn join(self: Box<Self>) -> std::thread::Result<()>;
}

/// Concatinate multiple joins into a [MultiJoinable] implementation.
#[derive(Debug)]
pub struct JoinMultiple {
    joins: Vec<Box<dyn MultiJoinable>>,
}

impl JoinMultiple {
    /// Construct a new [ConcatJoin].
    pub fn new(joins: Vec<Box<dyn MultiJoinable>>) -> Self {
        Self { joins }
    }

    /// Joins all joins contained within this [JoinMultiple].
    pub fn join(self) -> std::thread::Result<()> {
        join_multiple(self.joins)
    }
}

impl MultiJoinable for JoinMultiple {
    fn join(self: Box<Self>) -> std::thread::Result<()> {
        JoinMultiple::join(*self)
    }
}

/// A [MultiJoinable] implementation for a join which may be
/// constructed at some point in the future on different thread. If
/// the joiner has not yet been registered with
/// [JoinLater::register()], then it will join immediately.
#[derive(Debug, Clone)]
pub struct JoinLater {
    join: Arc<Mutex<Option<Box<dyn MultiJoinable + Send + 'static>>>>,
}

impl Default for JoinLater {
    fn default() -> Self {
        Self::new()
    }
}

impl JoinLater {
    pub fn new() -> Self {
        Self {
            join: Arc::new(Mutex::new(None)),
        }
    }

    /// Register a `join` to be joined when this [JoinLater] is joined.
    pub fn register<J: MultiJoinable + Send + 'static>(&self, join: J) {
        match self.join.lock() {
            Ok(mut guard) => *guard = Some(Box::new(join)),
            Err(error) => tracing::error!("Error obtaining lock on join to register it: {}", error),
        }
    }

    /// Join on the join that was registered with [Self::register()]
    /// otherwise if the has not been set, join immediately.
    pub fn join(self) -> std::thread::Result<()> {
        match self.join.lock() {
            Ok(mut guard) => {
                if let Some(join) = guard.take() {
                    join.join()
                } else {
                    Ok(())
                }
            }
            Err(error) => {
                tracing::error!("Error obtaining lock on join to join it: {}", error);
                Ok(())
            }
        }
    }
}

impl MultiJoinable for JoinLater {
    fn join(self: Box<Self>) -> std::thread::Result<()> {
        JoinLater::join(*self)
    }
}

/// Join multiple [MonitorProcessJoin]s.
#[tracing::instrument(level = "error", skip(joins))]
pub fn join_multiple(mut joins: Vec<Box<dyn MultiJoinable>>) -> std::thread::Result<()> {
    while let Some(join) = joins.pop() {
        join.join()?;
        tracing::debug!("Joins remaining: {:?}", joins);
    }
    tracing::debug!("Joined all processes");
    Ok(())
}
