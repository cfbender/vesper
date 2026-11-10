use super::prelude::*;

pub const PROVIDER: Provider = Provider::new("pushbullet", "Pushbullet", FIELDS, build)
    .docs("https://docs.pushbullet.com/#create-push")
    .identity(&["access_token"]);

const FIELDS: &[Field] = &[Field::secret(
    "access_token",
    "Access token from Settings → Account in Pushbullet; pushes go to all devices of that account.",
)
.example("o.AbCdEf0123456789")];

fn build(s: &Settings, m: &Message) -> Result<Vec<Request>, String> {
    let body = json!({
        "type": "link",
        "title": m.title,
        "body": m.summary(),
        "url": m.link,
    });
    Ok(vec![HttpRequest::post(
        "https://api.pushbullet.com/v2/pushes",
    )
    .header("Access-Token", s.str("access_token")?)
    .json(body)
    .into()])
}
