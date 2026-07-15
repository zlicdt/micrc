# Micrc: micro mc launcher

micrc is a terminal-native Minecraft Java Edition launcher built with Rust and Ratatui.


## Requirements

- Terminal with xterm-256color
- JRE
- Rust

## Build and run

```sh
cargo build --release
./target/release/micrc
```

The default data directory is `$XDG_DATA_HOME/micrc` or `$HOME/.local/share/micrc` on Linux, `%APPDATA%\micrc` on Windows, and the equivalent home-based path on macOS. Set `MICRC_HOME` to use a different directory.

## Controls

| Context | Key | Action |
| --- | --- | --- |
| Global | `Tab` / `Shift+Tab` | Change workspace |
| Global | `q` or `Ctrl+C` | Exit micrc |
| Lists | Arrow keys or `j` / `k` | Change selection |
| Play | Left / Right or `h` / `l` | Change account |
| Play | `Enter` | Launch the selected version |
| Play | `d` or `Delete` | Delete the selected installed version |
| Versions | `Enter` or `i` | Install the selected version |
| Versions | `d` or `Delete` | Delete the selected version when installed |
| Versions | `r` | Refresh the catalog |
| Versions | `s` | Toggle snapshots |
| Accounts | `n` | Add an offline profile |
| Accounts | `e` | Edit the selected offline profile name |
| Accounts | `m` | Start Microsoft login |
| Accounts | `Enter` | Select a profile |
| Accounts | `d` or `Delete` | Remove a profile |
| Microsoft login | `Enter` or `o` | Open the sign-in page again |
| Confirmation | `Enter` or `y` / `Esc` or `n` | Confirm or cancel |
| Settings | `Enter` | Edit a setting |
| Logs | `c` | Clear captured output |
| Input | `Enter` / `Esc` | Submit or cancel |

## Microsoft authentication

Create an application registration that supports personal Microsoft accounts, enable public client flows, and enter its application client ID in the Settings workspace. The ID can also be supplied through `MICRC_MICROSOFT_CLIENT_ID`.

Start Microsoft login from the Accounts workspace. micrc opens the device verification page in the default browser and displays the verification URL and code in the terminal. Complete the Microsoft sign-in in the browser while micrc waits. If the browser cannot be opened, use the displayed URL manually; press `Enter` or `o` to retry opening it. micrc then validates Xbox authorization, Minecraft ownership, and the Java Edition profile.

Access and refresh tokens are stored in `config.json`. On Unix systems, micrc sets the data directory to mode `0700` and the configuration file to mode `0600`.

Detailed account and version workflows are documented in the [user guide](docs/user-guide.md).

## Scope

micrc installs and launches official vanilla versions. Forge, Fabric, NeoForge, Quilt, custom inherited version profiles, and automatic Java runtime installation are outside the current scope.

## Development

```sh
cargo fmt -- --check
cargo clippy --all-targets -- -D warnings
cargo test --all-targets
```

The ignored live metadata test validates the current Mojang release without downloading the full game:

```sh
cargo test --test live_metadata -- --ignored
```

## Some saying
This is a old project on my old hard disk drive, with mess git commit history.

Then use AI to fix it, but I can't provide any support.

If you like, PR is welcome.

## License
MIT
