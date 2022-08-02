pub(crate) mod bot;
mod webhooks;

use crate::discord::discord_state::DiscordState;
use crate::models::race::Race;
use crate::models::race_run::RaceRun;
use std::sync::Arc;
use twilight_model::application::component::button::ButtonStyle;
use twilight_model::application::component::{ActionRow, Component};
pub(crate) use webhooks::Webhooks;

pub(crate) mod discord_state;
mod interactions;

extern crate rand;
extern crate sqlx;
extern crate tokio;

const CUSTOM_ID_START_RUN: &str = "start_run";
const CUSTOM_ID_FINISH_RUN: &str = "finish_run";
const CUSTOM_ID_FORFEIT_RUN: &str = "forfeit_run";

const CUSTOM_ID_FORFEIT_MODAL: &str = "forfeit_modal";
const CUSTOM_ID_FORFEIT_MODAL_INPUT: &str = "forfeit_modal_input";

const CUSTOM_ID_VOD_READY: &str = "vod_ready";

const CUSTOM_ID_VOD_MODAL: &str = "vod_modal";
const CUSTOM_ID_VOD_MODAL_INPUT: &str = "vod";

const CUSTOM_ID_USER_TIME: &str = "user_time";
const CUSTOM_ID_USER_TIME_MODAL: &str = "user_time_modal";

const CREATE_RACE_CMD: &str = "create_race";
const CANCEL_RACE_CMD: &str = "cancel_race";
const ADMIN_ROLE_NAME: &'static str = "Admin";

/// DM the player & save the run model if the DM sends successfully
pub(crate) async fn notify_racer(
    race_run: &mut RaceRun,
    race: &Race,
    state: &Arc<DiscordState>,
) -> Result<(), String> {
    let uid = race_run.racer_id()?;
    if Some(uid) == state.cache.current_user().map(|cu| cu.id) {
        println!("Not sending messages to myself");
        race_run.contact_succeeded();
        race_run.save(&state.pool).await?;
        return Ok(());
    }
    let dm = state.get_private_channel(uid).await?;
    let content = format!(
        "Hello, your asynchronous race is now ready.
When you're ready to begin your race, click \"Start run\" and you will be given
filenames to enter.

If anything goes wrong, tell an admin there was an issue with race `{}`",
        race.uuid
    );

    let resp = state
        .client
        .create_message(dm)
        .components(&[Component::ActionRow(ActionRow {
            components: vec![interactions::button_component(
                "Start run",
                CUSTOM_ID_START_RUN,
                ButtonStyle::Primary,
            )],
        })])
        .and_then(|cm| cm.content(&content))
        .map_err(|e| e.to_string())?
        .exec()
        .await
        .map_err(|e| e.to_string())?;

    if resp.status().is_success() {
        let msg = resp.model().await.map_err(|e| e.to_string())?;
        race_run.set_message_id(msg.id.get());
        race_run.contact_succeeded();
        race_run.save(&state.pool).await
    } else {
        Err(format!("Error sending message: {}", resp.status()))
    }
}
