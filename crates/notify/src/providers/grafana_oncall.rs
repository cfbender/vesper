use super::prelude::*;

pub const PROVIDER: Provider = Provider::new("grafana_oncall", "Grafana OnCall", FIELDS, build)
    .docs("https://grafana.com/docs/oncall/latest/configure/integrations/references/formatted-webhook/")
    .identity(&["webhook_url"])
    .per_incident();

const FIELDS: &[Field] = &[Field::secret_url(
    "webhook_url",
    "URL of a Formatted Webhook integration.",
)
.example("https://oncall-prod-us-central-0.grafana.net/oncall/integrations/v1/formatted_webhook/AbCdEf123/")];

fn build(s: &Settings, m: &Message) -> Result<Vec<Request>, String> {
    let state = match m.status {
        Status::Up => "ok",
        _ => "alerting",
    };
    let body = json!({
        "alert_uid": m.dedup_key(),
        "title": m.title,
        "message": m.summary(),
        "state": state,
        "link_to_upstream_details": m.link,
    });
    Ok(vec![HttpRequest::post(s.str("webhook_url")?)
        .json(body)
        .into()])
}
