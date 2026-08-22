# Token exchange

hyprlay reads voice-channel state over Discord's local RPC connection.
The connection needs a Discord OAuth **access token**. This guide
explains how to set up your own Discord application for it.

hyprlay uses only your own application. There is no shared or built-in
application.

## How sign-in works

1. hyprlay asks Discord's local IPC socket for an `AUTHORIZE` code.
   Discord shows its approval modal. Click **Authorize**.
2. hyprlay exchanges the code for a token at
   `https://discord.com/api/oauth2/token`. The exchange needs the
   `client_id` and the `client_secret` of your application.
3. hyprlay uses the token on the IPC socket and caches it at
   `~/.config/hyprlay/token.json` (mode 0600).

Codes, tokens, and client secrets never appear in logs.

## Set up your application

### Prerequisites

- A Discord account
- Access to <https://discord.com/developers/applications>

### Steps

1. Create an application in the
   [Discord Developer Portal](https://discord.com/developers/applications).
   Copy the **Client ID** and the **Client Secret**.

2. Open the **OAuth2** tab of your application. Add one redirect URI:

   ```
   http://127.0.0.1/callback
   ```

   Use exactly this value. Nothing ever opens this address. Discord
   refuses the authorize flow until a desktop redirect exists. If it is
   missing, clicking **Authorize** fails with
   `OAuth2 Error: invalid_request: Missing "redirect_uri" in request`.

Then give both values to hyprlay in one of these ways:

**GUI:** Run `hyprlay gui`. Open the **Connection** section (`Ctrl+5`).
Put the Client ID into the first field. Put the Client Secret into the
masked field. Click **Apply**. hyprlay writes `auth.json` and restarts
the daemon.

**File:** Write both fields to `~/.config/hyprlay/auth.json`:

```json
{
  "client_id": "...",
  "client_secret": "..."
}
```

Set the file mode to 0600.

**Environment variables:** Set `DISCORD_CLIENT_ID` and
`DISCORD_CLIENT_SECRET` before you start the daemon.

A source with only one half of the pair counts as missing. hyprlay
ignores it and logs a warning.

## Apply new credentials

hyprlay reads credentials once, at daemon startup:

1. The environment variables `DISCORD_CLIENT_ID` and
   `DISCORD_CLIENT_SECRET`
2. The file `~/.config/hyprlay/auth.json`

If you change the file or the variables, run `hyprlay restart`. Then run
`hyprlay status`. The reply ends with `auth=own-app`.

Without a complete pair, the daemon logs `credentials_missing` once and
the overlay stays offline. Apply credentials and run `hyprlay restart`
to connect.

## Notes

- The client secret lives only in `auth.json` or the environment. It
  never appears in `config.toml` and never travels over the control
  socket.
- You do not need to register RPC origins. The local IPC transport has
  no HTTP layer, so there is no origin check.
- A cached token from a different application fails with
  `4009 INVALID_TOKEN`. hyprlay drops that token and runs the authorize
  flow again.
