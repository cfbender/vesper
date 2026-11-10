use super::prelude::*;

pub const PROVIDER: Provider = Provider::new("call_me_bot", "CallMeBot", FIELDS, build)
    .docs("https://www.callmebot.com/blog/free-api-whatsapp-messages/")
    .identity(&["endpoint"]);

const FIELDS: &[Field] = &[Field::secret_url(
    "endpoint",
    "API URL from CallMeBot's activation message (WhatsApp, Signal, Facebook Messenger or Telegram), with your phone and API key; Vesper sets its `text` parameter.",
)
.example("https://api.callmebot.com/whatsapp.php?phone=34123123123&apikey=123456")];

fn build(s: &Settings, m: &Message) -> Result<Vec<Request>, String> {
    let mut url =
        url::Url::parse(s.str("endpoint")?).map_err(|_| "setting endpoint is not a valid URL")?;
    // Replace any `text` the copied URL already carries.
    let kept: Vec<(String, String)> = url
        .query_pairs()
        .filter(|(k, _)| k != "text")
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    url.query_pairs_mut()
        .clear()
        .extend_pairs(kept)
        .append_pair("text", &m.text());
    Ok(vec![HttpRequest::get(url.to_string()).into()])
}
