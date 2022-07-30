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