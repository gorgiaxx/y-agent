# Bot Adapters

The `y-bot` crate provides platform adapters that expose y-agent as a messaging bot.

## Supported Platforms

| Platform | Transport | Status |
|----------|-----------|--------|
| **Discord** | Gateway (WebSocket) + REST API + webhook | Implemented |
| **Feishu (Lark)** | Event webhook | Implemented |
| **Telegram** | Bot API webhook | Interface defined |

Bot adapters are wired into `y-web` and share the same `ServiceContainer`. Configure them in `config/bots.toml`.

## Configuration

```toml
# config/bots.toml

[discord]
enabled = true
public_key = "your-discord-public-key"
application_id = "your-discord-app-id"
bot_token_env = "DISCORD_BOT_TOKEN"

[feishu]
enabled = true
app_id = "your-feishu-app-id"
app_secret_env = "FEISHU_APP_SECRET"
verification_token_env = "FEISHU_VERIFICATION_TOKEN"
encrypt_key_env = "FEISHU_ENCRYPT_KEY"

[telegram]
enabled = true
bot_token_env = "TELEGRAM_BOT_TOKEN"
webhook_url = "https://your-domain.com/api/v1/bots/telegram/webhook"
```

## Architecture

All bot adapters follow the same pattern:

```
Discord Gateway (WS)  ->  MESSAGE_CREATE  ->  InboundMessage channel
                                                    |
Platform Webhook  ->  y-web Router  ->  BotPlatform::parse_event()
                                                    |
                                            BotService (y-service)
                                                    |
                                            BotPlatform::send_message()
```

The bot handler:
1. Verifies the incoming request signature/token
2. Extracts the user message
3. Routes it through `BotService` which manages per-user sessions
4. Returns the agent's response in the platform's expected format

## Discord Setup

Discord integration supports two modes: **Gateway** (persistent WebSocket) and **Webhook** (HTTP endpoint).

### Gateway Mode (Recommended)

The Gateway client maintains a persistent WebSocket connection to `wss://gateway.discord.gg` (v10), handling heartbeats, reconnection, and `MESSAGE_CREATE` events automatically.

1. Create a Discord application at the [Discord Developer Portal](https://discord.com/developers/applications)
2. Enable the **Message Content** intent
3. Set the bot token in `config/bots.toml`
4. Start y-agent with `y-agent serve` -- the Gateway client connects automatically

### Webhook Mode

1. Configure the Interactions Endpoint URL to point to your y-agent instance: `https://your-domain.com/api/v1/bots/discord/webhook`
2. Set the public key and bot token in `config/bots.toml`

## Feishu (Lark) Setup

1. Create an app in the [Feishu Open Platform](https://open.feishu.cn/)
2. Configure the event webhook URL: `https://your-domain.com/api/v1/bots/feishu/webhook`
3. Set the app credentials in `config/bots.toml`
4. Subscribe to the `im.message.receive_v1` event

## Telegram Setup

1. Create a bot via [@BotFather](https://t.me/BotFather)
2. Set the webhook URL: `https://your-domain.com/api/v1/bots/telegram/webhook`
3. Configure the bot token in `config/bots.toml`
