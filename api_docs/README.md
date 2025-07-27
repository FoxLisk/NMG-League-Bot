There is a little bit of an API. It's read-only and not super fleshed out, including not fleshed out enough for me to write up
proper HTML-ified docs for it. But here's what's available!

# Concepts

Some things that are going to be common to all or many endpoints.

## Root

All endpoints are from the root `https://nmg-league.foxlisk.com/api/v1`. All HTTP requests are `GET`s.

## Return format

All responses are HTTP 200s, with errors given in the payload.

All endpoints return data formatted as JSON. The return value will be a JSON object with either the key `"Ok"` associated with the value you requested, or the key `"Err"` associated with an error message. I am pretty-printing the JSON in this document but the actual API response content will be compact.

Values given as "optional" may have that value type or the value `null` to indicate missing.

Values given as `Enum` are internally defined Enums. The possible values of these Enums will be given.

These Enums are just direct serializations of [Rust Enums](https://doc.rust-lang.org/book/ch06-01-defining-an-enum.html), in case that information is useful to you.

## Paramaters Gotcha

Query parameters given as `Enum` will have to be serialized to JSON in your query string. This means that to filter for races in the format "new", you'd have to pass `?state="New"`. 

## Season Ordinals

Season endpoints use the season's ordinal. This is the order that the season occurred in. There should be minimal reason for anyone to be thinking about season database IDs, but just in case you have one of those, you need to use the ordinal instead.

# Players

URL: `/players`

This returns a list of all players. 

## Parameters

You may specify the query parameter `player_id` zero or more times. If at least one `player_id` is present, the output is filtered to only those players.



| Parameter Name    | Type  | Number          | Description                                      | Example         |
| ----------        | ----  | ------          | -----------                                      | -------         |
| player_id         | i32   | 0 or more       | Filters returned races to ones in this state     | 3               |

## Player Data

The returned data has the following fields:

| Field name        | Type            | Description                                      | Example               |
| ----------        | ----            | -----------                                      | -------               |
| id                | i32             | id                                               | 3                     |
| name              | String          | player's current display name                    | "FoxLisk"             |
| discord_id        | String          | user's Discord ID                                | "255676979460702210"  |
| racetime_username | optional String | user's RTgg full name, if known                  | "FoxLisk#8582"        |
| racetime_user_id  | optional String | user's RTgg username, if known                   | "RbOXG3ydNJBZVKq1"    |
| twitch_user_login | optional String | user's twitch login (not display name), if known | "foxlisk"             |

Note: the racetime username & user ID are either both present or neither. If you find this isn't the case, let me know.

Note: Discord IDs are bigints serialized as strings. [Read their docs here](https://discord.com/developers/docs/reference#snowflakes).

## Example

All players (omitting everyone but myself for space reasons)

```
$ curl https://nmg-league.foxlisk.com/api/v1/players
{
  "Ok": [
    ...,
    {
      "id": 3,
      "name": "FoxLisk",
      "discord_id": "255676979460702210",
      "racetime_username": "FoxLisk#8582",
      "twitch_user_login": "foxlisk",
      "racetime_user_id": "RbOXG3ydNJBZVKq1"
    },
    ...
  ]
}
```

Asking for players `3` and `37`
```
$ curl 'https://nmg-league.foxlisk.com/api/v1/players?player_id=3&player_id=37'
{
  "Ok": [
    {
      "id": 3,
      "name": "FoxLisk",
      "discord_id": "255676979460702210",
      "racetime_username": "FoxLisk#8582",
      "twitch_user_login": "foxlisk",
      "racetime_user_id": "RbOXG3ydNJBZVKq1"
    },
    {
      "id": 37,
      "name": "thisisnotyoho",
      "discord_id": "436176243453591552",
      "racetime_username": "thisisnotyoho#7417",
      "twitch_user_login": "thisisnotyoho",
      "racetime_user_id": "g497NdWRkpBmqXen"
    }
  ]
}
```

# Qualifiers

URL: `/season/<ordinal>/qualifiers`

The qualifiers are sorted by time in ascending order. Note that ALL qualifiers are returned, including obsolete ones.

## Qualifier Data

| Field name        | Type            | Description                                      | Example                                   |
| ----------        | ----            | -----------                                      | -------                                   |
| id                | i32             | id                                               | 287                                       |
| player_id         | i32             | id of the player                                 | 3                                         |
| player_name       | String          | player's current preferred name                  | "FoxLisk"                                 |
| time              | i32             | reported time of the run in seconds              | 5089                                      |
| vod               | String          | provided link to the run                         | "https://www.twitch.tv/videos/2406811169" |

Note: All of the data in qualifiers is user-submitted and has not been vetted in any way.


## Example

```
$ curl https://nmg-league.foxlisk.com/api/v1/season/9/qualifiers
{
  "Ok": [
    ...,
    {
      "id": 287,
      "player_id": 3,
      "player_name": "FoxLisk",
      "time": 5089,
      "vod": "https://www.twitch.tv/videos/2406811169"
    },
    ...,
    {
      "id": 280,
      "player_id": 3,
      "player_name": "FoxLisk",
      "time": 5100,
      "vod": "https://www.twitch.tv/videos/2405262583"
    },
    ...,
    {
      "id": 303,
      "player_id": 37,
      "player_name": "thisisnotyoho",
      "time": 5120,
      "vod": "https://www.twitch.tv/videos/2410161800"
    },
    ...
    {
      "id": 278,
      "player_id": 3,
      "player_name": "FoxLisk",
      "time": 5121,
      "vod": "https://www.twitch.tv/videos/2402664065?t=01h57m40s"
    },
    ...
  ]
}
```

# Brackets

URL: `/season/<ordinal>/brackets`

This API is mostly intended for users of the [Races endpoint](#races) to be able to look up the bracket info

## Bracket Data

| Field name        | Type            | Description                                      | Example         |
| ----------        | ----            | -----------                                      | -------         |
| id                | i32             | id                                               | 27              |
| name              | String          | name                                             | "Gold Sword"    |
| state             | Enum            | current state                                    | "Started"       |
| bracket_type      | Enum            | bracket type (Swiss or Round Robin)              | "Swiss"         |

`state` enum definition:

```
{
    Unstarted,
    Started,
    Finished,
}
```

`bracket_type` enum definition:

```
{
    Swiss,
    RoundRobin,
}
```


## Example:

```
$ curl https://nmg-league.foxlisk.com/api/v1/seasion/9/brackets
{
  "Ok": [
    {
      "id": 26,
      "name": "Fighter Sword",
      "season_id": 9,
      "state": "Started",
      "bracket_type": "Swiss"
    },
    {
      "id": 27,
      "name": "Swordless",
      "season_id": 9,
      "state": "Started",
      "bracket_type": "RoundRobin"
    },
    ...
  ]
}
```

# Races

URL: `/season/<ordinal>/races`

Returns races in the specified season.

## Parameters


| Parameter Name    | Type  | Number          | Description                                      | Example         |
| ----------        | ----  | ------          | -----------                                      | -------         |
| state             | Enum  | 0 or 1          | Filters returned races to ones in this state     | "Scheduled"     |

Remember that [Enum query parameters must be JSON encoded](#paramaters-gotcha)

`state` enum definition: 

```
{
    New,
    Scheduled,
    Finished,
}
```

## Race Data

| Field name        | Type            | Description                                                  | Example             |
| ----------        | ----            | -----------                                                  | -------             |
| id                | i32             | id                                                           | 290                 |
| bracket_id        | i32             | foreign key to [Bracket](#brackets)                          | 23                  |
| round             | i32             | round number*                                                | 1                   |
| player_1_id       | i32             | player 1's id                                                | 25                  |
| player_2_id       | i32             | player 2's id                                                | 1                   |
| state             | Enum            | race state                                                   | "Scheduled"         |
| player_1_result   | optional Enum   | player 1's result, if race is done                           | {"Finish":5025}     |
| player_2_result   | optional Enum   | player 2's result, if race is done                           | {"Finish":4869}     |
| outcome           | optional Enum   | result of the race, if done                                  | "P2Win"             |
| scheduled_for     | optional i64    | UTC timestamp of race time, if scheduled (or complete)       | 1743274860          |
| racetime_gg_url   | optional String | RTgg room URL, if any**                                      | "https://racetime.gg/alttp/witty-robin-9761" |
| restream_channel  | optional String | URL of a restream channel, if any***                         | "https://twitch.tv/zeldaspeedruns" |

\* Currently, all Round Robin bracket races have round "1". This is subject to change at any time.

\*\* This field should get set 30 minutes before the race is created, but could be out sync. Will be `null` for any races run asynchronously.

\*\*\* This will be null by default. The multistream links that populate the UI are not returned in this API.

`result` enum definition (for `player_1_result` and `player_2_result`):

```
{
    Forfeit,
    Finish(u32),
}
```

`outcome` enum definition:

```
{
    Tie,
    P1Win,
    P2Win,
}
```

## Examples

If you do not [JSON encode the value of an enum parameter](#paramaters-gotcha), you will get an opaque error response. Sorry! Maybe someday I'll clean this up.

```
$ curl https://nmg-league.foxlisk.com/api/v1/season/9/races?state=new
{"Err":"Bad Request"}
```

If you want only new (unscheduled) races:

```
$ curl 'https://nmg-league.foxlisk.com/api/v1/season/9/races?state="New"'
{
  "Ok": [
    {
      "id": 291,
      "bracket_id": 23,
      "round": 1,
      "player_1_id": 23,
      "player_2_id": 5,
      "state": "New",
      "player_1_result": null,
      "player_2_result": null,
      "outcome": null,
      "scheduled_for": null,
      "racetime_gg_url": null,
      "restream_channel": null
    },
    {
      "id": 299,
      "bracket_id": 25,
      "round": 1,
      "player_1_id": 38,
      "player_2_id": 43,
      "state": "New",
      "player_1_result": null,
      "player_2_result": null,
      "outcome": null,
      "scheduled_for": null,
      "racetime_gg_url": null,
      "restream_channel": null
    },
   ...
  ]
}
```

And an unfiltered response with some representative values:

```
{
  "Ok": [
    {
      "id": 290,
      "bracket_id": 23,
      "round": 1,
      "player_1_id": 25,
      "player_2_id": 1,
      "state": "Scheduled",
      "player_1_result": null,
      "player_2_result": null,
      "outcome": null,
      "scheduled_for": 1743274860,
      "racetime_gg_url": null,
      "restream_channel": null
    },
    {
      "id": 291,
      "bracket_id": 23,
      "round": 1,
      "player_1_id": 23,
      "player_2_id": 5,
      "state": "New",
      "player_1_result": null,
      "player_2_result": null,
      "outcome": null,
      "scheduled_for": null,
      "racetime_gg_url": null,
      "restream_channel": null
    },
    {
      "id": 292,
      "bracket_id": 23,
      "round": 1,
      "player_1_id": 3,
      "player_2_id": 93,
      "state": "Finished",
      "player_1_result": {
        "Finish": 5025
      },
      "player_2_result": {
        "Finish": 4869
      },
      "outcome": "P2Win",
      "scheduled_for": 1742952600,
      "racetime_gg_url": null,
      "restream_channel": null
    },
    ...
  ]
}
```


# Commentator Signups

URL: `/season/<ordinal>/commentator_signups`

Returns commentator signups in the specified season. These are collected by people using discord reactions,
and at the moment the only data I store about them is the user's discord ID. If you have a use case that would
benefit from a richer API here, please let me know so I can see about prioritizing it.

## Parameters


| Parameter Name    | Type  | Number          | Description                                            | Example |
| ----------        | ----  | ------          | -----------                                            | ------- |
| bracket_race_id   | i32   | 0 or more       | Filters returned signups to ones for the given race(s) | 315     |


## Commentator Signup Data

| Field name        | Type            | Description                                                  | Example              |
| ----------        | ----            | -----------                                                  | -------              |
| bracket_race_id   | i32             | foreign key to [Race](#races)                                | 315                  |
| discord_id        | String          | commentator's Discord ID                                     | "255676979460702210" |

Note: Discord IDs are bigints serialized as strings. [Read their docs here](https://discord.com/developers/docs/reference#snowflakes).



## Example

```
$ curl https://nmg-league.foxlisk.com/api/v1/season/9/commentator_signups?race_id=315
{
  "Ok": [
    {
      "bracket_race_id": 315,
      "discord_id": "270991884430606336"
    },
    {
      "bracket_race_id": 315,
      "discord_id": "306169649006116864"
    }
  ]
}
```
