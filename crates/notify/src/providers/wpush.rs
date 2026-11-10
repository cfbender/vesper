use super::prelude::*;

pub const PROVIDER: Provider = Provider::new("wpush", "WPush", FIELDS, build)
    .docs("https://wpush.cn/docs")
    .identity(&["api_key", "channel"]);

const FIELDS: &[Field] = &[
    Field::secret(
        "api_key",
        "API key from the WPush settings page; it starts with WPUSH.",
    )
    .example("WPUSHabcdefghijklmnopqrstuvwxyz0"),
    Field::choice(
        "channel",
        &[
            "wechat",
            "sms",
            "mail",
            "app",
            "webhook",
            "feishu",
            "dingtalk",
            "wechat_work",
            "clawbot",
            "qqbot",
        ],
        "Delivery channel bound in the WPush console.",
    )
    .default("wechat"),
];

fn build(s: &Settings, m: &Message) -> Result<Vec<Request>, String> {
    let form = [
        ("apikey", s.str("api_key")?.to_string()),
        ("title", truncate(&m.title, 255)),
        ("content", m.summary()),
        ("url", m.link.clone()),
        ("channel", s.str("channel")?.to_string()),
    ];
    Ok(vec![HttpRequest::post("https://api.wpush.cn/api/v1/send")
        .form(form)
        .accept(|r| body_code(r, "/code", "0"))
        .into()])
}
