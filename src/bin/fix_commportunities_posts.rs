use nmg_league_bot::db::raw_diesel_cxn_from_env;
use nmg_league_bot::models::bracket_race_infos::BracketRaceInfo;
use nmg_league_bot::schema::bracket_race_infos;
use diesel::prelude::*;
use twilight_http::Client;
use twilight_model::channel::embed::Embed;
use twilight_util::builder::embed::EmbedFooterBuilder;
use nmg_league_bot::ChannelConfig;
use nmg_league_bot::constants::TOKEN_VAR;
use nmg_league_bot::utils::{env_var, race_to_nice_embeds};

#[tokio::main]
async fn main() {
    dotenv::dotenv().unwrap();
    let mut db = raw_diesel_cxn_from_env().unwrap();
    let infos: Vec<BracketRaceInfo> = bracket_race_infos::table.load(&mut db).unwrap();

    let client = Client::new(env_var(TOKEN_VAR));
    let channel_config = ChannelConfig::new_from_env();

    for info in infos {
        println!("info: {:?}", info);
        let msg_id = match info.get_commportunities_message_id() {
            None => {continue;}
            Some(m) => {m}
        };
        let fields = race_to_nice_embeds(&info, &mut db).map_err(|e| e.to_string()).unwrap();
        let embeds = vec![Embed {
            author: None,
            color: Some(0x00b0f0),
            description: None,
            fields,
            footer: Some(EmbedFooterBuilder::new("React to volunteer").build()),
            image: None,
            kind: "rich".to_string(),
            provider: None,
            thumbnail: None,
            timestamp: None,
            title: Some(format!("New match available for commentary")),
            url: None,
            video: None,
        }];
        let msg = client
            .update_message(channel_config.commportunities, msg_id)
            .embeds(Some(&embeds)).unwrap()
            .exec()
            .await
            .unwrap();
        println!("{msg:?}");
    }
}