use super::prelude::*;

pub const PROVIDER: Provider = Provider::new("goalert", "GoAlert", FIELDS, build)
    .docs("https://goalert.me/docs/alerting/alerting-from-generic-api/")
    .identity(&["base_url"])
    .per_incident();

const FIELDS: &[Field] = &[
    Field::url("base_url", "GoAlert server.").example("https://goalert.example.com"),
    Field::secret("token", "Generic API integration key of the service."),
];

fn build(s: &Settings, m: &Message) -> Result<Vec<Request>, String> {
    let mut form = vec![
        ("summary", m.title.clone()),
        ("details", m.body()),
        ("dedup", m.dedup_key()),
    ];
    if m.status == Status::Up {
        form.push(("action", "close".into()));
    }
    let url = join(&s.base_url("base_url")?, "api/v2/generic/incoming");
    Ok(vec![HttpRequest::post(url)
        .bearer(s.str("token")?)
        .form(form)
        .into()])
}
