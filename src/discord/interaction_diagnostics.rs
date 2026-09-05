use chrono::{DateTime, Utc};
use std::time::Instant;
use twilight_model::application::interaction::{Interaction, InteractionData};

/// Captured before dispatch and before handlers remove interaction data. Never stores the token
/// or command arguments. Local durations are measured with a monotonic clock.
#[derive(Clone)]
pub(crate) struct InteractionDiagnostics {
    context: String,
    received_at: Instant,
}

impl InteractionDiagnostics {
    pub(crate) fn new(interaction: &Interaction) -> Self {
        Self::new_at(interaction, Instant::now(), Utc::now())
    }

    fn new_at(
        interaction: &Interaction,
        received_at: Instant,
        received_utc: DateTime<Utc>,
    ) -> Self {
        let name = match interaction.data.as_ref() {
            Some(InteractionData::ApplicationCommand(command)) => format!("/{}", command.name),
            Some(InteractionData::MessageComponent(component)) => {
                format!("component {}", component.custom_id)
            }
            Some(InteractionData::ModalSubmit(modal)) => format!("modal {}", modal.custom_id),
            _ => "unknown".to_string(),
        };
        #[allow(deprecated)]
        let channel_id = interaction
            .channel
            .as_ref()
            .map(|c| c.id)
            .or(interaction.channel_id);
        Self {
            context: format!(
                "Interaction: {name} ({:?})\nID: {}; user: {}; guild: {}; channel: {}\nReceived by host: {}",
                interaction.kind,
                interaction.id,
                interaction.author_id().map(|id| id.to_string()).unwrap_or_else(|| "unknown".into()),
                interaction.guild_id.map(|id| id.to_string()).unwrap_or_else(|| "DM".into()),
                channel_id.map(|id| id.to_string()).unwrap_or_else(|| "unknown".into()),
                received_utc.to_rfc3339(),
            ),
            received_at,
        }
    }

    /// Keep the diagnostics at the front and leave room within Discord's 2,000-character limit.
    pub(crate) fn report(
        &self,
        stage: &str,
        request_started: Instant,
        request_finished: Instant,
        details: &str,
    ) -> String {
        let before_request_ms = request_started.duration_since(self.received_at).as_millis();
        let request_ms = request_finished.duration_since(request_started).as_millis();
        let report = format!(
            "{}\nStage: {stage}\nReceipt to request: {before_request_ms} ms; request elapsed: {request_ms} ms\n{details}",
            self.context,
        );
        truncate_report(&report)
    }

    pub(crate) fn initial_response_failure_report(
        &self,
        stage: &str,
        request_started: Instant,
        request_finished: Instant,
        details: &str,
    ) -> String {
        const DEADLINE_MS: u128 = 3_000;
        let before_request_ms = request_started.duration_since(self.received_at).as_millis();
        let request_ms = request_finished.duration_since(request_started).as_millis();
        let diagnosis = if before_request_ms >= DEADLINE_MS {
            "The bot began responding after Discord's 3-second deadline. The delay occurred before the response request."
        } else if before_request_ms.saturating_add(request_ms) >= DEADLINE_MS {
            "The bot began responding before the deadline, but the request finished after it. Outbound HTTP latency may have consumed the remaining time."
        } else {
            "Host timing does not explain the failure: the response request finished within 3 seconds of receipt. Possible causes include delayed delivery or a Discord-side transient."
        };
        self.report(
            stage,
            request_started,
            request_finished,
            &format!("Diagnosis: {diagnosis}\n{details}"),
        )
    }
}

fn truncate_report(report: &str) -> String {
    if report.len() <= 1_900 {
        return report.to_owned();
    }

    let end = report.floor_char_boundary(1_900);
    format!("{}\n[truncated]", &report[..end])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_includes_timing_and_context_without_token_or_arguments() {
        let received_utc = DateTime::from_timestamp_millis(1_700_000_000_000).unwrap();
        let interaction: Interaction = serde_json::from_value(serde_json::json!({
            "id": "123456789123456789", "application_id": "123", "type": 2,
            "token": "secret-interaction-token", "guild_id": "456",
            "channel": { "id": "789", "type": 0 },
            "data": { "id": "123", "name": "submit_qualifier", "type": 1,
                "options": [{ "name": "vod", "type": 3, "value": "private-vod-url" }] },
            "entitlements": [], "authorizing_integration_owners": {},
        }))
        .unwrap();
        let now = Instant::now();
        let diagnostics = InteractionDiagnostics::new_at(&interaction, now, received_utc);
        let report = diagnostics.report(
            "initial response",
            now,
            now,
            "Handler: returned Ok\nUnknown interaction (10062)",
        );
        assert!(report.contains("/submit_qualifier"));
        assert!(report.contains("guild: 456; channel: 789"));
        assert!(report.contains("Received by host: 2023-11-14T22:13:20+00:00"));
        assert!(report.contains("Receipt to request: 0 ms"));
        assert!(report.contains("Unknown interaction (10062)"));
        assert!(!report.contains("secret-interaction-token"));
        assert!(!report.contains("private-vod-url"));
    }

    #[test]
    fn oversized_reports_preserve_diagnostics_and_fit_discord_limit() {
        let report = truncate_report(&format!(
            "Interaction: /submit_qualifier\n{}",
            "🦊".repeat(2_000)
        ));
        assert!(report.starts_with("Interaction: /submit_qualifier\n"));
        assert!(report.ends_with("[truncated]"));
        assert!(report.len() <= 2_000);
    }

    #[test]
    fn initial_response_failure_explains_a_missed_deadline() {
        let interaction: Interaction = serde_json::from_value(serde_json::json!({
            "id": "123456789123456789", "application_id": "123", "type": 2,
            "token": "secret", "data": { "id": "123", "name": "test", "type": 1 },
            "entitlements": [], "authorizing_integration_owners": {},
        }))
        .unwrap();
        let received = Instant::now();
        let diagnostics = InteractionDiagnostics::new_at(&interaction, received, Utc::now());
        let report = diagnostics.initial_response_failure_report(
            "initial response",
            received + std::time::Duration::from_millis(4_013),
            received + std::time::Duration::from_millis(4_174),
            "Unknown interaction (10062)",
        );
        assert!(report.contains("began responding after Discord's 3-second deadline"));
        assert!(report.contains("delay occurred before the response request"));
    }

    #[test]
    fn initial_response_failure_says_when_local_timing_does_not_explain_it() {
        let interaction: Interaction = serde_json::from_value(serde_json::json!({
            "id": "123456789123456789", "application_id": "123", "type": 2,
            "token": "secret", "data": { "id": "123", "name": "test", "type": 1 },
            "entitlements": [], "authorizing_integration_owners": {},
        }))
        .unwrap();
        let received = Instant::now();
        let diagnostics = InteractionDiagnostics::new_at(&interaction, received, Utc::now());
        let report = diagnostics.initial_response_failure_report(
            "initial response",
            received + std::time::Duration::from_millis(20),
            received + std::time::Duration::from_millis(181),
            "Unknown interaction (10062)",
        );
        assert!(report.contains("Host timing does not explain the failure"));
        assert!(report.contains("delayed delivery or a Discord-side transient"));
    }
}
