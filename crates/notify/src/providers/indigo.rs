use super::prelude::*;

pub const PROVIDER: Provider = Provider::new("indigo", "Indigo", FIELDS, build)
    .docs("https://wiki.indigodomo.com/2025.1/api")
    .identity(&["server_url", "variable_id", "action_group_id"]);

const FIELDS: &[Field] = &[
    Field::url(
        "server_url",
        "Indigo Reflector (`https://NAME.indigodomo.net`) or local Indigo Web Server address.",
    )
    .example("https://myhome.indigodomo.net"),
    Field::secret(
        "api_key",
        "API key from the Authorizations page of your Indigo account.",
    ),
    Field::int(
        "variable_id",
        "ID of the variable that receives the message text.",
    )
    .example("123456789"),
    Field::int(
        "action_group_id",
        "ID of an action group to run after the variable is updated.",
    )
    .optional()
    .example("987654321"),
];

fn build(s: &Settings, m: &Message) -> Result<Vec<Request>, String> {
    let url = join(&s.base_url("server_url")?, "v2/api/command");
    let key = s.str("api_key")?;
    let variable = s
        .int("variable_id")?
        .ok_or("setting variable_id is required")?;
    // The variable is written first so the action group can read it.
    let mut commands = vec![json!({
        "id": "vesper",
        "message": "indigo.variable.updateValue",
        "objectId": variable,
        "parameters": { "value": m.text() },
    })];
    if let Some(group) = s.int("action_group_id")? {
        commands.push(json!({
            "id": "vesper",
            "message": "indigo.actionGroup.execute",
            "objectId": group,
        }));
    }
    Ok(commands
        .into_iter()
        .map(|command| {
            HttpRequest::post(url.clone())
                .bearer(key)
                .json(command)
                .accept(classify)
                .into()
        })
        .collect())
}

/// Indigo can answer 200 with `error` or `validationErrors` in the body.
fn classify(r: &Response) -> Verdict {
    match by_status(r) {
        Verdict::Accepted => {
            let json = r.json();
            match (json.get("error"), json.get("validationErrors")) {
                (None | Some(Value::Null), None | Some(Value::Null)) => Verdict::Accepted,
                (Some(Value::String(e)), _) => Verdict::Rejected(e.clone()),
                (_, Some(v)) => Verdict::Rejected(v.to_string()),
                (Some(e), None) => Verdict::Rejected(e.to_string()),
            }
        }
        other => other,
    }
}
