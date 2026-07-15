# micrc user guide

This guide covers account and installed-version management. The complete keyboard reference is in the project [README](../README.md#controls).

## Microsoft account browser login

Microsoft authentication requires an Entra application client ID. The application registration must support personal Microsoft accounts and public client flows. Set the client ID in the Settings workspace, or start micrc with `MICRC_MICROSOFT_CLIENT_ID` set.

To sign in:

1. Open the Accounts workspace and press `m`.
2. micrc requests a device code and opens the Microsoft verification page in the system browser.
3. Enter the code shown by micrc if the browser does not fill it automatically, then finish the Microsoft sign-in.
4. Leave micrc running while it checks Xbox authorization, Minecraft: Java Edition ownership, and the Minecraft profile.
5. After validation, the profile is stored and selected automatically.

The terminal dialog always keeps the verification URL and code visible. If the browser does not open, visit that URL manually. Press `Enter` or `o` to ask micrc to open it again. Press `Esc` to cancel the login.

micrc uses `xdg-open` on Unix-like desktop systems, `open` on macOS, and the Windows URL handler on Windows. A minimal or remote environment may not have a graphical browser handler, so the manual URL remains available.

Access and refresh tokens are stored in `config.json`. On Unix systems, the micrc data directory uses mode `0700` and the configuration file uses mode `0600`. Remove an account from the Accounts workspace with `d` or `Delete` to remove its stored tokens from the configuration.

## Offline account names

Open the Accounts workspace and press `n` to create an offline profile. Names must contain 3 to 16 ASCII letters, numbers, or underscores and must be unique in the profile list.

To change a name, highlight an offline profile, press `e`, edit the value, and press `Enter`. Press `Esc` to discard the edit. Minecraft offline UUIDs are derived from the profile name, so micrc recalculates the UUID when the name changes and updates the selected profile reference. Microsoft profile names cannot be edited locally because they come from the authenticated Minecraft profile.

Offline mode does not authenticate a Minecraft license or provide access to online-mode servers.

## Deleting an installed version

Delete a version from either of these workspaces:

- In Play, highlight an installed version and press `d` or `Delete`.
- In Versions, highlight a row marked `INSTALLED` and press `d` or `Delete`.

micrc asks for confirmation before deleting files. Press `Enter` or `y` to confirm, or `Esc` or `n` to cancel. Deletion is unavailable while Minecraft or another launcher task is running.

Deletion removes only that version's directory under `minecraft/versions`. Shared libraries and assets remain because other installed versions may use them. If the deleted version was selected for launch, micrc selects another installed version when available; otherwise, the launch selection becomes empty.

Deleting an installed version does not remove accounts, settings, game saves, shared resources, or captured launcher logs. The version can be installed again from the Versions workspace.
