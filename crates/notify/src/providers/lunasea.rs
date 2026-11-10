use super::prelude::*;

pub const PROVIDER: Provider = Provider::new("lunasea", "LunaSea", FIELDS, build)
    .docs("https://docs.lunasea.app/lunasea/notifications/custom-notifications")
    .identity(&["target", "target_id"]);

const FIELDS: &[Field] = &[
    Field::choice(
        "target",
        &["device", "user"],
        "`device` notifies one device; `user` notifies every device signed in to the LunaSea account.",
    )
    .default("device"),
    Field::secret(
        "target_id",
        "Device or user identifier: the last part of any module's webhook URL in LunaSea; anyone who has it can notify you.",
    )
    .example("dK1aBcDeFgHiJkLmNoPqRs"),
];

fn build(s: &Settings, m: &Message) -> Result<Vec<Request>, String> {
    let target = match s.str("target")? {
        "user" => "user",
        _ => "device",
    };
    let url = format!(
        "https://notify.lunasea.app/v1/custom/{target}/{}",
        encode(s.str("target_id")?)
    );
    let body = json!({
        "title": m.title,
        "body": m.body(),
    });
    Ok(vec![HttpRequest::post(url).json(body).into()])
}
