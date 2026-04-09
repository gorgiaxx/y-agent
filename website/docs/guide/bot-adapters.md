# Bot Adapters

The `y-bot` crate provides platform adapters that expose y-agent as a messaging bot.

## Supported Platforms

| Platform | Transport | Status |
|----------|-----------|--------|
| **Discord** | Interactions Endpoint (Ed25519 signature verification) | Implemented |
| **Feishu (Lark)** | Event webhook | Implemented |
| **Telegram** | Bot API webhook | Implemented |

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
Platform Webhook -> y-web Router -> Bot Handler -> y-service::BotService -> ChatService
```

The bot handler:
1. Verifies the incoming request signature/token
2. Extracts the user message
3. Routes it through `BotService` which manages per-user sessions
4. Returns the agent's response in the platform's expected format

## Discord Setup

1. Create a Discord application at the [Discord Developer Portal](https://discord.com/developers/applications)
2. Configure the Interactions Endpoint URL to point to your y-agent instance: `https://your-domain.com/api/v1/bots/discord/interactions`
3. Set the public key and bot token in `config/bots.toml`
4. Start y-agent with `y-agent serve`

## Feishu (Lark) Setup

1. Create an app in the [Feishu Open Platform](https://open.feishu.cn/)
2. Configure the event webhook URL: `https://your-domain.com/api/v1/bots/feishu/webhook`
3. Set the app credentials in `config/bots.toml`
4. Subscribe to the `im.message.receive_v1` event

## Telegram Setup

1. Create a bot via [@BotFather](https://t.me/BotFather)
2. Set the webhook URL: `https://your-domain.com/api/v1/bots/telegram/webhook`
3. Configure the bot token in `config/bots.toml`
