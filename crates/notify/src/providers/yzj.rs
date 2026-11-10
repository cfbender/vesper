use super::prelude::*;

pub const PROVIDER: Provider = Provider::new("yzj", "YZJ (Yunzhijia)", FIELDS, build)
    .docs("https://www.yunzhijia.com/opendocs/docs.html#/tutorial/index/robot")
    .identity(&["webhook_url", "token"]);

const FIELDS: &[Field] = &[
    Field::url(
        "webhook_url",
        "Robot webhook URL, without its query string.",
    )
    .example("https://www.yunzhijia.com/gateway/robot/webhook/send"),
    Field::secret(
        "token",
        "Robot token, the `yzjtoken` parameter of the webhook URL.",
    )
    .example("abc123"),
];

fn build(s: &Settings, m: &Message) -> Result<Vec<Request>, String> {
    // yzjtype=0 selects the general group robot.
    let request = HttpRequest::post(s.str("webhook_url")?)
        .query([("yzjtype", "0"), ("yzjtoken", s.str("token")?)])?
        .json(json!({ "content": m.text() }))
        .accept(classify);
    Ok(vec![request.into()])
}

/// The robot API answers 200 with `success` and, on failure, `errmsg`.
fn classify(r: &Response) -> Verdict {
    match by_status(r) {
        Verdict::Accepted if r.json().get("success").and_then(Value::as_bool) == Some(true) => {
            Verdict::Accepted
        }
        Verdict::Accepted => Verdict::Rejected(r.summary()),
        other => other,
    }
}
