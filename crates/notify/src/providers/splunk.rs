use super::prelude::*;

pub const PROVIDER: Provider = Provider::new("splunk", "Splunk On-Call", FIELDS, build)
    .docs("https://help.victorops.com/knowledge-base/rest-endpoint-integration-guide/")
    .identity(&["rest_url"])
    .per_incident();

const FIELDS: &[Field] = &[
    Field::secret_url(
        "rest_url",
        "REST endpoint URL including the API key and routing key.",
    )
    .example("https://alert.victorops.com/integrations/generic/20131114/alert/0123abcd-4567-89ef-0123-456789abcdef/vesper"),
    Field::choice(
        "severity",
        &["critical", "warning", "info"],
        "Message type of problems and tests; `info` only adds a timeline entry.",
    )
    .default("critical"),
    Field::choice(
        "auto_resolve",
        &["recovery", "acknowledgement", "none"],
        "What a recovery does to the incident.",
    )
    .default("recovery"),
];

fn build(s: &Settings, m: &Message) -> Result<Vec<Request>, String> {
    let message_type = match (m.status, s.str("auto_resolve")?) {
        (Status::Up, "none") => return Ok(vec![]),
        (Status::Up, action) => action,
        _ => s.str("severity")?,
    };
    let mut body = json!({
        "message_type": message_type.to_uppercase(),
        "entity_id": m.dedup_key(),
        "entity_display_name": m.title,
        "state_message": m.body(),
        "monitoring_tool": "Vesper",
    });
    if m.status != Status::Up {
        body["vo_annotate.u.Vesper"] = json!(m.link);
    }
    Ok(vec![HttpRequest::post(s.str("rest_url")?)
        .json(body)
        .accept(classify)
        .into()])
}

/// The endpoint reports `{"result": "success"}` or `{"result": "failure", "message": …}`.
fn classify(r: &Response) -> Verdict {
    body_code(r, "/result", "success")
}
