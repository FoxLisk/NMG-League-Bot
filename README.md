# Running the bot locally

copy `.env-template` to `.env` and input relevant values where the defaults aren't right

Ask me (FoxLisk) for access to the bot tokens. There's a discord developer team that I can add you to. Alternatively
you can create your own discord application and run it through that if you prefer.

Running the bot should be as simple as `cargo run`. If you don't have rust on your system, use
[rustup](https://rustup.rs/). This project builds in stable version 1.60.0 at the moment. I don't particularly want
to be on nightly, but I have no qualms against upgrading the stable version aggressively.

You'll need to run this command to get the css compiled if you want to look at the web pages:

    npx tailwindcss -i ./tailwind_input.css -o ./http/static/css/tailwind.css

which will require npm to be installed, which I forget the steps to bootstrap, sorry, oops.

# other stuff

I have a junk server to spam bot stuff in because testing in the real League server makes it easy to
accidentally send testing messages to ppl who aren't involved in development, which is confusing and annoying.
Please ask me for access.

You probably want sqlite3 installed locally for inspecting the database manually.

`install.sh` is a command that should get this running on a centos server, but honestly you probably don't need that
unless you're running the real bot in "production"

# App structure

This is an asynchronous app built using the [tokio](https://docs.rs/tokio/latest/tokio/) runtime. The main entry
points are:

   1. the discord bot thread (`run_bot` in `src/discord/bot.rs`), which uses [twilight](https://twilight.rs/) to interact with discord. This currently
      just handles async races, which can be created by an admin using an application command (`/create_race`)
   2. the web frontend (`launch_website` in `src/web/mod.rs`), which uses [rocket](https://rocket.rs/). this currently just hosts an admins-only list of
      in-progress async races. It uses [tera](https://tera.netlify.app/docs/) for templating. templates + static
      resources live in `http/`.
   3. `race_cron.rs`, which does periodic scans of the db to report on any races that have been finished

# invite

i think this link has the correct permissions. `client_id` is the bot's client id. if you're joining my bot testing
server this is irrelevant.

https://discord.com/api/oauth2/authorize?client_id=<>&permissions=535061589072&scope=bot%20applications.commands

# expectations for contributing

just have fun and be yourself :)

And _absolutely_ no panics in steady-state running code. Crashing on a missing env variable on startup is totally fine,
but once startup is complete all errors must be handled.

------------

# diesel stuff:

`cargo install diesel_cli --no-default-features --features sqlite-bundled`

can probably do `--features sqlite` on linux

some commands
```
diesel migration generate --migration-dir diesel-migrations <migration_name>

diesel print-schema --database-url .\db\sqlite-diesel.db3
```

# commentary workflow

(via the inimitable clearmouse)

Okay sirius, I have a working prototype of the bot. Here is the proposed workflow:
- when there are 2 (or more) commentators signed up for a race, the bot will notify you in your sirius-inbox
- when you are ready to assign commentators to a race, react to the commportunities message with
:Linkbot: The bot will:
- update events with comms
- post a pingless notification in commentary-discussion for tentative assignments, pending a channel
- post a pingless notification in spam requesting a restream channel (with discord tags as ZSR requested)
- delete the post in commportunities
- when anyone in ZSR's channel is ready to assign a channel,
react with 1️⃣ for ZSR main, 2️⃣  for ZSR2, 3️⃣ for ZSR3, 4️⃣  for ZSR4. The bot will:
- post a pinged notification to commentary-discussion for both racers and all comms, with channel
- update the event with the restream channel
- NOT delete the ZSR post
- if you need to change/update the channel, remove the existing reaction, and then
repost with the correct one for the channel, e.g., it got changed to ZSR2,
so remove the 1️⃣ and react with 2️⃣

    I included a short demo of how the workflow should work. the current example for notifications is just any comm sign up, but the bot is currently set up to do 2+.

https://discord.com/channels/982502396590698498/982508018577051749/1000292512809881661

-- further info (for ZSR folks, again from clearmouse)

oh yeah, because I didn't post anything for ZSR:
- the bot will post something like that above, whenever we (sirius) assigns comms to a race
- once ZSR figures out a channel for the race, anyone in this channel can react to the message 
  to assign that channel:
    - react with 1️⃣  for ZSR main
    - react with 2️⃣  for ZSR2
    - react with 3️⃣  for ZSR3
    - react with 4️⃣  for ZSR4
    - react with :greenham: for FGFM (greenham's channel)
- the bot will then update the event, and then ping out the formal assignment

So as a request, once you guys end up figuring out a channel, can you (or someone) react to it? Would be much appreciated!

As a note, if the channel ends up changing, like ZSR2 -> ZSR3, you can always unreact with the current emote, and then 
react with the new one, e.g., remove 2️⃣  and then react with 3️⃣ 