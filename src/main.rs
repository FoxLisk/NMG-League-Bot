mod utils;

extern crate serenity;
extern crate tokio;

use serenity::builder::CreateApplicationCommands;
use serenity::client::{ClientBuilder, Context, EventHandler};
use serenity::http::Http;
use serenity::model::channel::Message;
use serenity::model::gateway::Ready;
use serenity::model::guild::{Guild, PartialGuild, Role};
use serenity::model::id::{GuildId, RoleId, UserId};
use serenity::model::interactions::application_command::{
    ApplicationCommandInteraction, ApplicationCommandOptionType, ApplicationCommandType,
};
use serenity::model::interactions::{Interaction, InteractionResponseType};
use serenity::model::prelude::application_command::ApplicationCommandInteractionDataOption;
use serenity::model::user::User;
use serenity::model::Permissions;
use serenity::prelude::{GatewayIntents, TypeMapKey};
use serenity::{async_trait, Client};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{RwLock};
use serenity::json::Value;
use serenity::utils::MessageBuilder;
use utils::send_response;
use std::fmt::{Display, Formatter};

const TOKEN_VAR: &str = "DISCORD_TOKEN";
const APPLICATION_ID_VAR: &str = "APPLICATION_ID";

struct AdminRoleMap;

impl TypeMapKey for AdminRoleMap {
    type Value = Arc<RwLock<HashMap<GuildId, RoleId>>>;
}

struct RaceHandler;

enum Error {
    BadInput(String),
    APIError(String),
}

impl Display for Error {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::BadInput(s) => {write!(f, "Bad input: {}", s)}
            Error::APIError(s) => {write!(f, "Internal error: {}", s)}
        }
    }
}

impl RaceHandler {
    const TEST_CMD: &'static str = "test";
    const CREATE_RACE_CMD: &'static str = "create_race";

    fn application_commands(
        commands: &mut CreateApplicationCommands,
    ) -> &mut CreateApplicationCommands {
        commands
            .create_application_command(|command| {
                command
                    .name(Self::TEST_CMD)
                    .description("Test that the bot is alive")
                    .kind(ApplicationCommandType::ChatInput)
                    .default_member_permissions(Permissions::ADMINISTRATOR)
            })
            .create_application_command(|command| {
                command
                    .name(Self::CREATE_RACE_CMD)
                    .description("Create an asynchronous race for two players")
                    .kind(ApplicationCommandType::ChatInput)
                    // N.B. this is imperfect; the Serenity library does not yet support
                    // setting role-based permissions on slash commands, so this is a stand-in
                    .default_member_permissions(Permissions::MANAGE_GUILD)
                    .create_option(|opt| {
                        opt.name("p1")
                            .description("First racer")
                            .kind(ApplicationCommandOptionType::User)
                            .required(true)
                    })
                    .create_option(|opt| {
                        opt.name("p2")
                            .description("Second racer")
                            .kind(ApplicationCommandOptionType::User)
                            .required(true)
                    })
            })
    }

    async fn handle_test(
        &self,
        http: impl AsRef<Http>,
        interaction: ApplicationCommandInteraction,
    ) {
        if let Err(e) = interaction
            .create_interaction_response(http, |ir| {
                ir.kind(InteractionResponseType::ChannelMessageWithSource)
                    .interaction_response_data(|data| data.content("Help, I'm alive!"))
            })
            .await
        {
            println!("Error sending test pong, lol: {}", e);
        }
    }

    fn option_to_user_id(opt: ApplicationCommandInteractionDataOption) -> Result<UserId, Error> {
        if ApplicationCommandOptionType::User != opt.kind {
            return Err(Error::APIError("Bad parameter: Expected user".to_string()));
        }

        if let Value::String(s) = opt.value.ok_or(Error::APIError("Missing user Id".to_string()))? {
            Ok(UserId(
                s.parse::<u64>().map_err(|_e| Error::APIError("Invalid user Id format".to_string()))?
            ))
        } else {
            Err(Error::APIError("Bad parameter: Expected valid user id".to_string()))
        }
    }

    async fn can_create_race(
        &self,
        ctx: &Context,
        guild_id: Option<GuildId>,
        user: User,
    ) -> Result<bool, String> {
        if let Some(gid) = guild_id {
            let needed_role: RoleId = ctx
                .data
                .read()
                .await
                .get::<AdminRoleMap>()
                .unwrap()
                .read()
                .await
                .get(&gid)
                .ok_or(format!("I don't know what role is needed in guild {}", gid))?
                .clone();
            user.has_role(&ctx, gid, needed_role)
                .await
                .map_err(|e| format!("Error checking if user has role: {}", e))
        } else {
            println!("Command called outside of a guild context");
            Ok(false)
        }
    }

    async fn get_racers(ctx: &Context, mut options: HashMap<String, ApplicationCommandInteractionDataOption>) -> Result<(User, User), Error> {
        let p1 = options
            .remove("p1")
            .ok_or(Error::APIError("Missing racer 1 parameter".to_string()))?;
        let p2 = options
            .remove("p2")
            .ok_or(Error::APIError("Missing racer 2 parameter".to_string()))?;


        let racer_1 = Self::option_to_user_id(p1)?;
        let racer_2 = Self::option_to_user_id(p2)?;
        if racer_1 == racer_2 {
            return Err(Error::BadInput("Those are the same player!".to_string()));
        }

        for t in ctx.cache.users() {
            println!("user: {:?}", t);
        }


        let u1 = racer_1.to_user(&ctx).await.map_err(|e| Error::BadInput(format!("Error looking up user {}: {}", racer_1, e)))?;
        let u2 = racer_2.to_user(&ctx).await.map_err(|e| Error::BadInput(format!("Error looking up user {}: {}", racer_1, e)))?;
        //
        // let u1 = ctx.cache.user( racer_1).ok_or(Error::APIError(format!("No known user for id {}", racer_1)))?;
        // let u2 = ctx.cache.user( racer_2).ok_or(Error::APIError(format!("No known user for id {}", racer_2)))?;
        Ok((u1, u2))
    }

    async fn handle_create_race(
        &self,
        ctx: &Context,
        mut interaction: ApplicationCommandInteraction,
    ) -> Result<(), String> {
        let merr = match self
            .can_create_race(
                &ctx,
                std::mem::take(&mut interaction.guild_id),
                std::mem::take(&mut interaction.user),
            )
            .await
        {
            Ok(true) => None,
            Ok(false) => Some("You are not authorized to create races"),
            Err(e) => {
                println!("Error checking if user can create a race: {}", e);
                Some("Internal error")
            }
        };
        if let Some(err) = merr {
            return send_response(&ctx.http, interaction, err).await;
         }

        let options = interaction
            .data
            .options
            .drain(0..)
            .map(|o| (o.name.clone(), o))
            .collect::<HashMap<String, ApplicationCommandInteractionDataOption>>();
        let (r1, r2) = match Self::get_racers(&ctx, options).await {
            Ok(rs) => rs,
            Err(Error::APIError( e)) => {
                println!("Error finding out racer info: {}", e);
                return send_response(&ctx.http, interaction, "Internal error finding racers").await;
            }
            Err(Error::BadInput(e)) => {

                return send_response(&ctx.http, interaction, e).await;
            }
        };

        interaction
            .create_interaction_response(&ctx.http, |ir| {
                ir.kind(InteractionResponseType::ChannelMessageWithSource)
                    .interaction_response_data(|data|
                        data.content(
                            MessageBuilder::new()
                                .push("Race created for users ")
                                .mention(&r1)
                                .push(" and ")
                                .mention(&r2)
                                .build()
                        )
                            .allowed_mentions(|m| m.users(Vec::<UserId>::new()))
                    )

            })
            .await
            .map_err(|e| {
                println!("Error creating response: {}", e);
                e.to_string()
            })
    }
}

async fn maybe_update_admin_role(ctx: &Context, role: Role) {
    if role.name.to_lowercase() == "admin" {
        let data = ctx.data.write().await;
        let role_map = data.get::<AdminRoleMap>().unwrap();
        role_map.write().await.insert(role.guild_id, role.id);
    }
}

#[async_trait]
impl EventHandler for RaceHandler {
    async fn cache_ready(&self, _ctx: Context, _guilds: Vec<GuildId>) {
        println!("Cache ready");
    }

    async fn guild_create(&self, ctx: Context, guild: Guild, is_new: bool) {
        println!("Guild create for guild {:?} (is new? {}", guild, is_new);

        let set_commands_result = guild
            .set_application_commands(&ctx.http, Self::application_commands)
            .await;
        println!("Set commands: {:?}", set_commands_result);
        for role in guild.roles.into_values() {
            maybe_update_admin_role(&ctx, role).await;
        }
    }

    async fn guild_role_create(&self, ctx: Context, new: Role) {
        println!("Guild role created: {:?}", new);
        maybe_update_admin_role(&ctx, new).await;
    }

    async fn guild_unavailable(&self, _ctx: Context, guild_id: GuildId) {
        println!("guild unavailable: {}", guild_id);
    }

    async fn guild_update(
        &self,
        _ctx: Context,
        _old_data_if_available: Option<Guild>,
        new_but_incomplete: PartialGuild,
    ) {
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
                Self::TEST_CMD => {
                    self.handle_test(&ctx.http, i).await;

                }
                Self::CREATE_RACE_CMD => {
                    if let Err(e) = self.handle_create_race(&ctx, i).await {
                        println!("Error creating race: {}", e);
                    }
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
    let tok = dotenv::var(TOKEN_VAR).expect(&*format!("{} not found in environment", TOKEN_VAR));
    let application_id = dotenv::var(APPLICATION_ID_VAR)
        .expect(&*format!("{} not found in environment", APPLICATION_ID_VAR))
        .parse::<u64>()
        .expect("Application ID was not a valid u64");
    println!("tok: {}", tok);
    println!("aid: {}", application_id);
    let intents = GatewayIntents::GUILDS
        | GatewayIntents::GUILD_MESSAGES
        | GatewayIntents::GUILD_MESSAGE_REACTIONS
        | GatewayIntents::DIRECT_MESSAGES
        | GatewayIntents::DIRECT_MESSAGE_REACTIONS
        | GatewayIntents::MESSAGE_CONTENT
        | GatewayIntents::GUILD_MEMBERS
        // adding guild presences *solely* to get guild members populated in the cache to avoid
        // subsequent http requests
        | GatewayIntents::GUILD_PRESENCES;
    let mut cb: Client = ClientBuilder::new(&tok, intents)
        .application_id(application_id)
        .event_handler(RaceHandler {})
        .type_map_insert::<AdminRoleMap>(Arc::new(RwLock::new(HashMap::<GuildId, RoleId>::new())))
        .await
        .unwrap();

    if let Err(e) = cb.start().await {
        println!("Error starting bot: {}", e);
    }
}
