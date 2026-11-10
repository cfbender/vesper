use super::prelude::*;

pub const PROVIDER: Provider = Provider::new("spugpush", "Spug Push", FIELDS, build)
    .docs("https://push.spug.cc/guide/send")
    .identity(&["template_key"]);

const FIELDS: &[Field] = &[Field::secret(
    "template_key",
    "Code of a notification template that uses ${title} and ${content}, such as the Uptime Kuma template from the template market.",
)
.example("a1b2c3d4e5f6")];

fn build(s: &Settings, m: &Message) -> Result<Vec<Request>, String> {
    let url = format!(
        "https://push.spug.cc/send/{}",
        encode(s.str("template_key")?)
    );
    let body = json!({
        "title": m.title,
        "content": m.body(),
    });
    Ok(vec![HttpRequest::post(url)
        .json(body)
        .accept(|r| body_code(r, "/code", "200"))
        .into()])
}
