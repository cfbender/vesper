use super::prelude::*;

pub const PROVIDER: Provider = Provider::new("opsgenie", "Opsgenie", FIELDS, build)
    .docs("https://docs.opsgenie.com/docs/alert-api")
    .identity(&["region", "api_key"])
    .per_incident();

const FIELDS: &[Field] = &[
    Field::secret("api_key", "API key of an Opsgenie API integration."),
    Field::choice("region", &["us", "eu"], "Region of the Opsgenie account.").default("us"),
    Field::choice(
        "priority",
        &["P1", "P2", "P3", "P4", "P5"],
        "Priority of problem alerts; tests use P5.",
    )
    .default("P3"),
];

/// Alias of the alert; slashes are replaced so it can be used in a URL path.
fn alias(m: &Message) -> String {
    m.dedup_key().replace('/', "-")
}

fn build(s: &Settings, m: &Message) -> Result<Vec<Request>, String> {
    let alerts = match s.str("region")? {
        "eu" => "https://api.eu.opsgenie.com/v2/alerts",
        _ => "https://api.opsgenie.com/v2/alerts",
    };
    let alias = alias(m);
    let request = match m.status {
        Status::Up => HttpRequest::post(format!("{alerts}/{}/close", encode(&alias)))
            .query([("identifierType", "alias")])?
            .json(json!({
                "source": "Vesper",
                "note": truncate(&m.body(), 25000),
            })),
        _ => {
            let priority = match m.status {
                Status::Test => "P5",
                _ => s.str("priority")?,
            };
            let mut body = json!({
                "message": truncate(&m.title, 130),
                "alias": alias,
                "description": truncate(&m.body(), 15000),
                "source": "Vesper",
                "priority": priority,
            });
            if let Some(i) = &m.incident {
                body["entity"] = json!(truncate(&i.service_name, 512));
            }
            HttpRequest::post(alerts).json(body)
        }
    };
    let key = s.str("api_key")?;
    Ok(vec![request
        .header("Authorization", format!("GenieKey {key}"))
        .into()])
}
