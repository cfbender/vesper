use super::prelude::*;

pub const PROVIDER: Provider = Provider::new("bitrix24", "Bitrix24", FIELDS, build)
    .docs(
        "https://apidocs.bitrix24.com/api-reference/chats/notifications/im-notify-system-add.html",
    )
    .identity(&["webhook_url", "user_id"]);

const FIELDS: &[Field] = &[
    Field::secret_url(
        "webhook_url",
        "Inbound webhook URL with the `im` scope, from Developer resources → Other → Inbound webhook.",
    )
    .example("https://example.bitrix24.com/rest/1/abc123xyz"),
    Field::text("user_id", "ID of the user who receives the system notification.").example("1"),
];

fn build(s: &Settings, m: &Message) -> Result<Vec<Request>, String> {
    let color = match m.status {
        Status::Down => "#b73419",
        Status::Up => "#67b518",
        Status::Test => "#2fc6f6",
    };
    let url = join(&s.base_url("webhook_url")?, "im.notify.system.add.json");
    let request = HttpRequest::post(url).form([
        ("USER_ID", s.str("user_id")?.to_string()),
        ("MESSAGE", format!("[B]{}[/B]", m.title)),
        ("ATTACH[COLOR]", color.to_string()),
        ("ATTACH[BLOCKS][0][MESSAGE]", m.body()),
    ]);
    Ok(vec![request.into()])
}
