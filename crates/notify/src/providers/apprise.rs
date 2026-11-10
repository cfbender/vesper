use super::prelude::*;

pub const PROVIDER: Provider = Provider::new("apprise", "Apprise API", FIELDS, build)
    .docs("https://github.com/caronc/apprise-api#api-details")
    .identity(&["server_url", "urls", "config_key", "tag"]);

const FIELDS: &[Field] = &[
    Field::url("server_url", "Apprise API server.").example("http://apprise:8000"),
    Field::secret(
        "urls",
        "Apprise URLs to notify, separated by commas or spaces; without them and without config_key the server's APPRISE_STATELESS_URLS are used.",
    )
    .optional()
    .example("tgram://bottoken/ChatID"),
    Field::text(
        "config_key",
        "Key of a configuration saved on the server, used instead of urls.",
    )
    .optional()
    .example("vesper"),
    Field::text(
        "tag",
        "Notify only the saved configuration's URLs with this tag (with config_key).",
    )
    .optional(),
    Field::text("username", "Basic Auth user name, when the server requires a login.").optional(),
    Field::secret("password", "Basic Auth password for the username.").optional(),
];

fn build(s: &Settings, m: &Message) -> Result<Vec<Request>, String> {
    let kind = match m.status {
        Status::Down => "failure",
        Status::Up => "success",
        Status::Test => "info",
    };
    let mut body = json!({
        "title": m.title,
        "body": m.body(),
        "type": kind,
        "format": "text",
    });
    let base = s.base_url("server_url")?;
    let url = match s.get("config_key") {
        Some(key) => {
            if let Some(tag) = s.get("tag") {
                body["tag"] = json!(tag);
            }
            join(&base, &format!("notify/{}", encode(key)))
        }
        None => {
            if let Some(urls) = s.get("urls") {
                body["urls"] = json!(urls);
            }
            join(&base, "notify/")
        }
    };
    let mut request = HttpRequest::post(url).json(body).accept(classify);
    if let Some(user) = s.get("username") {
        request = request.basic(user, s.get("password").unwrap_or_default());
    }
    Ok(vec![request.into()])
}

/// 424 means at least one downstream notification failed, which is worth
/// retrying; everything else follows the status code.
fn classify(r: &Response) -> Verdict {
    match r.status {
        424 => Verdict::Transient(r.summary()),
        _ => by_status(r),
    }
}
