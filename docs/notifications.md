# Notifications

Vesper sends incident notifications to any number of channels. The channel kinds are ported from [Uptime Kuma's notification providers](https://github.com/louislam/uptime-kuma/tree/master/server/notification-providers): chat apps, push services, email, SMS and WhatsApp gateways, and on-call tools.

```yaml
notifications:
  dashboard_url: https://status.example.com   # links in messages point here
  reminder_seconds: 3600                      # optional: repeat unacknowledged incidents
  channels:
    telegram:
      kind: telegram
      bot_token: {file: telegram-bot-token}
      chat_id: {file: telegram-chat-id}
    phone:
      kind: ntfy
      topic: {file: ntfy-topic}
    oncall:
      kind: pagerduty
      integration_key: {env: PAGERDUTY_KEY}
```

- **Every channel gets every notification.** Use a service's `notifications: false` to keep it quiet everywhere. Channel IDs follow the same rules as machine and service IDs.
- **Credentials must be secret references.** Settings marked *secret* below only accept `{file: name}` (read from `/run/secrets/name`) or `{env: NAME}`. Any other setting takes a literal or a secret reference, so a private ntfy topic or a phone number can stay out of YAML too.
- **List settings** take a YAML list or one comma-separated text, for example `to: [ops@example.com, me@example.com]`.
- **Messages are summaries.** A message names every new problem, recovery and reminder pending for that channel, with times, durations and a dashboard link, and never probe evidence or credentials. A channel that was rate-limited or offline sends one current summary when it can, not a burst of old events.
- **On-call kinds send one incident per message.** Kinds marked *per incident* (PagerDuty, Opsgenie, Splunk On-Call and the like) create one alert per Vesper incident, keyed `vesper/incident/<id>`, and resolve it on recovery.
- **Delivery state is per destination.** Each channel tracks what it has delivered, its rate limits and its backoff. If you change a channel's destination (its chat, topic, recipients or webhook), open incidents are sent to the new one; rotating a token does not resend anything.
- **Rejections stop a channel.** If a service rejects a message as unauthorized or misconfigured, that channel stops until the configuration changes or a test message from **Settings → Notifications** succeeds. Server and network failures retry with backoff.

## Network policy

Notification requests go to public addresses freely. A self-hosted service on a private network (RFC 1918, CGNAT or IPv6 ULA addresses), such as Gotify, ntfy, Matrix or Home Assistant on your LAN, must be inside `network.allowed_cidrs`, the same list that gates checks. Loopback, link-local and cloud metadata addresses are never reachable. Vesper connects only to the addresses it checked, uses no proxy, follows no redirects and always verifies TLS against public roots plus `network.private_ca_file`.

## Not ported

- **Web Push** needs browser subscriptions managed by the dashboard, which Vesper does not have.
- **Nostr** needs relay WebSockets and NIP-17 encrypted direct messages.
- **Apprise** runs the `apprise` command in Uptime Kuma. Vesper's `apprise` kind talks to an [Apprise API](https://github.com/caronc/apprise-api) server instead.
- Uptime Kuma's custom message templates, per-notification proxies and "ignore TLS errors" options are left out of every kind.

## Upgrading from the `telegram` block

The older top-level `telegram:` block still works and acts as one channel with ID `telegram`. To add more channels, move it into `notifications` as shown above: `dashboard_url` and `reminder_seconds` move up to `notifications`, and the bot token and chat ID become settings of a `kind: telegram` channel named `telegram`. Delivery state carries over. The two blocks cannot be used together.

## Channel kinds

The list below is generated from the catalog in `crates/notify` (`VESPER_UPDATE_DOCS=1 cargo test -p vesper-notify` rewrites it). Settings without *required* are optional.

<!-- catalog:start -->

<a id="360messenger"></a>
### 360messenger (`360messenger`)

[API documentation](https://developer.360messenger.com/)

| Setting | Value | Description |
| --- | --- | --- |
| `auth_token` | text; *secret*, *required* | API key from the 360messenger dashboard. |
| `recipient_type` | `phone`, `group`; default `phone` | Whether recipients are WhatsApp phone numbers or group IDs. |
| `recipients` | list; *required* | Phone numbers with country code and no `+`, or WhatsApp group IDs. |

<a id="46elks"></a>
### 46elks (`46elks`)

[API documentation](https://46elks.com/docs/send-sms)

| Setting | Value | Description |
| --- | --- | --- |
| `username` | text; *required* | API username from https://46elks.com/account. |
| `password` | text; *secret*, *required* | API password from https://46elks.com/account. |
| `from_number` | text; *required* | Text sender ID of up to 11 characters, or an E.164 number to be able to receive replies. |
| `to_number` | text; *required* | Recipient phone number in E.164 format. |

<a id="alerta"></a>
### Alerta (`alerta`)

[API documentation](https://docs.alerta.io/api/reference.html#create-an-alert) · per incident

| Setting | Value | Description |
| --- | --- | --- |
| `api_endpoint` | URL; *required* | Alert endpoint of the Alerta API. |
| `environment` | text; *required* | Alerta environment of the alerts. |
| `api_key` | text; *secret* | API key, unless the server runs without authentication. |
| `alert_state` | text; default `critical` | Severity of problems and tests. |
| `recover_state` | text; default `cleared` | Severity of recoveries; a normal severity closes the alert. |

<a id="alertnow"></a>
### AlertNow (`alertnow`)

[API documentation](https://service.opsnow.com/docs/alertnow/en/user-guide-alertnow-en.html#standard) · per incident

| Setting | Value | Description |
| --- | --- | --- |
| `webhook_url` | URL; *secret*, *required* | Webhook URL of an AlertNow Standard integration. |

<a id="aliyun-sms"></a>
### Alibaba Cloud SMS (Aliyun) (`aliyun_sms`)

[API documentation](https://help.aliyun.com/zh/sms/developer-reference/api-dysmsapi-2017-05-25-sendsms)

| Setting | Value | Description |
| --- | --- | --- |
| `access_key_id` | text; *secret*, *required* | AccessKey ID of a RAM user allowed to send SMS. |
| `access_key_secret` | text; *secret*, *required* | AccessKey secret of that RAM user. |
| `phone_numbers` | list; *required* | Recipient mobile numbers. |
| `template_code` | text; *required* | Approved template code; the template may use `${name}`, `${time}` and `${status}`. |
| `sign_name` | text; *required* | Approved signature name. |
| `optional_parameters` | true/false | Also fill `${msg}` with the summary (addresses removed); templates with free text are often rejected. |

<a id="amootsms"></a>
### Amoot SMS (`amootsms`)

[API documentation](https://portal.amootsms.com/documentation)

| Setting | Value | Description |
| --- | --- | --- |
| `api_token` | text; *secret*, *required* | API token from the Amoot SMS portal. |
| `mobiles` | list; *required* | Recipient mobile numbers. |
| `line_number` | text; default `public` | Sender line number, or `public` for the shared line. |
| `use_pattern` | true/false | Send through an approved pattern instead of free text. |
| `pattern_code_id` | integer | Pattern code ID from the portal, required with use_pattern. |
| `use_own_line` | true/false | Send the pattern from line_number instead of the shared pattern line. |

<a id="apprise"></a>
### Apprise API (`apprise`)

[API documentation](https://github.com/caronc/apprise-api#api-details)

| Setting | Value | Description |
| --- | --- | --- |
| `server_url` | URL; *required* | Apprise API server. |
| `urls` | text; *secret* | Apprise URLs to notify, separated by commas or spaces; without them and without config_key the server's APPRISE_STATELESS_URLS are used. |
| `config_key` | text | Key of a configuration saved on the server, used instead of urls. |
| `tag` | text | Notify only the saved configuration's URLs with this tag (with config_key). |
| `username` | text | Basic Auth user name, when the server requires a login. |
| `password` | text; *secret* | Basic Auth password for the username. |

<a id="bale"></a>
### Bale (`bale`)

[API documentation](https://docs.bale.ai/)

| Setting | Value | Description |
| --- | --- | --- |
| `bot_token` | text; *secret*, *required* | Bot token from @BotFather on Bale. |
| `chat_id` | text; *required* | Chat, group or channel ID; message the bot first so it may write to you. |

<a id="bark"></a>
### Bark (`bark`)

[API documentation](https://bark.day.app/en-us/tutorial)

| Setting | Value | Description |
| --- | --- | --- |
| `endpoint` | URL; *secret*, *required* | Push URL with the device key, as shown in the Bark app, on api.day.app or your own bark-server. |
| `api_version` | `v1`, `v2`; default `v1` | `v1` sends a GET with query parameters; `v2` posts JSON (bark-server 2 and later). |
| `group` | text; default `Vesper` | Group the notifications are filed under on the device. |
| `sound` | `alarm`, `anticipate`, `bell`, `birdsong`, `bloom`, `calypso`, `chime`, `choo`, `descent`, `electronic`, `fanfare`, `glass`, `gotosleep`, `healthnotification`, `horn`, `ladder`, `mailsent`, `minuet`, `multiwayinvitation`, `newmail`, `newsflash`, `noir`, `paymentsuccess`, `shake`, `sherwoodforest`, `silence`, `spell`, `suspense`, `telegraph`, `tiptoes`, `typewriters`, `update`; default `telegraph` | Notification sound. |

<a id="bearsms"></a>
### BearSMS (`bearsms`)

[API documentation](https://www.bearsms.com/1-q-a/)

| Setting | Value | Description |
| --- | --- | --- |
| `username` | text; *required* | BearSMS API username. |
| `hash_key` | text; *secret*, *required* | Webservices token (hash key) from the BearSMS preferences. |
| `phone_number` | text; *required* | Recipient number in international format without a plus sign. |
| `sender_id` | text | Sender name or number of up to 11 characters. |

<a id="bitrix24"></a>
### Bitrix24 (`bitrix24`)

[API documentation](https://apidocs.bitrix24.com/api-reference/chats/notifications/im-notify-system-add.html)

| Setting | Value | Description |
| --- | --- | --- |
| `webhook_url` | URL; *secret*, *required* | Inbound webhook URL with the `im` scope, from Developer resources → Other → Inbound webhook. |
| `user_id` | text; *required* | ID of the user who receives the system notification. |

<a id="brevo"></a>
### Brevo (`brevo`)

[API documentation](https://developers.brevo.com/reference/sendtransacemail)

| Setting | Value | Description |
| --- | --- | --- |
| `api_key` | text; *secret*, *required* | API key from SMTP & API settings. |
| `from_email` | text; *required* | Verified sender address. |
| `from_name` | text; default `Vesper` | Sender name. |
| `to` | list; *required* | Recipients. |
| `cc` | list | Copy recipients. |
| `bcc` | list | Blind copy recipients. |
| `subject_prefix` | text | Text before the subject, such as `[homelab]`. |

<a id="call-me-bot"></a>
### CallMeBot (`call_me_bot`)

[API documentation](https://www.callmebot.com/blog/free-api-whatsapp-messages/)

| Setting | Value | Description |
| --- | --- | --- |
| `endpoint` | URL; *secret*, *required* | API URL from CallMeBot's activation message (WhatsApp, Signal, Facebook Messenger or Telegram), with your phone and API key; Vesper sets its `text` parameter. |

<a id="cellsynt"></a>
### Cellsynt (`cellsynt`)

[API documentation](https://www.cellsynt.com/en/sms/api-integration)

| Setting | Value | Description |
| --- | --- | --- |
| `username` | text; *required* | Cellsynt account username. |
| `password` | text; *secret*, *required* | Cellsynt account password. |
| `destination` | text; *required* | Recipient number in international format with a 00 prefix. |
| `originator_type` | `alpha`, `numeric`; default `alpha` | `alpha` sends from a name (recipients cannot reply); `numeric` from a phone number. |
| `originator` | text; default `Vesper` | Sender shown to the recipient: up to 11 letters and digits, or up to 15 digits when numeric. |
| `allow_long_sms` | true/false | Allow up to 6 concatenated messages instead of cutting the text to one 160-character SMS. |

<a id="clicksend-sms"></a>
### ClickSend SMS (`clicksend_sms`)

[API documentation](https://developers.clicksend.com/docs/messaging/sms/other/send-sms)

| Setting | Value | Description |
| --- | --- | --- |
| `username` | text; *required* | API username of the ClickSend account or subaccount. |
| `api_key` | text; *secret*, *required* | API key of the account or subaccount. |
| `to_number` | text; *required* | Recipient phone number in E.164 format. |
| `sender_name` | text | Sender ID of up to 11 characters; leave empty to use a shared number. |

<a id="clickup"></a>
### ClickUp (`clickup`)

[API documentation](https://developer.clickup.com/reference/createchatmessage)

| Setting | Value | Description |
| --- | --- | --- |
| `token` | text; *secret*, *required* | Personal API token (`pk_…`). |
| `workspace_id` | text; *required* | Workspace ID, the number after app.clickup.com/ in its URL. |
| `channel_id` | text; *required* | Chat channel ID, from the channel's URL. |
| `disable_url` | true/false | Leave the dashboard link out of messages. |

<a id="dingding"></a>
### DingTalk (`dingding`)

[API documentation](https://open.dingtalk.com/document/robots/custom-robot-access)

| Setting | Value | Description |
| --- | --- | --- |
| `webhook_url` | URL; *secret*, *required* | Webhook URL of the group's custom robot, with its access_token. |
| `secret_key` | text; *secret*, *required* | Signing secret (starting with SEC) from the robot's security settings. |
| `mentioning` | `nobody`, `everyone`, `specify-mobiles`, `specify-users`; default `nobody` | Whom the message mentions. |
| `mobile_list` | list | Mobile numbers to mention with `specify-mobiles`. |
| `user_list` | list | User IDs to mention with `specify-users`. |

<a id="discord"></a>
### Discord (`discord`)

[API documentation](https://discord.com/developers/docs/resources/webhook#execute-webhook)

| Setting | Value | Description |
| --- | --- | --- |
| `webhook_url` | URL; *secret*, *required* | Webhook URL from Channel settings → Integrations → Webhooks. |
| `username` | text; default `Vesper` | Display name of the webhook. |
| `avatar_url` | URL | Avatar image URL. |
| `thread_id` | text | Post into this existing thread or forum post. |
| `thread_name` | text | Create a forum post with this name (forum channels only). |
| `suppress_notifications` | true/false | Post without push and desktop notifications. |

<a id="egosms"></a>
### EgoSMS (`egosms`)

[API documentation](https://developers.pahappa.com/)

| Setting | Value | Description |
| --- | --- | --- |
| `username` | text; *required* | EgoSMS account username. |
| `password` | text; *secret*, *required* | EgoSMS account password. |
| `phone_number` | text; *required* | Recipient number in international format without a plus sign. |
| `sender` | text; default `EGOSMS` | Sender name or number of up to 11 characters. |

<a id="smtp"></a>
### Email (SMTP) (`smtp`)

[API documentation](https://github.com/cfbender/vesper/blob/main/docs/notifications.md#smtp)

| Setting | Value | Description |
| --- | --- | --- |
| `host` | text; *required* | SMTP server host name or address. |
| `port` | integer; default `587` | SMTP port. |
| `security` | `starttls`, `tls`, `none`; default `starttls` | `starttls` upgrades with STARTTLS (port 587), `tls` is implicit TLS (port 465), `none` is plain SMTP for a trusted relay. |
| `username` | text | Login user name. |
| `password` | text; *secret* | Login password. |
| `from` | text; *required* | Sender, such as `Vesper <vesper@example.com>`. |
| `to` | list; *required* | Recipients. |
| `cc` | list | Copy recipients. |
| `bcc` | list | Blind copy recipients. |
| `subject_prefix` | text | Text before the subject, such as `[homelab]`. |

<a id="evolution"></a>
### Evolution API (`evolution`)

[API documentation](https://docs.evoapicloud.com/api-reference/message-controller/send-text)

| Setting | Value | Description |
| --- | --- | --- |
| `server_url` | URL; *required* | Address of the Evolution API server. |
| `instance_name` | text; *required* | Name of the connected WhatsApp instance. |
| `auth_token` | text; *secret*, *required* | API key of the instance (`apikey` header). |
| `recipient` | text; *required* | Phone number with country code and no `+`, or a group JID such as `120363000000000000@g.us`. |

<a id="feishu"></a>
### Feishu (Lark) (`feishu`)

[API documentation](https://open.feishu.cn/document/client-docs/bot-v3/add-custom-bot)

| Setting | Value | Description |
| --- | --- | --- |
| `webhook_url` | URL; *secret*, *required* | Webhook URL of the group's custom bot. |
| `secret` | text; *secret* | Signing secret, when the bot has signature verification enabled. |

<a id="flashduty"></a>
### Flashduty (`flashduty`)

[API documentation](https://docs.flashduty.com/en/on-call/integration/alert-integration/alert-sources/standard-alert) · per incident

| Setting | Value | Description |
| --- | --- | --- |
| `integration_key` | text; *secret*, *required* | Integration key of a standard alert integration, or its full push URL. |
| `severity` | `Info`, `Warning`, `Critical`; default `Warning` | Severity of problems and tests. |

<a id="flowtriq"></a>
### Flowtriq (`flowtriq`)

[API documentation](https://flowtriq.com/docs?section=webhooks) · per incident

| Setting | Value | Description |
| --- | --- | --- |
| `webhook_url` | URL; *secret*, *required* | Flowtriq webhook endpoint. |
| `api_key` | text; *secret* | API key sent as the X-API-Key header. |

<a id="fluxer"></a>
### Fluxer (`fluxer`)

[API documentation](https://docs.fluxer.app/http-api/webhooks)

| Setting | Value | Description |
| --- | --- | --- |
| `webhook_url` | URL; *secret*, *required* | Webhook URL from the Webhooks tab of the community or channel settings. |
| `username` | text; default `Vesper` | Display name of the webhook. |
| `prefix_message` | text | Text sent before the embed, such as a mention like `@everyone`. |
| `message_format` | `normal`, `minimalist`; default `normal` | `normal` posts an embed; `minimalist` posts plain text with a status emoji. |

<a id="freemobile"></a>
### Free Mobile (`freemobile`)

[API documentation](https://mobile.free.fr/account/mes-options)

| Setting | Value | Description |
| --- | --- | --- |
| `user` | text; *required* | Free Mobile login (identifiant) of the line that receives the SMS. |
| `api_key` | text; *secret*, *required* | Key shown after enabling Mes Options → Notifications par SMS. |

<a id="goalert"></a>
### GoAlert (`goalert`)

[API documentation](https://goalert.me/docs/alerting/alerting-from-generic-api/) · per incident

| Setting | Value | Description |
| --- | --- | --- |
| `base_url` | URL; *required* | GoAlert server. |
| `token` | text; *secret*, *required* | Generic API integration key of the service. |

<a id="google-chat"></a>
### Google Chat (`google_chat`)

[API documentation](https://developers.google.com/workspace/chat/quickstart/webhooks)

| Setting | Value | Description |
| --- | --- | --- |
| `webhook_url` | URL; *secret*, *required* | Webhook URL from the space's Apps & integrations → Webhooks. |

<a id="google-sheets"></a>
### Google Sheets (`google_sheets`)

[API documentation](https://developers.google.com/apps-script/guides/web)

| Setting | Value | Description |
| --- | --- | --- |
| `webhook_url` | URL; *secret*, *required* | Web app URL of a Google Apps Script deployed with access for anyone, which appends each posted row to the sheet. |

<a id="gorush"></a>
### Gorush (`gorush`)

[API documentation](https://github.com/appleboy/gorush#web-api)

| Setting | Value | Description |
| --- | --- | --- |
| `server_url` | URL; *required* | Gorush server. |
| `device_token` | text; *required* | Push token of the receiving device. |
| `platform` | `ios`, `android`, `huawei`; *required* | Platform of the device: APNs, FCM or Huawei Push Kit. |
| `priority` | `normal`, `high` | Delivery priority. |
| `retry` | integer; default `0` | How many times gorush retries a failed push. |
| `topic` | text | APNs topic (app bundle ID) or FCM topic. |

<a id="gotify"></a>
### Gotify (`gotify`)

[API documentation](https://gotify.net/docs/pushmsg)

| Setting | Value | Description |
| --- | --- | --- |
| `server_url` | URL; *required* | Gotify server. |
| `application_token` | text; *secret*, *required* | Token of the application the messages are posted as, from Apps in the Gotify web UI. |
| `priority` | integer; default `8` | Message priority from 0 (lowest) to 10 (highest). |

<a id="grafana-oncall"></a>
### Grafana OnCall (`grafana_oncall`)

[API documentation](https://grafana.com/docs/oncall/latest/configure/integrations/references/formatted-webhook/) · per incident

| Setting | Value | Description |
| --- | --- | --- |
| `webhook_url` | URL; *secret*, *required* | URL of a Formatted Webhook integration. |

<a id="gtx-messaging"></a>
### GTX Messaging (`gtx_messaging`)

[API documentation](https://www.gtx-messaging.com/en/api-docs/sms-rest-api)

| Setting | Value | Description |
| --- | --- | --- |
| `api_key` | text; *secret*, *required* | API key from the GTX dashboard. |
| `from` | text; *required* | Sender: a phone number or alphanumeric originator (TPOA). |
| `to` | text; *required* | Recipient phone number in E.164 format. |

<a id="halopsa"></a>
### HaloPSA (`halopsa`)

[API documentation](https://halopsa.com/guides/) · per incident

| Setting | Value | Description |
| --- | --- | --- |
| `webhook_url` | URL; *secret*, *required* | Incoming webhook URL of the Halo instance. |
| `username` | text | Username for Basic authentication of the webhook. |
| `password` | text; *secret* | Password for the username. |

<a id="heii-oncall"></a>
### HeyOnCall (Heii On-Call) (`heii_oncall`)

[API documentation](https://heyoncall.com/docs)

| Setting | Value | Description |
| --- | --- | --- |
| `api_key` | text; *secret*, *required* | API key, of the form `heii_…`. |
| `trigger_id` | text; *required* | ID of the manual trigger that problems alert and recoveries resolve. |

<a id="home-assistant"></a>
### Home Assistant (`home_assistant`)

[API documentation](https://developers.home-assistant.io/docs/api/rest/)

| Setting | Value | Description |
| --- | --- | --- |
| `url` | URL; *required* | Home Assistant base URL. |
| `access_token` | text; *secret*, *required* | Long-lived access token, created at the bottom of your Home Assistant profile page. |
| `service` | text; default `notify` | Notify action to call, such as `mobile_app_my_phone`; `notify` reaches all devices. |

<a id="indigo"></a>
### Indigo (`indigo`)

[API documentation](https://wiki.indigodomo.com/2025.1/api)

| Setting | Value | Description |
| --- | --- | --- |
| `server_url` | URL; *required* | Indigo Reflector (`https://NAME.indigodomo.net`) or local Indigo Web Server address. |
| `api_key` | text; *secret*, *required* | API key from the Authorizations page of your Indigo account. |
| `variable_id` | integer; *required* | ID of the variable that receives the message text. |
| `action_group_id` | integer | ID of an action group to run after the variable is updated. |

<a id="jira-service-management"></a>
### Jira Service Management (`jira_service_management`)

[API documentation](https://developer.atlassian.com/cloud/jira/service-desk-ops/rest/v1/api-group-integration-events/) · per incident

| Setting | Value | Description |
| --- | --- | --- |
| `api_key` | text; *secret*, *required* | API key of a Jira Service Management Operations API integration. |
| `priority` | `P1`, `P2`, `P3`, `P4`, `P5`; default `P3` | Priority of problem alerts; tests use P5. |

<a id="keep"></a>
### Keep (`keep`)

[API documentation](https://docs.keephq.dev/overview/alertseverityandstatus) · per incident

| Setting | Value | Description |
| --- | --- | --- |
| `server_url` | URL; *required* | Keep API server. |
| `api_key` | text; *secret*, *required* | Keep API key with permission to write alerts. |

<a id="kook"></a>
### KOOK (`kook`)

[API documentation](https://developer.kookapp.cn/doc/http/message)

| Setting | Value | Description |
| --- | --- | --- |
| `bot_token` | text; *secret*, *required* | Bot token from the KOOK developer center. |
| `channel_id` | text; *required* | ID of the text channel the bot posts to (Uptime Kuma calls it Guild ID); copy it with developer mode on. |

<a id="line"></a>
### LINE Messaging API (`line`)

[API documentation](https://developers.line.biz/en/reference/messaging-api/#send-push-message)

| Setting | Value | Description |
| --- | --- | --- |
| `channel_access_token` | text; *secret*, *required* | Long-lived channel access token from the Messaging API tab of the LINE Developers Console. |
| `user_id` | text; *required* | Your user ID from the Basic settings tab, or a group or room ID the bot is in. |

<a id="lunasea"></a>
### LunaSea (`lunasea`)

[API documentation](https://docs.lunasea.app/lunasea/notifications/custom-notifications)

| Setting | Value | Description |
| --- | --- | --- |
| `target` | `device`, `user`; default `device` | `device` notifies one device; `user` notifies every device signed in to the LunaSea account. |
| `target_id` | text; *secret*, *required* | Device or user identifier: the last part of any module's webhook URL in LunaSea; anyone who has it can notify you. |

<a id="matrix"></a>
### Matrix (`matrix`)

[API documentation](https://spec.matrix.org/latest/client-server-api/#put_matrixclientv3roomsroomidsendeventtypetxnid)

| Setting | Value | Description |
| --- | --- | --- |
| `homeserver_url` | URL; *required* | Client-server API URL of the homeserver. |
| `room_id` | text; *required* | Internal room ID (Room settings → Advanced), not a room alias; the account must have joined it. |
| `access_token` | text; *secret*, *required* | Access token of the sending account. |

<a id="mattermost"></a>
### Mattermost (`mattermost`)

[API documentation](https://developers.mattermost.com/integrate/webhooks/incoming/)

| Setting | Value | Description |
| --- | --- | --- |
| `webhook_url` | URL; *secret*, *required* | Incoming webhook URL from Integrations → Incoming Webhooks. |
| `username` | text; default `Vesper` | Display name of the post. |
| `icon_url` | URL | Image URL used as the post icon. |
| `icon_emoji` | text | Emoji used as the post icon; two space-separated emojis, such as `:white_check_mark: :red_circle:`, set the recovery and problem icons. |
| `channel` | text | Channel name to post into instead of the webhook's default channel. |

<a id="max"></a>
### MAX (`max`)

[API documentation](https://dev.max.ru/docs-api/methods/POST/messages)

| Setting | Value | Description |
| --- | --- | --- |
| `bot_token` | text; *secret*, *required* | Bot token from the MAX business platform (Chat bots → Settings). |
| `api_url` | URL; default `https://platform-api.max.ru` | Bot API server. |
| `chat_id` | text; *required* | Numeric ID of the chat or channel the bot posts to. |

<a id="microsoft-graph"></a>
### Microsoft Graph (Outlook mail) (`microsoft_graph`)

[API documentation](https://learn.microsoft.com/en-us/graph/api/user-sendmail)

| Setting | Value | Description |
| --- | --- | --- |
| `tenant_id` | text; *required* | Directory (tenant) ID of the Entra ID app registration. |
| `client_id` | text; *required* | Application (client) ID of the app registration. |
| `client_secret` | text; *secret*, *required* | Client secret of the app registration, which needs the Mail.Send application permission. |
| `mailbox` | text; *required* | Mailbox (user principal name or address) that sends the mail. |
| `to` | list; *required* | Recipients. |
| `cc` | list | Copy recipients. |
| `bcc` | list | Blind copy recipients. |

<a id="teams"></a>
### Microsoft Teams (`teams`)

[API documentation](https://learn.microsoft.com/en-us/microsoftteams/platform/webhooks-and-connectors/how-to/add-incoming-webhook)

| Setting | Value | Description |
| --- | --- | --- |
| `webhook_url` | URL; *secret*, *required* | Workflows (Power Automate) or incoming webhook URL of the channel. |

<a id="milky"></a>
### Milky (`milky`)

[API documentation](https://milky.ntqqrev.org/api/message)

| Setting | Value | Description |
| --- | --- | --- |
| `server_url` | URL; *required* | HTTP address of the Milky implementation, without the `/api` path. |
| `access_token` | text; *secret* | Access token configured in the Milky implementation. |
| `message_type` | `private`, `group`; default `private` | Send to a friend (`private`) or a group (`group`). |
| `receiver_id` | text; *required* | QQ number of the friend, or the group number. |

<a id="nextcloud-talk"></a>
### Nextcloud Talk (`nextcloud_talk`)

[API documentation](https://nextcloud-talk.readthedocs.io/en/latest/bots/#sending-a-chat-message)

| Setting | Value | Description |
| --- | --- | --- |
| `server_url` | URL; *required* | Address of the Nextcloud server. |
| `conversation_token` | text; *required* | Token of the conversation the bot is set up in, from its URL (`/call/<token>`). |
| `bot_secret` | text; *secret*, *required* | Secret given to `occ talk:bot:install` when installing the bot. |
| `send_silent_up` | true/false | Send recoveries without a notification. |
| `send_silent_down` | true/false | Send problems without a notification. |

<a id="notifery"></a>
### Notifery (`notifery`)

[API documentation](https://docs.notifery.com/api/event/)

| Setting | Value | Description |
| --- | --- | --- |
| `api_key` | text; *secret*, *required* | API key from the Notifery zone settings. |
| `group` | text | Group identifier the events are filed under. |

<a id="notifyapp"></a>
### Notify! (`notifyapp`)

[API documentation](https://getnotifyapp.com/apidocs)

| Setting | Value | Description |
| --- | --- | --- |
| `device_id` | text; *required* | Device ID, or group ID (GRP…), from the Devices tab of the Notify! app. |
| `token` | text; *secret*, *required* | Token belonging to the device or group ID. |
| `icon_url` | URL | HTTPS URL of a small sender icon shown next to the title. |

<a id="ntfy"></a>
### ntfy (`ntfy`)

[API documentation](https://docs.ntfy.sh/publish/#publish-as-json)

| Setting | Value | Description |
| --- | --- | --- |
| `server_url` | URL; default `https://ntfy.sh` | ntfy server. |
| `topic` | text; *required* | Topic name. On a public server anyone who knows it can read it; use a secret reference to keep it private. |
| `priority` | `1`, `2`, `3`, `4`, `5`; default `4` | Priority of problems and tests; recoveries use one less. |
| `username` | text | Username for an access-controlled topic. |
| `password` | text; *secret* | Password for the username. |
| `access_token` | text; *secret* | Access token, instead of username and password. |
| `icon` | URL | Notification icon URL. |

<a id="octopush"></a>
### Octopush (`octopush`)

[API documentation](https://dev.octopush.com/en/sms-gateway-api-documentation/send-sms)

| Setting | Value | Description |
| --- | --- | --- |
| `api_key` | text; *secret*, *required* | API key from the Octopush account. |
| `login` | text; *required* | E-mail address of the Octopush account (api-login). |
| `phone_number` | text; *required* | Recipient phone number in E.164 format. |
| `sms_type` | `sms_premium`, `sms_low_cost`; default `sms_premium` | Premium or low-cost route. |
| `sender_name` | text | Sender name of 3 to 11 letters and digits. |

<a id="onebot"></a>
### OneBot (`onebot`)

[API documentation](https://github.com/botuniverse/onebot-11/blob/master/api/public.md#send_msg-%E5%8F%91%E9%80%81%E6%B6%88%E6%81%AF)

| Setting | Value | Description |
| --- | --- | --- |
| `server_url` | URL; *required* | HTTP API address of the OneBot 11 implementation. |
| `access_token` | text; *secret* | Access token configured in the OneBot implementation. |
| `message_type` | `private`, `group`; default `private` | Send to a user (`private`) or a group (`group`). |
| `receiver_id` | text; *required* | QQ number of the user, or the group number. |

<a id="onechat"></a>
### OneChat (`onechat`)

[API documentation](https://chat-develop.one.th/docs)

| Setting | Value | Description |
| --- | --- | --- |
| `access_token` | text; *secret*, *required* | Bot access token from the OneChat developer console. |
| `receiver_id` | text; *required* | OneChat user ID or group ID that receives the message. |
| `bot_id` | text; *required* | ID of the bot that sends the message. |

<a id="onesender"></a>
### OneSender (`onesender`)

[API documentation](https://onesender.net/docs/)

| Setting | Value | Description |
| --- | --- | --- |
| `url` | URL; *required* | Messages endpoint of your OneSender instance. |
| `token` | text; *secret*, *required* | OneSender API token. |
| `recipient_type` | `private`, `group`; default `private` | Send to a private number or to a group. |
| `receiver` | text; *required* | Phone number with country code (such as `628123456789`) or group ID (such as `628123456789-1473638645`), without the `@` suffix. |

<a id="ooredoo"></a>
### Ooredoo Maldives Bulk SMS (`ooredoo`)

[API documentation](https://business.ooredoo.mv/product/bulk-sms-goveli-sms)

| Setting | Value | Description |
| --- | --- | --- |
| `username` | text; *required* | Bulk SMS account user name. |
| `access_key` | text; *secret*, *required* | Access key of the account; Vesper Base64-encodes it. |
| `bearer_token` | text; *secret*, *required* | Bearer token of the account. |
| `to` | list; *required* | Recipient numbers; a 7-digit local number gets the 960 country code. |
| `server_url` | URL; default `https://o-papi1-lb01.ooredoo.mv/bulk_sms/v2` | Bulk SMS API endpoint. |

<a id="openwa"></a>
### OpenWA (`openwa`)

[API documentation](https://docs.open-wa.org/guides/sending-messages)

| Setting | Value | Description |
| --- | --- | --- |
| `server_url` | URL; *required* | Address of the self-hosted OpenWA gateway, without `/api`. |
| `api_key` | text; *secret*, *required* | OpenWA API key with operator role or higher. |
| `session` | text; *required* | ID of the session that sends the message. |
| `chat_ids` | list; *required* | Chat IDs such as `628123456789@c.us`, `120363021234567890@g.us` or `1234567890@lid`. |

<a id="opsgenie"></a>
### Opsgenie (`opsgenie`)

[API documentation](https://docs.opsgenie.com/docs/alert-api) · per incident

| Setting | Value | Description |
| --- | --- | --- |
| `api_key` | text; *secret*, *required* | API key of an Opsgenie API integration. |
| `region` | `us`, `eu`; default `us` | Region of the Opsgenie account. |
| `priority` | `P1`, `P2`, `P3`, `P4`, `P5`; default `P3` | Priority of problem alerts; tests use P5. |

<a id="opsimate"></a>
### OpsiMate (`opsimate`)

[API documentation](https://docs.opsimate.dev/docs/integrations/overview) · per incident

| Setting | Value | Description |
| --- | --- | --- |
| `server_url` | URL; *required* | OpsiMate server. |
| `api_token` | text; *secret*, *required* | API token of the OpsiMate server (its `API_TOKEN`). |

<a id="pagerduty"></a>
### PagerDuty (`pagerduty`)

[API documentation](https://developer.pagerduty.com/docs/events-api-v2/trigger-events/) · per incident

| Setting | Value | Description |
| --- | --- | --- |
| `integration_key` | text; *secret*, *required* | Events API v2 integration (routing) key of the service. |
| `integration_url` | URL; default `https://events.pagerduty.com/v2/enqueue` | Events API endpoint. |
| `severity` | `info`, `warning`, `error`, `critical`; default `warning` | Severity of triggered alerts. |
| `auto_resolve` | `resolve`, `acknowledge`, `none`; default `resolve` | What a recovery does to the alert. |

<a id="pagertree"></a>
### PagerTree (`pagertree`)

[API documentation](https://pagertree.com/docs/integration-guides/webhook) · per incident

| Setting | Value | Description |
| --- | --- | --- |
| `integration_url` | URL; *secret*, *required* | Endpoint URL of a PagerTree webhook integration. |
| `urgency` | `low`, `medium`, `high`, `critical`; default `medium` | Urgency of created alerts. |
| `auto_resolve` | `resolve`, `none`; default `resolve` | What a recovery does to the alert. |

<a id="pinglet"></a>
### Pinglet (`pinglet`)

[API documentation](https://pinglet.dev/docs/publishing/)

| Setting | Value | Description |
| --- | --- | --- |
| `topic_url` | URL; *required* | Topic URL to publish to, namespace and topic name; the topic is created on first publish. |
| `api_key` | text; *secret*, *required* | Namespace API key, starting with pinglet_. |

<a id="plivo"></a>
### Plivo (`plivo`)

[API documentation](https://www.plivo.com/docs/messaging/api/message)

| Setting | Value | Description |
| --- | --- | --- |
| `auth_id` | text; *required* | Auth ID from the Plivo console. |
| `auth_token` | text; *secret*, *required* | Auth token from the Plivo console. |
| `from_number` | text; *required* | Plivo number in E.164 format, or a sender ID, to send or call from. |
| `to_number` | text; *required* | Recipient phone number in E.164 format. |
| `message_type` | `sms`, `call`; default `sms` | `sms` sends a text message; `call` places a voice call. |
| `answer_url` | URL | Plivo XML answer URL for voice calls; the message is added as its `message` query parameter. |

<a id="promosms"></a>
### PromoSMS (`promosms`)

[API documentation](https://promosms.com/pomoc/api/api-informacje-ogolne)

| Setting | Value | Description |
| --- | --- | --- |
| `login` | text; *required* | PromoSMS API login. |
| `password` | text; *secret*, *required* | PromoSMS API password. |
| `phone_number` | text; *required* | Recipient phone number with country code. |
| `sms_type` | `0`, `1`, `3`, `4`; default `3` | Message type: 0 flash, 1 ECO, 3 FULL (with sender name) or 4 SPEED. |
| `sender_name` | text | Sender name of up to 11 characters, for FULL and SPEED messages. |
| `allow_long_sms` | true/false | Allow up to 4 concatenated messages (639 characters) instead of one (159). |

<a id="pumble"></a>
### Pumble (`pumble`)

[API documentation](https://pumble.com/help/integrations/add-pumble-apps/incoming-webhooks-for-pumble/)

| Setting | Value | Description |
| --- | --- | --- |
| `webhook_url` | URL; *secret*, *required* | Incoming webhook URL of the channel. |

<a id="techulus-push"></a>
### Push by Techulus (`techulus_push`)

[API documentation](https://docs.push.techulus.com/reference/notify)

| Setting | Value | Description |
| --- | --- | --- |
| `api_key` | text; *secret*, *required* | Account or team API key; it decides which devices are notified. |
| `channel` | text | Notification channel: letters, digits and hyphens. |
| `sound` | `default`, `arcade`, `correct`, `fail`, `harp`, `reveal`, `bubble`, `doorbell`, `flute`, `money`, `scifi`, `clear`, `elevator`, `guitar`, `pop` | Notification sound. |
| `time_sensitive` | true/false; default `true` | Deliver problems immediately, even in Do Not Disturb (iOS only). |

<a id="pushbullet"></a>
### Pushbullet (`pushbullet`)

[API documentation](https://docs.pushbullet.com/#create-push)

| Setting | Value | Description |
| --- | --- | --- |
| `access_token` | text; *secret*, *required* | Access token from Settings → Account in Pushbullet; pushes go to all devices of that account. |

<a id="pushdeer"></a>
### PushDeer (`pushdeer`)

[API documentation](https://www.pushdeer.com/dev.html)

| Setting | Value | Description |
| --- | --- | --- |
| `server_url` | URL; default `https://api2.pushdeer.com` | PushDeer API server, for a self-hosted instance. |
| `push_key` | text; *secret*, *required* | PushKey from the Key tab of the PushDeer app; several keys may be separated by commas. |

<a id="pushover"></a>
### Pushover (`pushover`)

[API documentation](https://pushover.net/api)

| Setting | Value | Description |
| --- | --- | --- |
| `user_key` | text; *required* | User or group key of the recipient, from the Pushover dashboard. |
| `app_token` | text; *secret*, *required* | API token of your Pushover application. |
| `device` | text | Send only to this device name instead of all the user's devices. |
| `priority` | `-2`, `-1`, `0`, `1`, `2`; default `0` | Priority from -2 (silent) to 2 (emergency, repeated until acknowledged); recoveries never use emergency. |
| `sound` | `pushover`, `bike`, `bugle`, `cashregister`, `classical`, `cosmic`, `falling`, `gamelan`, `incoming`, `intermission`, `magic`, `mechanical`, `pianobar`, `siren`, `spacealarm`, `tugboat`, `alien`, `climb`, `persistent`, `echo`, `updown`, `vibrate`, `none` | Sound for problems and tests; the user's default when unset. |
| `sound_up` | `pushover`, `bike`, `bugle`, `cashregister`, `classical`, `cosmic`, `falling`, `gamelan`, `incoming`, `intermission`, `magic`, `mechanical`, `pianobar`, `siren`, `spacealarm`, `tugboat`, `alien`, `climb`, `persistent`, `echo`, `updown`, `vibrate`, `none` | Sound for recoveries; the problem sound when unset. |
| `ttl` | integer | Seconds after which the message is deleted from devices. |

<a id="pushplus"></a>
### PushPlus (`pushplus`)

[API documentation](https://www.pushplus.plus/doc/guide/api.html)

| Setting | Value | Description |
| --- | --- | --- |
| `token` | text; *secret*, *required* | User or message token from the PushPlus console. |
| `topic` | text | Group code to send to the group's members instead of only yourself. |

<a id="pushy"></a>
### Pushy (`pushy`)

[API documentation](https://pushy.me/docs/api/send-notifications)

| Setting | Value | Description |
| --- | --- | --- |
| `api_key` | text; *secret*, *required* | Secret API key of your Pushy app, from the Pushy dashboard. |
| `device_token` | text; *required* | Device token of the recipient, or a topic such as /topics/alerts. |

<a id="resend"></a>
### Resend (`resend`)

[API documentation](https://resend.com/docs/api-reference/emails/send-email)

| Setting | Value | Description |
| --- | --- | --- |
| `api_key` | text; *secret*, *required* | API key with sending access. |
| `from_email` | text; *required* | Sender address on a verified domain. |
| `from_name` | text; default `Vesper` | Sender name. |
| `to` | list; *required* | Recipients, at most 50. |
| `subject_prefix` | text | Text before the subject, such as `[homelab]`. |

<a id="rocket-chat"></a>
### Rocket.Chat (`rocket_chat`)

[API documentation](https://docs.rocket.chat/docs/integrations)

| Setting | Value | Description |
| --- | --- | --- |
| `webhook_url` | URL; *secret*, *required* | Incoming webhook URL from Administration → Integrations. |
| `username` | text | Display name of the post. |
| `icon_emoji` | text | Emoji used as the post icon, such as `:rotating_light:`. |
| `channel` | text | Channel (`#name`) or user (`@name`) to post to instead of the webhook's default. |

<a id="sendgrid"></a>
### SendGrid (`sendgrid`)

[API documentation](https://www.twilio.com/docs/sendgrid/api-reference/mail-send/mail-send)

| Setting | Value | Description |
| --- | --- | --- |
| `api_key` | text; *secret*, *required* | API key with the Mail Send permission. |
| `from_email` | text; *required* | Verified sender address. |
| `to` | list; *required* | Recipients. |
| `cc` | list | Copy recipients. |
| `bcc` | list | Blind copy recipients. |
| `subject_prefix` | text | Text before the subject, such as `[homelab]`. |

<a id="serverchan"></a>
### ServerChan (`serverchan`)

[API documentation](https://doc.sc3.ft07.com/en/serverchan3/server/api)

| Setting | Value | Description |
| --- | --- | --- |
| `send_key` | text; *secret*, *required* | SendKey of ServerChan Turbo (SCT…) or ServerChan³ (sctp…); the key decides the endpoint. |

<a id="serwersms"></a>
### SerwerSMS.pl (`serwersms`)

[API documentation](https://dev.serwersms.pl/en/https-api-v2/sending-sms-messages-with-the-same-content)

| Setting | Value | Description |
| --- | --- | --- |
| `username` | text; *required* | API user name from Interface settings → HTTPS XML API → Users. |
| `password` | text; *secret*, *required* | Password of the API user. |
| `recipient_type` | `phone`, `group`; default `phone` | Send to one phone number or to a contact group. |
| `recipient` | text; *required* | Phone number, or the group ID from the Customer Panel when recipient_type is group. |
| `sender_name` | text | Approved sender name for SMS FULL; without one an SMS ECO is sent. |

<a id="sevenio"></a>
### seven (`sevenio`)

[API documentation](https://docs.seven.io/en/rest-api/endpoints/sms)

| Setting | Value | Description |
| --- | --- | --- |
| `api_key` | text; *secret*, *required* | API key from the seven dashboard (Developer section). |
| `sender` | text; default `Vesper` | Sender: up to 11 alphanumeric or 16 numeric characters. |
| `receiver` | text; *required* | Recipient number in international format; separate several with commas. |

<a id="signal"></a>
### Signal (`signal`)

[API documentation](https://bbernhard.github.io/signal-cli-rest-api/)

| Setting | Value | Description |
| --- | --- | --- |
| `url` | URL; *required* | Send endpoint of your signal-cli-rest-api server. |
| `number` | text; *required* | Registered Signal number that sends the message, in international format. |
| `recipients` | list; *required* | Phone numbers in international format, or group IDs, that receive the message. |

<a id="signalgrid"></a>
### Signalgrid (`signalgrid`)

[API documentation](https://docs.signalgrid.co/api/push-api)

| Setting | Value | Description |
| --- | --- | --- |
| `client_key` | text; *secret*, *required* | Client key of your Signalgrid account. |
| `channel` | text; *required* | Token of the channel whose subscribers are notified. |
| `critical` | true/false | Deliver problems as critical alerts that bypass silent mode, where the app allows it. |

<a id="signl4"></a>
### SIGNL4 (`signl4`)

[API documentation](https://docs.signl4.com/integrations/webhook/webhook.html) · per incident

| Setting | Value | Description |
| --- | --- | --- |
| `webhook_url` | URL; *secret*, *required* | Team webhook URL, which contains the team secret. |

<a id="slack"></a>
### Slack (`slack`)

[API documentation](https://api.slack.com/messaging/webhooks)

| Setting | Value | Description |
| --- | --- | --- |
| `webhook_url` | URL; *secret*, *required* | Incoming webhook URL from the Slack app's Incoming Webhooks page. |
| `username` | text | Display name of the post; only legacy webhooks honour it. |
| `icon_emoji` | text | Emoji used as the post icon, such as `:rotating_light:`; only legacy webhooks honour it. |
| `channel` | text | Channel to post into, such as `#alerts`; only legacy webhooks honour it. |
| `rich_message` | true/false | Send problems and recoveries as a coloured block message with a button. |
| `channel_notify` | true/false | Mention @channel in every message. |

<a id="sms-gateway"></a>
### SMS Gateway (`sms_gateway`)

[API documentation](https://github.com/mattboston/sms-gateway)

| Setting | Value | Description |
| --- | --- | --- |
| `server_url` | URL; *required* | Address of the self-hosted SMS Gateway. |
| `api_key` | text; *secret*, *required* | API key of the gateway. |
| `to` | list; *required* | Recipient phone numbers in international format. |

<a id="smspartner"></a>
### SMS Partner (`smspartner`)

[API documentation](https://www.docpartner.dev/en/api/sms-partner)

| Setting | Value | Description |
| --- | --- | --- |
| `api_key` | text; *secret*, *required* | API key from the SMS Partner dashboard. |
| `phone_numbers` | list; *required* | Recipient phone numbers in international format. |
| `sender_name` | text; *required* | Sender name of 3 to 11 letters and digits; longer names are cut. |

<a id="smsir"></a>
### SMS.ir (`smsir`)

[API documentation](https://sms.ir/rest-api/)

| Setting | Value | Description |
| --- | --- | --- |
| `api_key` | text; *secret*, *required* | API key from the SMS.ir panel (Developers, API keys). |
| `mobiles` | list; *required* | Recipient mobile numbers, such as `9123456789` or `09123456789`. |
| `template_id` | integer; *required* | ID of the verify (fast send) template defined in the SMS.ir panel. |
| `parameter_name` | text; default `uptkumaalert` | Template variable that receives the alert, written `#name#` in the template. |

<a id="smsc"></a>
### SMSC (`smsc`)

[API documentation](https://smsc.kz/api/http/send)

| Setting | Value | Description |
| --- | --- | --- |
| `login` | text; *required* | SMSC account login. |
| `password` | text; *secret*, *required* | SMSC account password or API password. |
| `to_number` | text; *required* | Recipient phone number in international format; separate several with commas. |
| `sender_name` | text | Registered sender name of up to 15 characters; leave empty for the shared sender. |
| `translit` | `0`, `1`, `2`; default `0` | 0 sends the text as is, 1 transliterates Cyrillic to Latin (translit), 2 to look-alike Latin letters. |

<a id="smseagle"></a>
### SMSEagle (`smseagle`)

[API documentation](https://www.smseagle.eu/docs/apiv2/)

| Setting | Value | Description |
| --- | --- | --- |
| `server_url` | URL; *required* | Address of the SMSEagle device. |
| `access_token` | text; *secret*, *required* | API access token created in the device's web GUI. |
| `api_version` | `v2`, `v1`; default `v2` | `v2` for current firmware, `v1` for the legacy HTTP API. |
| `recipient_type` | `phone`, `group`, `contact`; default `phone` | Whether recipients are phone numbers, phonebook groups or phonebook contacts. |
| `recipients` | list; *required* | Phone numbers in international format, or group or contact IDs (APIv2) or names (APIv1). |
| `message_type` | `sms`, `ring`, `tts`, `tts_advanced`; default `sms` | Send an SMS, ring only, or call and read the message aloud (text-to-speech). |
| `priority` | integer; default `0` | SMS priority from 0 to 9; APIv1 treats any value above 0 as high. |
| `unicode` | true/false | Encode SMS as Unicode for national characters. |
| `duration` | integer; default `10` | Call duration in seconds, for ring and text-to-speech calls. |
| `voice_id` | integer; default `1` | Voice model ID for advanced text-to-speech calls. |

<a id="smsmanager"></a>
### SmsManager (`smsmanager`)

[API documentation](https://smsmanager.cz/docs/api-reference/http-api/send)

| Setting | Value | Description |
| --- | --- | --- |
| `api_key` | text; *secret*, *required* | API key from app.smsmanager.com. |
| `numbers` | list; *required* | Recipient phone numbers in international format. |
| `gateway` | `high`, `economy`, `low`; default `high` | Message route (price and quality level). |

<a id="sms-planet"></a>
### SMSPLANET (`sms_planet`)

[API documentation](https://smsplanet.pl/doc/slate/index.html)

| Setting | Value | Description |
| --- | --- | --- |
| `api_token` | text; *secret*, *required* | API token from the SMSPLANET customer panel. |
| `phone_numbers` | list; *required* | Recipient phone numbers. |
| `sender_name` | text; *required* | Sender name of up to 11 characters, approved in the SMSPLANET panel. |

<a id="splunk"></a>
### Splunk On-Call (`splunk`)

[API documentation](https://help.victorops.com/knowledge-base/rest-endpoint-integration-guide/) · per incident

| Setting | Value | Description |
| --- | --- | --- |
| `rest_url` | URL; *secret*, *required* | REST endpoint URL including the API key and routing key. |
| `severity` | `critical`, `warning`, `info`; default `critical` | Message type of problems and tests; `info` only adds a timeline entry. |
| `auto_resolve` | `recovery`, `acknowledgement`, `none`; default `recovery` | What a recovery does to the incident. |

<a id="spugpush"></a>
### Spug Push (`spugpush`)

[API documentation](https://push.spug.cc/guide/send)

| Setting | Value | Description |
| --- | --- | --- |
| `template_key` | text; *secret*, *required* | Code of a notification template that uses ${title} and ${content}, such as the Uptime Kuma template from the template market. |

<a id="squadcast"></a>
### Squadcast (`squadcast`)

[API documentation](https://developers.squadcast.com/incoming-webhooks/payload-format) · per incident

| Setting | Value | Description |
| --- | --- | --- |
| `webhook_url` | URL; *secret*, *required* | Incident Webhook URL of the Squadcast service. |

<a id="stackfield"></a>
### Stackfield (`stackfield`)

[API documentation](https://www.stackfield.com/developer-api)

| Setting | Value | Description |
| --- | --- | --- |
| `webhook_url` | URL; *secret*, *required* | Chat message webhook URL from Room settings → Integrations. |

<a id="telegram"></a>
### Telegram (`telegram`)

[API documentation](https://core.telegram.org/bots/api#sendmessage)

| Setting | Value | Description |
| --- | --- | --- |
| `bot_token` | text; *secret*, *required* | Bot token from @BotFather. |
| `chat_id` | text; *secret*, *required* | Chat, group or channel ID; message the bot first so it may write to you. |
| `message_thread_id` | text | Topic (forum thread) ID in a supergroup. |
| `silent` | true/false | Deliver without sound. |
| `protect_content` | true/false | Prevent forwarding and saving. |
| `server_url` | URL; default `https://api.telegram.org` | Bot API server, for a self-hosted telegram-bot-api. |

<a id="telnyx"></a>
### Telnyx (`telnyx`)

[API documentation](https://developers.telnyx.com/api/messaging/send-message)

| Setting | Value | Description |
| --- | --- | --- |
| `api_key` | text; *secret*, *required* | API key from the Telnyx portal. |
| `from_number` | text; *required* | Telnyx number in E.164 format, short code or alphanumeric sender ID to send from. |
| `to_number` | text; *required* | Recipient phone number in E.164 format. |
| `messaging_profile_id` | text | Messaging profile to send through, required for alphanumeric sender IDs. |

<a id="teltonika"></a>
### Teltonika (`teltonika`)

[API documentation](https://developers.teltonika-networks.com/reference/rut241/7.19.4/v1.11.1/messages)

| Setting | Value | Description |
| --- | --- | --- |
| `url` | URL; *required* | Router address; only scheme, host and port are used (RutOS 7.14 or later). |
| `username` | text; *required* | Router user allowed to send SMS. |
| `password` | text; *secret*, *required* | Password of that user. |
| `modem` | text; *required* | Modem ID, such as `1-1`. |
| `phone_number` | text; *required* | Recipient phone number in international format. |

<a id="threema"></a>
### Threema Gateway (`threema`)

[API documentation](https://gateway.threema.ch/en/developer/api)

| Setting | Value | Description |
| --- | --- | --- |
| `sender_identity` | text; *required* | Gateway ID in basic mode, eight characters starting with `*`. |
| `secret` | text; *secret*, *required* | API authentication secret of the Gateway ID. |
| `recipient_type` | `identity`, `phone`, `email`; default `identity` | Whether the recipient is a Threema ID, a phone number or an email address linked to one. |
| `recipient` | text; *required* | Threema ID (eight characters), phone number in international format without `+` (such as `41791234567`), or email address. |

<a id="turbosmtp"></a>
### turboSMTP (`turbosmtp`)

[API documentation](https://serversmtp.com/docs/api-reference/overview)

| Setting | Value | Description |
| --- | --- | --- |
| `consumer_key` | text; *secret*, *required* | Consumer key with HTTP API sending permission. |
| `consumer_secret` | text; *secret*, *required* | Consumer secret of the key pair. |
| `region` | `us`, `eu`; default `us` | Sending infrastructure: `us` (global) or `eu`. |
| `from_email` | text; *required* | Sender, an address or `Name <address>`. |
| `to` | list; *required* | Recipients. |
| `cc` | list | Copy recipients. |
| `bcc` | list | Blind copy recipients. |
| `subject_prefix` | text | Text before the subject, such as `[homelab]`. |

<a id="twilio"></a>
### Twilio (`twilio`)

[API documentation](https://www.twilio.com/docs/messaging/api/message-resource#create-a-message-resource)

| Setting | Value | Description |
| --- | --- | --- |
| `account_sid` | text; *required* | Account SID from the Twilio console, starting with AC. |
| `api_key_sid` | text | API key SID (SK…) to authenticate with instead of the account SID; auth_token is then the key's secret. |
| `auth_token` | text; *secret*, *required* | Auth token of the account, or the secret of the API key. |
| `from_number` | text; *required* | Twilio number in E.164 format, or an alphanumeric sender ID, to send from. |
| `to_number` | text; *required* | Recipient phone number in E.164 format. |
| `messaging_service_sid` | text | Messaging Service SID (MG…) to send through. |

<a id="vk"></a>
### VK (`vk`)

[API documentation](https://dev.vk.com/en/method/messages.send)

| Setting | Value | Description |
| --- | --- | --- |
| `access_token` | text; *secret*, *required* | Community or user access token with the messages permission. |
| `api_version` | text; default `5.199` | VK API version to call. |
| `peer_id` | text; *required* | Destination: a user ID, 2000000000 plus a chat ID for a group chat, or minus a community ID. |
| `dont_parse_links` | true/false | Do not turn links in the message into snippets. |

<a id="vkteams"></a>
### VK Teams (`vkteams`)

[API documentation](https://teams.vk.com/botapi/)

| Setting | Value | Description |
| --- | --- | --- |
| `bot_token` | text; *secret*, *required* | Bot token from Metabot. |
| `chat_id` | text; *required* | Chat ID or user login (email) the bot writes to; message the bot first. |
| `base_url` | URL; default `https://myteam.mail.ru` | Bot API server, for on-premise VK Teams installations. |

<a id="waha"></a>
### WAHA (WhatsApp HTTP API) (`waha`)

[API documentation](https://waha.devlike.pro/docs/how-to/send-messages/)

| Setting | Value | Description |
| --- | --- | --- |
| `server_url` | URL; *required* | Address of the self-hosted WAHA server. |
| `api_key` | text; *secret* | API key, when WAHA is started with WAHA_API_KEY. |
| `session` | text; default `default` | WAHA session that sends the message. |
| `chat_id` | text; *required* | Chat ID: a phone number with country code (`11234567890@c.us`) or a group (`123456789012345678@g.us`). |

<a id="webhook"></a>
### Webhook (`webhook`)

[API documentation](https://github.com/cfbender/vesper/blob/main/docs/notifications.md#webhook)

| Setting | Value | Description |
| --- | --- | --- |
| `url` | URL; *required* | Endpoint that receives the notification. |
| `content_type` | `json`, `form`; default `json` | `json` posts the message object; `form` posts its fields form-encoded. |
| `authorization` | text; *secret* | Value of the Authorization header. |

<a id="wecom"></a>
### WeCom (`wecom`)

[API documentation](https://developer.work.weixin.qq.com/document/path/91770)

| Setting | Value | Description |
| --- | --- | --- |
| `bot_key` | text; *secret*, *required* | Key of the group robot, the `key` parameter of its webhook URL. |
| `mentioned_mobile_list` | list | Mobile numbers of members to mention, or `@all`. |

<a id="whapi"></a>
### Whapi.Cloud (`whapi`)

[API documentation](https://whapi.readme.io/reference/sendmessagetext)

| Setting | Value | Description |
| --- | --- | --- |
| `server_url` | URL; default `https://gate.whapi.cloud` | Whapi API gateway. |
| `auth_token` | text; *secret*, *required* | Channel API token from the Whapi dashboard. |
| `recipient` | text; *required* | Phone number with country code and no `+`, or a chat or group ID such as `120363194020948049@g.us`. |

<a id="wpush"></a>
### WPush (`wpush`)

[API documentation](https://wpush.cn/docs)

| Setting | Value | Description |
| --- | --- | --- |
| `api_key` | text; *secret*, *required* | API key from the WPush settings page; it starts with WPUSH. |
| `channel` | `wechat`, `sms`, `mail`, `app`, `webhook`, `feishu`, `dingtalk`, `wechat_work`, `clawbot`, `qqbot`; default `wechat` | Delivery channel bound in the WPush console. |

<a id="wxpusher"></a>
### WxPusher (`wxpusher`)

[API documentation](https://wxpusher.zjiecode.com/docs/#/?id=spt)

| Setting | Value | Description |
| --- | --- | --- |
| `spt` | list; *secret*, *required* | Simple push tokens (SPT) of the recipients, from scanning the WxPusher QR code. |

<a id="yzj"></a>
### YZJ (Yunzhijia) (`yzj`)

[API documentation](https://www.yunzhijia.com/opendocs/docs.html#/tutorial/index/robot)

| Setting | Value | Description |
| --- | --- | --- |
| `webhook_url` | URL; *required* | Robot webhook URL, without its query string. |
| `token` | text; *secret*, *required* | Robot token, the `yzjtoken` parameter of the webhook URL. |

<a id="zoho-cliq"></a>
### Zoho Cliq (`zoho_cliq`)

[API documentation](https://www.zoho.com/cliq/help/restapi/v2/#Post_Message_Channel)

| Setting | Value | Description |
| --- | --- | --- |
| `webhook_url` | URL; *secret*, *required* | Channel message URL including its `zapikey` webhook token. |
<!-- catalog:end -->
