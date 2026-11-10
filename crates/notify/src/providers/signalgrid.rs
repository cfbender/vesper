use super::prelude::*;

pub const PROVIDER: Provider = Provider::new("signalgrid", "Signalgrid", FIELDS, build)
    .docs("https://docs.signalgrid.co/api/push-api")
    .identity(&["channel"]);

const FIELDS: &[Field] = &[
    Field::secret("client_key", "Client key of your Signalgrid account."),
    Field::text(
        "channel",
        "Token of the channel whose subscribers are notified.",
    )
    .example("ch_a1b2c3d4e5"),
    Field::bool(
        "critical",
        "Deliver problems as critical alerts that bypass silent mode, where the app allows it.",
    ),
];

fn build(s: &Settings, m: &Message) -> Result<Vec<Request>, String> {
    let kind = match m.status {
        Status::Down => "CRIT",
        Status::Up => "SUCCESS",
        Status::Test => "INFO",
    };
    let critical = m.status == Status::Down && s.bool("critical");
    let form = [
        ("client_key", s.str("client_key")?.to_string()),
        ("channel", s.str("channel")?.to_string()),
        ("title", m.title.clone()),
        ("body", m.body()),
        ("type", kind.to_string()),
        ("critical", critical.to_string()),
    ];
    Ok(vec![HttpRequest::post("https://api.signalgrid.co/v1/push")
        .form(form)
        .accept(classify)
        .into()])
}

/// Signalgrid echoes the outcome as a text `code` in the body; anything but
/// `200` there is a failure even when the HTTP status is 200.
fn classify(r: &Response) -> Verdict {
    match by_status(r) {
        Verdict::Accepted => {}
        other => return other,
    }
    let json = r.json();
    let code = json
        .get("code")
        .map(|c| c.as_str().map_or_else(|| c.to_string(), str::to_string));
    match code.as_deref() {
        None | Some("200") => Verdict::Accepted,
        Some("429") => Verdict::RetryAfter(r.retry_after.unwrap_or(30).max(1)),
        Some(c) if c.starts_with('5') => Verdict::Transient(r.summary()),
        Some(_) => Verdict::Rejected(r.summary()),
    }
}
