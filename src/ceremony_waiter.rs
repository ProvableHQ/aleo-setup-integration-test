use mpmc_bus::Receiver;

use crate::{
    waiter::{MessageWaiter, WaiterClosureResult, WaiterJoinCondition},
    CeremonyMessage,
};

#[derive(Copy, Clone)]
pub struct ContributionWaiterState {
    /// Record of how many contributions have been made so far since
    /// the waiter was started.
    contributions: u64,
}

pub fn spawn_contribution_waiter<J>(
    after_contributions: u64,
    on_messages_received: J,
    rx: Receiver<CeremonyMessage>,
) -> MessageWaiter<CeremonyMessage>
where
    J: FnOnce() -> eyre::Result<()> + Send + 'static,
{
    let span = tracing::error_span!("contribution_waiter");
    MessageWaiter::spawn(
        move |message, mut state| {
            let _guard = span.enter();
            match message {
                CeremonyMessage::SuccessfulContribution {
                    contributor: _,
                    chunk: _,
                } => state.contributions += 1,
                _ => {}
            }

            if state.contributions >= after_contributions {
                WaiterClosureResult::Join(WaiterJoinCondition::MessagesReceived)
            } else {
                WaiterClosureResult::Continue(state)
            }
        },
        on_messages_received,
        ContributionWaiterState { contributions: 0 },
        rx,
    )
}
