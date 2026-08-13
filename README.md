# mpd-herald

`mpd-herald` is a background companion for MPD. It listens for player and queue
changes to provide desktop notifications, Discord Rich Presence, and Last.fm
now playing status and scrobbles.

<p align="center">
  <img
    width="552"
    src="https://github.com/user-attachments/assets/90804966-568d-43d9-8f7b-f06555b9ab13"
    alt="Desktop notification showing the currently playing track"
  >
  <br>
  <sub>Desktop Notification</sub>
</p>

<p align="center">
  <img
    width="552"
    src="https://github.com/user-attachments/assets/947f0998-dc8a-4ca8-94fd-71a9b23fed14"
    alt="Discord Rich Presence showing the currently playing track"
  >
  <br>
  <sub>Discord Rich Presence</sub>
</p>

## Install

### Cargo

Install from git with Cargo:

```sh
cargo install --git https://github.com/sarowish/mpd-herald --locked
```

### Nix Flake

You can install the latest development version from GitHub using the flake:

```nix
{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    mpd-herald.url = "github:sarowish/mpd-herald";
  };

  outputs = { nixpkgs, mpd-herald, ... }: {
    nixosConfigurations.nixos = nixpkgs.lib.nixosSystem {
      system = "x86_64-linux";
      modules = [
        ({ pkgs, ... }: {
          environment.systemPackages = [
            mpd-herald.packages.${pkgs.system}.default
          ];
        })
      ];
    };
  };
}
```

## Usage

```text
Usage: mpd-herald [COMMAND]

Commands:
  authenticate  Create last.fm user session
  help          Print this message or the help of the given subcommand(s)

Options:
  -h, --help     Print help
  -V, --version  Print version
```

The default configuration connects to `localhost:6600`,
enables notifications and Discord Rich Presence.

## Configuration

On Linux, this is `$XDG_CONFIG_HOME/mpd-herald/config.toml`, or
`$HOME/.config/mpd-herald/config.toml` when `XDG_CONFIG_HOME` is not set.

For example:

```toml
host = "localhost"
port = 6600

[notification]
enable = true
timeout = 6000

[notification.playing_text]
summary = "Playing %title%"
body = "%albumartist% - %album%"

[notification.paused_text]
summary = "Paused %title%"
body = "%albumartist% - %album%"

[notification.stopped_text]
summary = "Stopped"
body = ""

[discord_rpc]
enable = true
client_id = 1465967948861669469
state = "%albumartist%"
details = "%title%"
large_text = "%album%"
small_text = ""
large_image = ""
small_image = ""
display_type = "state"
buttons = []

[scrobbling.lastfm]
enable = false
api_key = "your-api-key"
secret = "your-shared-secret"
prefer_album_artist = false
```

At least one integration must be enabled. If notifications, Discord Rich
Presence, and Last.fm scrobbling are all disabled, the app exits.

The root of `config.toml` has:

| Option | Description | Default       |
| ------ | ----------- | ------------- |
| `host` | MPD host.   | `"localhost"` |
| `port` | MPD port.   | `6600`        |

The `[notification]` section has:

| Option                 | Description                      | Default                     |
| ---------------------- | -------------------------------- | --------------------------- |
| `enable`               | Enable this integration.         | `true`                      |
| `timeout`              | Display timeout in milliseconds. | `6000`                      |
| `playing_text.summary` | Summary shown while playing.     | `play icon + "%title%"`     |
| `playing_text.body`    | Body shown while playing.        | `"%albumartist% - %album%"` |
| `paused_text.summary`  | Summary shown while paused.      | `pause icon + "%title%"`    |
| `paused_text.body`     | Body shown while paused.         | `"%albumartist% - %album%"` |
| `stopped_text.summary` | Summary shown when stopped.      | `"Stopped"`                 |
| `stopped_text.body`    | Body shown when stopped.         | `""`                        |

Notification album art is loaded from MPD using the current song URI. Processed
images are cached in the platform cache directory.

The `[discord_rpc]` section has:

| Option          | Description                                                          | Default               |
| --------------- | -------------------------------------------------------------------- | --------------------- |
| `enable`        | Enable this integration.                                             | `true`                |
| `client_id`     | Discord application client ID.                                       | `1465967948861669469` |
| `state`         | Format string for the second line.                                   | `"%albumartist%"`     |
| `details`       | Format string for the top line.                                      | `"%title%"`           |
| `large_text`    | Format string for the bottom line and large image hover tooltip.     | `"%album%"`           |
| `small_text`    | Format string for the small image hover tooltip.                     | `""`                  |
| `large_image`   | Fallback asset key or external URL for the large image.              | `""`                  |
| `small_image`   | Asset key or external URL for the small image.                       | `""`                  |
| `display_type`  | Field used for Discord's status text: `name`, `state`, or `details`. | `"state"`             |
| `buttons`       | Up to two activity buttons, each with a `label` and `url`.           | `[]`                  |

Discord album art is fetched from Cover Art Archive when the song has a
MusicBrainz release ID or release group ID tag. When both tags are absent,
MusicBrainz is searched using the `AlbumArtist` (or `Artist`) and `Album`
tags for a release group. If no image is found, `large_image` is used when
configured.

To add activity buttons, set `buttons` to an array containing one or two button
definitions:

```toml
[discord_rpc]
buttons = [
    { label = "Listen to %title%", url = "https://example.com/tracks/%title%" },
    { label = "%artist%", url = "https://example.com/artists/%artist%" },
]
```

The `[scrobbling.lastfm]` section has:

| Option                  | Description                                            | Default |
| ----------------------- | ------------------------------------------------------ | ------- |
| `enable`                | Enable Last.fm scrobbling.                             | —       |
| `api_key`               | Last.fm API key.                                       | —       |
| `secret`                | Last.fm shared secret.                                 | —       |
| `prefer_album_artist`   | Prefer `AlbumArtist` over `Artist` for scrobbling.     | `false` |

### Tokens

Text outside tokens is displayed as written; known tokens are replaced with the
matching song fields:

| Token            | Value                 |
| ---------------- | --------------------- |
| `%name%`         | `Name` tag            |
| `%artist%`       | `Artist` tag          |
| `%album%`        | `Album` tag           |
| `%albumartist%`  | `AlbumArtist` tag     |
| `%composer%`     | `Composer` tag        |
| `%date%`         | `Date` tag            |
| `%originaldate%` | `OriginalDate` tag    |
| `%disc%`         | `Disc` tag            |
| `%genre%`        | `Genre` tag           |
| `%performer%`    | `Performer` tag       |
| `%title%`        | `Title` tag           |
| `%track%`        | `Track` tag           |
| `%time%`         | Track duration        |
| `%elapsed%`      | Elapsed playback time |
| `%file%`         | MPD song URI          |

Unknown tokens are rendered as their name without `%`.

## Last.fm

Add Last.fm credentials to `config.toml`:

```toml
[scrobbling.lastfm]
enable = true
api_key = "your-api-key"
secret = "your-shared-secret"
prefer_album_artist = false
```

Then authenticate:

```sh
mpd-herald authenticate
```

The command opens Last.fm in your browser. After granting access, press enter in
the terminal to save the session.

A track with a known duration of at least 30 seconds is scrobbled after half its
duration has played, or after four minutes. A track with
an unknown duration is scrobbled after four minutes.
