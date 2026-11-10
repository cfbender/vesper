use super::prelude::*;

pub const PROVIDER: Provider =
    Provider::new("heii_oncall", "HeyOnCall (Heii On-Call)", FIELDS, build)
        .docs("https://heyoncall.com/docs")
        .identity(&["trigger_id"]);

const FIELDS: &[Field] = &[
    Field::secret("api_key", "API key, of the form `heii_…`."),
    Field::text(
        "trigger_id",
        "ID of the manual trigger that problems alert and recoveries resolve.",
    )
    .example("0f1e2d3c-4b5a-6978-8796-a5b4c3d2e1f0"),
];

/// A trigger is one alerting/resolved state, so every problem message
/// alerts it and a message with only recoveries resolves it.
fn build(s: &Settings, m: &Message) -> Result<Vec<Request>, String> {
    let action = match m.status {
        Status::Up => "resolve",
        _ => "alert",
    };
    let url = format!(
        "https://api.heyoncall.com/triggers/{}/{action}",
        encode(s.str("trigger_id")?)
    );
    let body = json!({
        "title": m.title,
        "message": m.summary(),
        "url": m.link,
        "status": m.status_word(),
    });
    Ok(vec![HttpRequest::post(url)
        .bearer(s.str("api_key")?)
        .header("Accept", "application/json")
        .json(body)
        .into()])
}
