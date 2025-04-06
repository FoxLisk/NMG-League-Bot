# Season 9

* Feature: Round Robin brackets now generate all pairings upfront and allow free scheduling of races.
* Feature: There is now [an API](api_docs/README.md)
* Internals: The rtgg rooms that get created are now immediately sent to the bot. This *might* fix an issue where
             the bot would create a room but then not notice it appear and thus not send it out to players, etc.
* Internals: Helper Bot now requires the `helper_bot` cargo feature to run
* Feature: Helper Bot info page at `/helper_bot`
* Internals: User data validation script


# Season 8

* Internals: Discord event management is now asynchronous. This means there will be a slight delay after scheduling
  a race before it appears in the schedule.
* Feature: Season 7 history blurb
* Feature: New "Helper Bot" to allow users to get Discord Events synced to their own servers

# Season 7

* Feature: `/commentators {add,remove}` - admin-only commands to update commentators on races that have
  made it out of #commportunities
* Feature: `/update_user_info` now tolerant to users putting their twitch URL instead of their twitch username
* Feature: Season 6 history blurb
* Enhancement: URLs now use Season ordinals instead of database IDs, making them more obvious to end users
* Enhancement: Player history pages now look nice

# Season 6

* Feature: Player race history now shown on player history page
* Feature: Many aesthetic and design improvements, mostly thanks to RequiemOfSpirit
* Feature: Season 5 history blurb
* Feature: New admin-only command, `/unscheduled_races` to show a digest of unscheduled races
* Feature: Bracket standings now additionally show players' average race times with forfeits removed
* Feature: Player profile command added to Discord context menu
* Feature: Commentator signups will now use best known name for users
* Feature: Support non-2^n-sized Swiss brackets

# Season 5 and before

... lost to the mists of history ...