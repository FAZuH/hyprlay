# Token exchange

hyprlay reads voice-channel state over Discord's local RPC connection. The connection needs a Discord OAuth **access token**. This guide explains how to set up your own Discord application for it.

## Set up your application

### 0. Prerequisites

- A Discord account
- Access to <https://discord.com/developers/applications>

### 1. Create a Discord application

1. Create an application in the [Discord Developer Portal](https://discord.com/developers/applications).
2. Go to OAuth2 page of your application.
3. Copy the **Client ID** and the **Client Secret**.
4. **Add redirect** URI `http://127.0.0.1/callback`.

![[assets/token.png]]

### 2. Use your Discord application

There are 3 ways to add your Client ID and Client Secrets:

**GUI:** Run `hyprlay gui`. Open the **Connection** section (`Ctrl+5`). Enter the Client ID and Client Secret you copied. Click **Apply**.

**File:** Write both fields to `~/.config/hyprlay/auth.json`:

```json
{
  "client_id": "...",
  "client_secret": "..."
}
```

**Environment variables:** Set `DISCORD_CLIENT_ID` and `DISCORD_CLIENT_SECRET` before you start the daemon.

## How sign-in works

1. hyprlay asks Discord's local IPC socket for an `AUTHORIZE` code. Discord shows its approval modal. Click **Authorize**.
2. hyprlay exchanges the code for a token at `https://discord.com/api/oauth2/token`. The exchange needs the `client_id` and the `client_secret` of your application.
3. hyprlay uses the token on the IPC socket and caches it at `~/.config/hyprlay/token.json` (mode 0600).
