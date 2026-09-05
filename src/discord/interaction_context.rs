use crate::discord::discord_state::{DiscordOperations, DiscordState};
use crate::discord::interaction_diagnostics::InteractionDiagnostics;
use std::sync::Arc;
use twilight_model::gateway::payload::incoming::InteractionCreate;
use twilight_model::http::interaction::InteractionResponse;

/// Shared services and metadata for one interaction. The payload is extracted before dispatch
/// and passed separately to handlers. Diagnostics retain the original gateway receipt time.
///
/// Handlers borrow this context; deferred work keeps it alive through an Arc. Database
/// connections are acquired by the work that needs them, not when the context is created.
pub(crate) struct InteractionContext<S = DiscordState> {
    pub(crate) state: Arc<S>,
    pub(crate) interaction: Box<InteractionCreate>,
    pub(crate) diagnostics: InteractionDiagnostics,
}

impl<S: DiscordOperations> InteractionContext<S> {
    /// Send an initial response for this interaction. The caller owns acknowledgment timing
    /// and error reporting, and must not send another initial response after deferring.
    pub(crate) async fn respond(&self, response: &InteractionResponse) -> Result<(), String> {
        self.state
            .create_response_err_to_str(self.interaction.id, &self.interaction.token, response)
            .await
    }
}
