use super::prelude::*;

pub const PROVIDER: Provider = Provider::new("alerta", "Alerta", FIELDS, build)
    .docs("https://docs.alerta.io/api/reference.html#create-an-alert")
    .identity(&["api_endpoint", "environment"])
    .per_incident();

const FIELDS: &[Field] = &[
    Field::url("api_endpoint", "Alert endpoint of the Alerta API.")
        .example("https://alerta.example.com/api/alert"),
    Field::text("environment", "Alerta environment of the alerts.").example("Production"),
    Field::secret(
        "api_key",
        "API key, unless the server runs without authentication.",
    )
    .optional(),
    Field::text("alert_state", "Severity of problems and tests.").default("critical"),
    Field::text(
        "recover_state",
        "Severity of recoveries; a normal severity closes the alert.",
    )
    .default("cleared"),
];

fn build(s: &Settings, m: &Message) -> Result<Vec<Request>, String> {
    let severity = match m.status {
        Status::Up => s.str("recover_state")?,
        _ => s.str("alert_state")?,
    };
    let resource = m
        .incident
        .as_ref()
        .map_or("Vesper".to_string(), |i| i.service_id.clone());
    let body = json!({
        "resource": resource,
        "event": m.dedup_key(),
        "environment": s.str("environment")?,
        "severity": severity,
        "service": ["Vesper"],
        "group": "Vesper",
        "value": m.status_word(),
        "text": m.summary(),
        "tags": ["vesper"],
        "attributes": { "link": m.link },
        "origin": "vesper",
        "type": "vesperAlert",
    });
    let mut request = HttpRequest::post(s.str("api_endpoint")?).json(body);
    if let Some(key) = s.get("api_key") {
        request = request.header("Authorization", format!("Key {key}"));
    }
    Ok(vec![request.into()])
}
