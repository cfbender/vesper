use super::prelude::*;

pub const PROVIDER: Provider = Provider::new("signal", "Signal", FIELDS, build)
    .docs("https://bbernhard.github.io/signal-cli-rest-api/")
    .identity(&["recipients"]);

const FIELDS: &[Field] = &[
    Field::url("url", "Send endpoint of your signal-cli-rest-api server.")
        .example("http://signal-api:8080/v2/send"),
    Field::text(
        "number",
        "Registered Signal number that sends the message, in international format.",
    )
    .example("+15551234567"),
    Field::list(
        "recipients",
        "Phone numbers in international format, or group IDs, that receive the message.",
    )
    .example("+15557654321"),
];

fn build(s: &Settings, m: &Message) -> Result<Vec<Request>, String> {
    let recipients: Vec<String> = s
        .required_list("recipients")?
        .into_iter()
        .map(|r| r.split_whitespace().collect())
        .collect();
    let body = json!({
        "message": m.text(),
        "number": s.str("number")?,
        "recipients": recipients,
    });
    Ok(vec![HttpRequest::post(s.str("url")?).json(body).into()])
}
