use super::prelude::*;
use crate::FieldKind;

pub const PROVIDER: Provider = Provider::new("wxpusher", "WxPusher", FIELDS, build)
    .docs("https://wxpusher.zjiecode.com/docs/#/?id=spt")
    .identity(&["spt"]);

/// The simple-push endpoint takes at most this many SPTs per request.
const SPT_PER_REQUEST: usize = 10;

const FIELDS: &[Field] = &[Field {
    kind: FieldKind::List,
    ..Field::secret(
        "spt",
        "Simple push tokens (SPT) of the recipients, from scanning the WxPusher QR code.",
    )
    .example("SPT_abcdefghijklmnop")
}];

fn build(s: &Settings, m: &Message) -> Result<Vec<Request>, String> {
    let spts = s.required_list("spt")?;
    let summary = truncate(&m.title, 100);
    let requests = spts
        .chunks(SPT_PER_REQUEST)
        .map(|batch| {
            let body = json!({
                "content": m.text(),
                "summary": summary,
                "contentType": 1,
                "sptList": batch,
                "url": m.link,
            });
            HttpRequest::post("https://wxpusher.zjiecode.com/api/send/message/simple-push")
                .json(body)
                .accept(|r| body_code(r, "/code", "1000"))
                .into()
        })
        .collect();
    Ok(requests)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Setting;

    #[test]
    fn batches_ten_tokens_per_request() {
        let spts: Vec<String> = (1..=11).map(|i| format!("SPT_{i}")).collect();
        let settings = Settings::new(&PROVIDER, [("spt".to_string(), Setting::List(spts))].into());
        let message = super::super::tests::message(Status::Down, false);
        let requests = build(&settings, &message).unwrap();
        let lists: Vec<usize> = requests
            .iter()
            .map(|r| match r {
                Request::Http(h) => match &h.body {
                    crate::RequestBody::Json(v) => v["sptList"].as_array().unwrap().len(),
                    _ => panic!("expected JSON"),
                },
                Request::Mail(_) => panic!("expected HTTP"),
            })
            .collect();
        assert_eq!(lists, [10, 1]);
    }
}
