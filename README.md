# Running the bot locally

copy `.env-template` to `.env` and input relevant values where the defaults aren't right.

Ask me _(FoxLisk)_ for access to the bot tokens. There's a discord developer team that I can add you to. 
Alternatively, you can create your own discord application and run it through that if you prefer.

## Dependencies

* If you don't have rust on your system, use [rustup](https://rustup.rs/). This project builds in stable version 1.67.0 at the moment. I don't particularly want to be on nightly, but I have no qualms against upgrading the stable version aggressively.
* You'll need node and npm. You can install them from the [official node site](https://nodejs.org/en/download).
* You might run into an issue about needing openssl dev packages. The commands on [this post](https://ma.ttias.be/could-not-find-directory-of-openssl-installation/) have solved it for at least one person. 

## Running
Running the bot should be as simple as running `cargo run`.

To view the web pages, run `npm install` to install dependencies and then `npm run build`
to compile the `sass` stylesheets and `typescript` files.
To actively develop on the frontend, run `npm run build:watch` to have stylesheets and ts files
recompile automatically on change.

The site also has a couple admin only features and pages like the asyncs page and extra columns on the quals page.
To see these, start the server with `cargo run --features no_auth_website`.

There's also scripts in `src/bin` which you can run with `cargo run --bin <script name>`. 
These aren't, like, maximally well maintained and useful, but `generate_test_data` is a decent place to start 
populating the databases. Some of these scripts are for one-off migrations, some are for testing. 
Sorry the place is a mess.

### RaceTime

The racetime bot is disabled by default because it's not really appropriate to run in dev unless you also have a
local instance of the racetime app running. To run it, use the `racetime_bot` feature (and you will have to fill in
some extra env vars).

It's not hard to get the app running: just [clone their repo](https://github.com/racetimeGG/racetime-app) and follow the steps they provide:

# Other stuff

I have a junk server to spam bot stuff in because testing in the real League server makes it easy to accidentally
send testing messages to ppl who aren't involved in development, which is confusing and annoying.
Please ask me for access.

You probably want sqlite3 installed locally for inspecting the database manually.

`install.sh` is a command that should get this running on a centos server, but honestly you probably don't need that unless you're running the real bot in "production"

# App structure

This is an asynchronous app built using the [tokio](https://docs.rs/tokio/latest/tokio/) runtime. The main entry points are:
  1. The discord bot thread (`run_bot` in `src/discord/bot.rs`), which uses [twilight](https://twilight.rs/) to interact with discord.
  2. The web frontend (`launch_website` in `src/web/mod.rs`), which uses [rocket](https://rocket.rs/). 
     Templates, stylesheets, scripts and other static resources live in `http/`.
  3. The racetime.gg bot (`run_bot` in `src/racetime_bot/mod.rs`), which uses [racetime-bot](https://github.com/racetimeGG/racetime-bot).
     This creates race rooms, invites people, handles race monitor promotion, etc. Only enabled on the `racetime_bot` feature.
  3. The workers in the `workers` module.

If this is out of date, you can see exactly what gets launched in `main.rs`, which should do nothing but start up tasks.

# Invite

i think this link has the correct permissions. `client_id` is the bot's client id. 
If you're joining my bot testing server this is irrelevant.

https://discord.com/api/oauth2/authorize?client_id=<>&permissions=1644905888848&scope=bot%20applications.commands

# Expectations for contributing

just have fun and be yourself :)

And _absolutely_ no panics in steady-state running code. Crashing on a missing env variable on startup is totally fine, 
but once startup is complete all errors must be handled.

------------

# Diesel stuff:

`cargo install diesel_cli --no-default-features --features sqlite-bundled`

can probably do `--features sqlite` on linux

some commands
```
diesel migration generate --migration-dir diesel-migrations <migration_name>

diesel print-schema --database-url .\db\sqlite-diesel.db3
```