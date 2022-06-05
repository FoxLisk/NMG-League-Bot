extern crate serenity;

use serenity::client::{ClientBuilder, EventHandler, Context};
use serenity::prelude::GatewayIntents;
use serenity::model::channel::Message;
use serenity::{async_trait, Client, Error};
use serenity::model::guild::{Guild, PartialGuild};
use serenity::model::gateway::Ready;
use serenity::model::id::{GuildId, ApplicationId};
use serenity::model::interactions::application_command::{ApplicationCommand, ApplicationCommandType, ApplicationCommandInteraction};
use std::collections::{HashMap, HashSet};
use serenity::http::routing::RouteInfo::CreateGuildApplicationCommand;
use serenity::builder::{CreateApplicationCommand, CreateApplicationCommands, CreateInteractionResponse};
use serenity::model::interactions::{Interaction, InteractionResponseType};
use serenity::http::Http;

const TOKEN_VAR: &str = "DISCORD_TOKEN";
const APPLICATION_ID_VAR: &str = "APPLICATION_ID";

struct EchoHandler;

impl EchoHandler {
    fn application_commands(commands: &mut CreateApplicationCommands) -> &mut CreateApplicationCommands {
        commands.create_application_command(|command|
            command.name("test")
                .description("test description")
                .kind(ApplicationCommandType::ChatInput)
        )
    }

    async fn handle_test(&self, http: impl AsRef<Http>,  interaction: ApplicationCommandInteraction) {
        interaction.create_interaction_response(http, |ir|
            ir.kind(InteractionResponseType::ChannelMessageWithSource)
                .interaction_response_data(|data| data.content("test response"))
        ).await;

    }

}

#[async_trait]
impl EventHandler for EchoHandler {
    async fn cache_ready(&self, _ctx: Context, _guilds: Vec<GuildId>) {
        println!("Cache ready");
    }

    async fn guild_create(&self, ctx: Context, guild: Guild, is_new: bool) {
        println!("Guild create for guild {:?} (is new? {}", guild, is_new);
        let cmds: Result<Vec<ApplicationCommand>, Error> = guild.get_application_commands(&ctx.http).await;
        let existing_cmds = match cmds {
            Ok(cs) => {
                cs.into_iter().map(|ac| ac.name).collect::<HashSet<String>>()
            }
            Err(e) => {
                println!("Error fetching application commands: {:?}", e);
                Default::default()
            }
        };
        let blah = guild.set_application_commands(&ctx.http, Self::application_commands).await;
        println!("blah: {:?}", blah);
    }

    async fn guild_unavailable(&self, _ctx: Context, guild_id: GuildId) {
        println!("guild unavailable: {}", guild_id);
    }

    async fn guild_update(&self, _ctx: Context, _old_data_if_available: Option<Guild>, new_but_incomplete: PartialGuild) {
        println!("Guild update: {:?}", new_but_incomplete);
    }


    async fn message(&self, _ctx: Context, new_message: Message) {
        println!("Got message: {:?}", new_message);
    }

    async fn ready(&self, _ctx: Context, data_about_bot: Ready) {
        println!("Ready! {:?}", data_about_bot);
        for guild in &data_about_bot.guilds {
            println!("Guild {:?}", guild);
        }
    }

    async fn interaction_create(&self, ctx: Context, interaction: Interaction) {
        println!("Got interaction: {:?}", interaction);
        if let Interaction::ApplicationCommand(i) = interaction {
            println!("interaction name: {}", i.data.name);
            match i.data.name.as_str() {
                "test" => {
                    self.handle_test(&ctx.http, i).await;
                    // i.create_interaction_response(&ctx.http, |r| self.handle_test(r)).await;
                }
                _ => {
                    println!("Unknown interaction :( :(");
                }
            }
        } else {
            println!("Unexpected interaction");
        }
    }
}

#[tokio::main]
async fn main() {
    // https://discord.com/api/oauth2/authorize?client_id=982863079555600414&permissions=122675080256&scope=bot%20applications.commands
    let tok = std::env::var(TOKEN_VAR).expect("DISCORD_TOKEN not found in environment");
    let application_id = std::env::var(APPLICATION_ID_VAR).expect("APPLICATION_ID not found in environment")
        .parse::<u64>().expect("Application ID was not a valid u64");
    let intents = GatewayIntents::GUILDS | GatewayIntents::GUILD_MESSAGES | GatewayIntents::GUILD_MESSAGE_REACTIONS |
        GatewayIntents::DIRECT_MESSAGES | GatewayIntents::DIRECT_MESSAGE_REACTIONS | GatewayIntents::MESSAGE_CONTENT;
    let mut cb: Client = ClientBuilder::new(&tok, intents)
        .application_id(application_id)
        .event_handler(EchoHandler{})

        .await
        .unwrap();
    cb.start().await.unwrap();
    println!("Hello, world!");
}
