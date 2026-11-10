use super::prelude::*;

pub const PROVIDER: Provider = Provider::new(
    "jira_service_management",
    "Jira Service Management",
    FIELDS,
    build,
)
.docs("https://developer.atlassian.com/cloud/jira/service-desk-ops/rest/v1/api-group-integration-events/")
.identity(&["api_key"])
.per_incident();

const FIELDS: &[Field] = &[
    Field::secret(
        "api_key",
        "API key of a Jira Service Management Operations API integration.",
    ),
    Field::choice(
        "priority",
        &["P1", "P2", "P3", "P4", "P5"],
        "Priority of problem alerts; tests use P5.",
    )
    .default("P3"),
];

const ALERTS: &str = "https://api.atlassian.com/jsm/ops/integration/v2/alerts";

/// Alias of the alert; slashes are replaced so it can be used in a URL path.
fn alias(m: &Message) -> String {
    m.dedup_key().replace('/', "-")
}

fn build(s: &Settings, m: &Message) -> Result<Vec<Request>, String> {
    let alias = alias(m);
    let request = match m.status {
        Status::Up => HttpRequest::post(format!("{ALERTS}/{}/close", encode(&alias)))
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
                "tags": ["Vesper"],
            });
            if let Some(i) = &m.incident {
                body["entity"] = json!(truncate(&i.service_name, 512));
            }
            HttpRequest::post(ALERTS).json(body)
        }
    };
    let key = s.str("api_key")?;
    Ok(vec![request
        .header("Authorization", format!("GenieKey {key}"))
        .header("Accept", "application/json")
        .into()])
}
